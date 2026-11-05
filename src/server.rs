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
    let scheme = if args.edge || !args.acme_domain.is_empty() || tls_terminated {
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

    if args.backup_keep > 0 {
        let st_backup = st.clone();
        let keep = args.backup_keep;
        tokio::spawn(async move {
            loop {
                backup_db(&st_backup, keep);
                tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
            }
        });
    }

    let http_port = if args.edge && args.http_port == 8080 {
        80
    } else {
        args.http_port
    };
    let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, http_port));
    tracing::info!("listening on {addr}, public url {public_url}");

    let mut domains = args.acme_domain.clone();
    for (_, d) in st.db.domains().unwrap_or_default() {
        if let Some(d) = d {
            if !domains.contains(&d) {
                domains.push(d);
            }
        }
    }
    for (host, _) in st.db.domain_hosts().unwrap_or_default() {
        if !domains.contains(&host) {
            domains.push(host);
        }
    }
    if let Some(base) = &args.base_domain {
        for a in st.db.list_apps().unwrap_or_default() {
            let host = format!("{}.{}", a.name, base);
            if !domains.contains(&host) {
                domains.push(host);
            }
        }
    }
    let tls_mode = args.edge || !args.acme_domain.is_empty();

    if !tls_mode {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(
            listener,
            crate::api::router(st).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
        return Ok(());
    }
    if domains.is_empty() {
        tracing::warn!("edge/ACME enabled but no domains found; serving plain HTTP on {addr}");
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(
            listener,
            crate::api::router(st).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
        return Ok(());
    }

    let domain_count = domains.len();
    let cert_hosts = domains.clone();
    let mut acme = rustls_acme::AcmeConfig::new(domains)
        .contact(args.acme_email.iter().map(|e| format!("mailto:{e}")))
        .cache(rustls_acme::caches::DirCache::new(data.join("acme")))
        .directory_lets_encrypt(args.acme_prod)
        .state();
    let acceptor = acme.axum_acceptor(acme.default_rustls_config());
    let resolver = acme.resolver();

    // Plain HTTP: in edge mode this is ACME HTTP-01 + HTTPS redirect; otherwise
    // it keeps serving the full API/proxy for an external TLS terminator.
    let http_listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot bind HTTP port {addr}"))?;
    let http_st = st.clone();
    let edge = args.edge;
    let http = tokio::spawn(async move {
        if edge {
            let challenge = resolver.clone();
            let app = axum::Router::new().fallback(
                move |req: axum::http::Request<axum::body::Body>| {
                    let resolver = challenge.clone();
                    async move {
                        let path = req.uri().path().to_string();
                        if let Some(token) =
                            path.strip_prefix("/.well-known/acme-challenge/")
                        {
                            if let Some(body) = resolver.get_http_01_key_auth(token) {
                                return axum::response::Response::builder()
                                    .status(axum::http::StatusCode::OK)
                                    .header(
                                        axum::http::header::CONTENT_TYPE,
                                        "application/octet-stream",
                                    )
                                    .body(axum::body::Body::from(body))
                                    .unwrap();
                            }
                        }
                        let host = crate::proxy::host_of(&req);
                        let pq = req
                            .uri()
                            .path_and_query()
                            .map(|v| v.as_str())
                            .unwrap_or("/");
                        let location = format!("https://{host}{pq}");
                        axum::response::Response::builder()
                            .status(axum::http::StatusCode::MOVED_PERMANENTLY)
                            .header(axum::http::header::LOCATION, location)
                            .body(axum::body::Body::empty())
                            .unwrap()
                    }
                },
            );
            if let Err(e) = axum::serve(http_listener, app).await {
                tracing::error!("edge http server stopped: {e}");
            }
            return;
        }
        if let Err(e) = axum::serve(
            http_listener,
            crate::api::router(http_st).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        {
            tracing::error!("http server stopped: {e}");
        }
    });

    let cert_st = st.clone();
    tokio::spawn(async move {
        use futures_util::StreamExt;
        while let Some(event) = acme.next().await {
            match event {
                Ok(ok) => {
                    tracing::info!("acme: {ok:?}");
                    if matches!(ok, rustls_acme::EventOk::DeployedNewCert) {
                        for host in &cert_hosts {
                            let value = serde_json::json!({
                                "status": "issued",
                                "issued_at": crate::db::now(),
                                "expires": null
                            });
                            let _ = cert_st
                                .db
                                .set_setting(&format!("cert:{host}"), &value.to_string());
                        }
                    }
                }
                Err(err) => tracing::error!("acme: {err:?}"),
            }
        }
    });

    let tls_std = std::net::TcpListener::bind(("0.0.0.0", args.tls_port))
        .with_context(|| format!("cannot bind TLS port {}", args.tls_port))?;
    tls_std.set_nonblocking(true)?;
    tracing::info!(
        "ACME: {} ({} domain(s)){}",
        if args.acme_prod { "production" } else { "staging" },
        domain_count,
        if args.edge { ", edge mode" } else { "" }
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

fn backup_db(st: &AppState, keep: usize) {
    let dir = st.data.join("backups");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let stamp = crate::db::now();
    let mut path = dir.join(format!("dok-{stamp}.db"));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("dok-{stamp}-{n}.db"));
        n += 1;
    }
    if let Err(e) = st.db.backup_to(&path) {
        tracing::warn!("backup failed: {e}");
        return;
    }
    tracing::info!("backup written to {}", path.display());
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|x| x == "db").unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    while files.len() > keep {
        let old = files.remove(0);
        let _ = std::fs::remove_file(old);
    }
}

fn init_token(db: &Db, data: &std::path::Path, given: Option<&str>) -> Result<()> {
    if let Some(t) = given {
        db.add_token(&crate::api::hash(t), "cli", None)?;
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
    db.add_token(&crate::api::hash(&token), "initial", None)?;
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
