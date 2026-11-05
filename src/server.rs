use crate::cli::ServeArgs;
use crate::db::Db;
use crate::state::AppState;
use anyhow::{Context, Result};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

pub async fn run(args: ServeArgs) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dok=info".into()),
        )
        .init();

    let data = expand(&args.data);
    std::fs::create_dir_all(&data)
        .with_context(|| format!("create data dir {}", data.display()))?;

    let db = Db::open(&data.join("dok.db"))?;
    init_token(&db, &data, args.token.as_deref())?;

    // Behind Traefik/Caddy TLS termination, --dashboard-origin tells us the
    // scheme clients actually use.
    let tls_terminated = args
        .dashboard_origin
        .as_deref()
        .map(|o| o.starts_with("https://"))
        .unwrap_or(false);
    let scheme = if !args.acme_domain.is_empty() || tls_terminated {
        "https"
    } else {
        "http"
    };
    let public_url = match &args.dashboard_origin {
        Some(o) => o.trim_end_matches('/').to_string(),
        None => match scheme {
            "https" => format!(
                "https://{}",
                args.base_domain
                    .clone()
                    .or_else(|| args.acme_domain.first().cloned())
                    .unwrap_or_else(|| "localhost".into())
            ),
            _ => {
                let host = if args.public_host == "0.0.0.0" {
                    "localhost".to_string()
                } else {
                    args.public_host.clone()
                };
                format!("http://{host}:{}", args.http_port)
            }
        },
    };

    let origin = args
        .dashboard_origin
        .clone()
        .unwrap_or_else(|| public_url.clone());
    let auth = match url::Url::parse(&origin) {
        Ok(u) => match crate::auth::AuthState::new(u) {
            Ok(a) => Some(std::sync::Arc::new(a)),
            Err(e) => {
                tracing::warn!("dashboard disabled: {e}");
                None
            }
        },
        Err(e) => {
            tracing::warn!("dashboard disabled: bad origin {origin}: {e}");
            None
        }
    };

    let st = AppState::new(
        db,
        data.clone(),
        args.base_domain.clone(),
        public_url.clone(),
    )
    .with_auth(auth)
    .with_traefik_dir(args.traefik_dir.clone());

    st.sync_traefik();

    let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, args.http_port));
    tracing::info!("listening on {addr}, public url {public_url}");

    if args.acme_domain.is_empty() {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(
            listener,
            crate::api::router(st).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
        return Ok(());
    }

    // Plain HTTP (API + proxy) and HTTPS with automatic Let's Encrypt certificates.
    let http_listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot bind HTTP port {addr}"))?;
    let http_st = st.clone();
    let http = tokio::spawn(async move {
        if let Err(e) = axum::serve(
            http_listener,
            crate::api::router(http_st).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        {
            tracing::error!("http server stopped: {e}");
        }
    });

    let mut domains = args.acme_domain.clone();
    for (_, d) in st.db.domains().unwrap_or_default() {
        if let Some(d) = d {
            if !domains.contains(&d) {
                domains.push(d);
            }
        }
    }
    let domain_count = domains.len();
    let mut acme = rustls_acme::AcmeConfig::new(domains)
        .contact(args.acme_email.iter().map(|e| format!("mailto:{e}")))
        .cache(rustls_acme::caches::DirCache::new(data.join("acme")))
        .directory_lets_encrypt(args.acme_prod)
        .state();
    let acceptor = acme.axum_acceptor(acme.default_rustls_config());

    tokio::spawn(async move {
        use futures_util::StreamExt;
        while let Some(event) = acme.next().await {
            match event {
                Ok(ok) => tracing::info!("acme: {ok:?}"),
                Err(err) => tracing::error!("acme: {err:?}"),
            }
        }
    });

    let tls_std = std::net::TcpListener::bind(("0.0.0.0", args.tls_port))
        .with_context(|| format!("cannot bind TLS port {}", args.tls_port))?;
    tls_std.set_nonblocking(true)?;
    tracing::info!(
        "ACME: {} ({} domain(s))",
        if args.acme_prod { "production" } else { "staging" },
        domain_count
    );
    let https_st = st.clone();
    let https = tokio::spawn(async move {
        if let Err(e) = axum_server::from_tcp(tls_std)
            .expect("tls listener")
            .acceptor(acceptor)
            .serve(
                crate::api::router(https_st)
                    .into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
        {
            tracing::error!("https server stopped: {e}");
        }
    });

    let _ = tokio::join!(http, https);
    Ok(())
}

fn init_token(db: &Db, data: &std::path::Path, given: Option<&str>) -> Result<()> {
    if let Some(t) = given {
        db.add_token(&crate::api::hash(t), "cli")?;
        tracing::info!("token registered");
        return Ok(());
    }
    if db.count_tokens()? > 0 {
        return Ok(());
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    db.add_token(&crate::api::hash(&token), "initial")?;
    let path = data.join("initial-token");
    std::fs::write(&path, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    eprintln!(
        "\n  dok initial token: {token}\n  saved to {}\n  use: dok login http://<host>:8080 {token}\n",
        path.display()
    );
    Ok(())
}

fn expand(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs_home() {
            return home.join(rest);
        }
    }
    PathBuf::from(p)
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
