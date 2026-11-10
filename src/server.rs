use crate::cli::ServeArgs;
use crate::db::Db;
use crate::state::AppState;
use anyhow::{Context, Result};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tower::ServiceExt;

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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700));
    }

    let db = Db::open(&data.join("dok.db"))?;

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
    crate::ui::SECURE_COOKIES.store(scheme == "https", Ordering::Relaxed);
    // Passkey RP IDs must be domain names, not bare IPs, so a default
    // `dok serve` uses localhost for the dashboard origin. An explicit
    // --dashboard-origin may be given as `host:port` or as a full URL; the
    // port is never appended twice.
    let origin = match normalize_origin(
        args.dashboard_origin.as_deref(),
        &args.public_host,
        args.http_port,
        args.tls_port,
        scheme,
    ) {
        Ok(o) => o,
        Err(e) => {
            anyhow::bail!(
                "{e}\nset a valid --dashboard-origin, e.g. --dashboard-origin http://localhost:{}",
                args.http_port
            );
        }
    };
    let public_url = match &args.dashboard_origin {
        // The origin is the URL clients use for the dashboard and API.
        Some(_) => origin.clone(),
        None => match scheme {
            "https" => format!(
                "https://{}",
                args.base_domain
                    .clone()
                    .or_else(|| args.acme_domain.first().cloned())
                    .unwrap_or_else(|| "localhost".into())
            ),
            _ => {
                let host = if args.public_host == "0.0.0.0" || args.public_host == "::" {
                    "localhost".to_string()
                } else {
                    args.public_host.clone()
                };
                format!("http://{host}")
            }
        },
    };
    // Append the listening port for plain HTTP output when the host has none.
    let public_url = ensure_port(&public_url, args.http_port, "http");

    // Edge mode terminates TLS itself, so the dashboard origin must be a real
    // hostname that can receive a certificate. Include it in ACME from boot;
    // otherwise passkeys cannot work on a bare fresh install.
    let mut edge_dashboard_domains = Vec::new();
    if args.edge {
        for raw in [&public_url, &origin] {
            if let Ok(u) = url::Url::parse(raw) {
                if let Some(h) = u.host_str() {
                    let h = h.trim_end_matches('.').to_ascii_lowercase();
                    if !edge_dashboard_domains.contains(&h) {
                        edge_dashboard_domains.push(h);
                    }
                }
            }
        }
        for host in &edge_dashboard_domains {
            if host.parse::<std::net::IpAddr>().is_ok()
                || host == "localhost"
                || host.ends_with(".localhost")
            {
                anyhow::bail!(
                    "--edge needs a public dashboard hostname (set --public-host and --dashboard-origin, e.g. dok.example.com); use plain `dok serve` behind an external TLS proxy instead"
                );
            }
        }
    }

    init_token(&db, &data, args.token.as_deref(), &public_url)?;

    let auth = match url::Url::parse(&origin) {
        Ok(u) => match crate::auth::AuthState::new(u) {
            Ok(a) => Some(std::sync::Arc::new(a)),
            Err(e) => {
                tracing::error!(
                    "dashboard disabled: {e}; pass --dashboard-origin with a real hostname (e.g. https://dok.example.com)"
                );
                None
            }
        },
        Err(e) => {
            tracing::error!("dashboard disabled: bad origin {origin}: {e}");
            None
        }
    };

    let mut trusted = Vec::new();
    for raw in &args.trusted_proxy {
        match raw.parse::<ipnet::IpNet>() {
            Ok(net) => trusted.push(net),
            Err(e) => tracing::warn!("ignoring --trusted-proxy {raw}: {e}"),
        }
    }
    let st = AppState::new(
        db,
        data.clone(),
        args.base_domain.clone(),
        public_url.clone(),
    )
    .with_auth(auth)
    .with_dashboard_origin(Some(origin.clone()))
    .with_traefik_dir(args.traefik_dir.clone())
    .with_trusted_proxies(trusted);

    st.sync_traefik();
    // The post-receive hook runs in a separate process without the CLI flags;
    // persist what it needs to build preview URLs.
    let _ = st
        .db
        .set_setting("base_domain", args.base_domain.as_deref().unwrap_or(""));
    let _ = st.db.set_setting("public_url", &public_url);
    sweep_tmp(&data);

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
    tracing::info!(
        "listening on {addr}, public url {public_url}, dashboard {origin}{}",
        if origin != public_url {
            " (open the dashboard on this URL for passkeys)"
        } else {
            ""
        }
    );

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

    // In edge mode :80 serves the dashboard until the first certificate is
    // ready, then switches to ACME + HTTPS redirect. Without this, a fresh
    // install with no domains yet would need a restart before TLS could start.
    let tls_active = Arc::new(AtomicBool::new(false));

    let shared_resolver: std::sync::Arc<
        std::sync::Mutex<Option<std::sync::Arc<rustls_acme::ResolvesServerCertAcme>>>,
    > = std::sync::Arc::new(std::sync::Mutex::new(None));

    // Plain HTTP: in edge mode this is ACME HTTP-01 + HTTPS redirect; otherwise
    // it keeps serving the full API/proxy for an external TLS terminator.
    let http_listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot bind HTTP port {addr}"))?;
    let http_st = st.clone();
    let edge = args.edge;
    let http_resolver = shared_resolver.clone();
    let http_tls_active = tls_active.clone();
    let http = tokio::spawn(async move {
        if edge {
            let resolver = http_resolver;
            let active = http_tls_active;
            let fallback_st = http_st;
            let app =
                axum::Router::new().fallback(move |req: axum::http::Request<axum::body::Body>| {
                    let resolver = resolver.clone();
                    let active = active.clone();
                    let st = fallback_st.clone();
                    async move {
                        let path = req.uri().path().to_string();
                        if let Some(token) = path.strip_prefix("/.well-known/acme-challenge/") {
                            let current =
                                resolver.lock().unwrap_or_else(|e| e.into_inner()).clone();
                            if let Some(r) = current {
                                if let Some(body) = r.get_http_01_key_auth(token) {
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
                        }
                        if active.load(Ordering::Relaxed) {
                            let host = crate::proxy::host_of(&req);
                            let pq = req
                                .uri()
                                .path_and_query()
                                .map(|v| v.as_str())
                                .unwrap_or("/");
                            let location = format!("https://{host}{pq}");
                            return axum::response::Response::builder()
                                .status(axum::http::StatusCode::MOVED_PERMANENTLY)
                                .header(axum::http::header::LOCATION, location)
                                .body(axum::body::Body::empty())
                                .unwrap();
                        }
                        // No certificate yet: keep the dashboard reachable over
                        // HTTP so the first domain/project can be added.
                        match crate::api::router(st).oneshot(req).await {
                            Ok(r) => r,
                            Err(_) => axum::response::Response::builder()
                                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                                .body(axum::body::Body::from("internal\n"))
                                .unwrap(),
                        }
                    }
                });
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

    let extra_domains = {
        let mut domains = args.acme_domain.clone();
        domains.extend(edge_dashboard_domains.clone());
        domains
    };
    let base_domain = args.base_domain.clone();
    let acme_email = args.acme_email.clone();
    let acme_prod = args.acme_prod;
    let acme_dir = data.join("acme");
    let tls_port = args.tls_port;
    let supervisor_st = st.clone();
    let supervisor_tls_active = tls_active.clone();
    let supervisor = tokio::spawn(async move {
        let mut current_hosts: Vec<String> = Vec::new();
        let mut current_tls: Option<tokio::task::JoinHandle<()>> = None;
        let mut current_acme: Option<tokio::task::JoinHandle<()>> = None;
        let mut last_renew = supervisor_st
            .db
            .get_setting("cert_renew")
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        loop {
            let hosts = desired_hosts(&supervisor_st, &extra_domains, &base_domain);
            let renew_at = supervisor_st
                .db
                .get_setting("cert_renew")
                .ok()
                .flatten()
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0);
            let renew = renew_at > last_renew;
            let renew_host = supervisor_st
                .db
                .get_setting("cert_renew_host")
                .ok()
                .flatten()
                .unwrap_or_default();
            if hosts.is_empty() {
                // Keep the HTTP dashboard alive; TLS starts as soon as a
                // project or custom domain is registered.
                if !current_hosts.is_empty() {
                    supervisor_tls_active.store(false, Ordering::Relaxed);
                    if let Some(h) = current_tls.take() {
                        h.abort();
                    }
                    if let Some(h) = current_acme.take() {
                        h.abort();
                    }
                    current_hosts.clear();
                }
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                continue;
            }
            if hosts != current_hosts || renew {
                if renew {
                    last_renew = renew_at;
                    clear_cert_cache_for(&acme_dir, &renew_host);
                }
                supervisor_tls_active.store(false, Ordering::Relaxed);
                if let Some(h) = current_tls.take() {
                    h.abort();
                    let _ = h.await;
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                if let Some(h) = current_acme.take() {
                    h.abort();
                }
                current_hosts = hosts.clone();
                tracing::info!(
                    "ACME: {} ({} domain(s)){}",
                    if acme_prod { "production" } else { "staging" },
                    hosts.len(),
                    if edge { ", edge mode" } else { "" }
                );
                let mut acme = rustls_acme::AcmeConfig::new(hosts.clone())
                    .contact(acme_email.iter().map(|e| format!("mailto:{e}")))
                    .cache(rustls_acme::caches::DirCache::new(acme_dir.clone()))
                    .directory_lets_encrypt(acme_prod)
                    .state();
                let acceptor = acme.axum_acceptor(acme.default_rustls_config());
                *shared_resolver.lock().unwrap_or_else(|e| e.into_inner()) = Some(acme.resolver());
                let cert_st = supervisor_st.clone();
                let cert_hosts = hosts.clone();
                let cert_dir = acme_dir.clone();
                current_acme = Some(tokio::spawn(async move {
                    use futures_util::StreamExt;
                    while let Some(event) = acme.next().await {
                        match event {
                            Ok(ok) => {
                                tracing::info!("acme: {ok:?}");
                                if matches!(
                                    ok,
                                    rustls_acme::EventOk::DeployedNewCert
                                        | rustls_acme::EventOk::DeployedCachedCert
                                ) {
                                    record_cert_status(&cert_st, &cert_hosts, &cert_dir, None);
                                }
                            }
                            Err(err) => {
                                tracing::error!("acme: {err:?}");
                                record_cert_status(
                                    &cert_st,
                                    &cert_hosts,
                                    &cert_dir,
                                    Some(&err.to_string()),
                                );
                            }
                        }
                    }
                }));

                let mut listener = None;
                for _ in 0..50 {
                    match std::net::TcpListener::bind(("0.0.0.0", tls_port)) {
                        Ok(l) => {
                            let _ = l.set_nonblocking(true);
                            listener = Some(l);
                            break;
                        }
                        Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
                    }
                }
                let Some(listener) = listener else {
                    tracing::error!("cannot bind TLS port {tls_port}");
                    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                    continue;
                };
                let https_st = supervisor_st.clone();
                let tls_flag = supervisor_tls_active.clone();
                current_tls = Some(tokio::spawn(async move {
                    if let Err(e) = axum_server::from_tcp(listener)
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
                    tls_flag.store(false, Ordering::Relaxed);
                }));
                supervisor_tls_active.store(true, Ordering::Relaxed);
            }
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        }
    });

    let _ = tokio::join!(http, supervisor);
    Ok(())
}

fn desired_hosts(st: &AppState, extra: &[String], base: &Option<String>) -> Vec<String> {
    let mut hosts: Vec<String> = extra.to_vec();
    for a in st.db.list_apps().unwrap_or_default() {
        if let Some(d) = a.domain {
            if !hosts.contains(&d) {
                hosts.push(d);
            }
        }
        if let Some(b) = base {
            let h = format!("{}.{}", a.name, b);
            if !hosts.contains(&h) {
                hosts.push(h);
            }
        }
        for h in st.db.domains_for(&a.name).unwrap_or_default() {
            if !hosts.contains(&h) {
                hosts.push(h);
            }
        }
    }
    for (h, _) in st.db.domain_hosts().unwrap_or_default() {
        if !hosts.contains(&h) {
            hosts.push(h);
        }
    }
    hosts.sort();
    hosts.dedup();
    hosts
}

fn clear_cert_cache_for(dir: &std::path::Path, host: &str) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("cached_cert_") {
            continue;
        }
        if host.is_empty() {
            let _ = std::fs::remove_file(entry.path());
            continue;
        }
        let matches = name.to_string_lossy().contains(host)
            || std::fs::read(entry.path())
                .map(|b| String::from_utf8_lossy(&b).contains(host))
                .unwrap_or(false);
        if matches {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn parse_cert_expiry(bytes: &[u8]) -> Option<i64> {
    let text = String::from_utf8_lossy(bytes).replace("\\n", "\n");
    let mut reader = std::io::BufReader::new(text.as_bytes());
    let der = rustls_pemfile::certs(&mut reader).next()?.ok()?;
    let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).ok()?;
    Some(cert.validity().not_after.timestamp())
}

/// Expiry of the cached certificate that belongs to this host, so one host's
/// cert is never reported as another host's status.
fn cert_expiry_for(dir: &std::path::Path, host: &str) -> Option<i64> {
    let rd = std::fs::read_dir(dir).ok()?;
    for entry in rd.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("cached_cert_") {
            continue;
        }
        let bytes = std::fs::read(entry.path()).ok()?;
        let text = String::from_utf8_lossy(&bytes).replace("\\n", "\n");
        if name.to_string_lossy().contains(host) || text.contains(host) {
            return parse_cert_expiry(&bytes);
        }
    }
    None
}

fn record_cert_status(
    st: &AppState,
    hosts: &[String],
    acme_dir: &std::path::Path,
    error: Option<&str>,
) {
    let now = crate::db::now();
    for host in hosts {
        let expiry = cert_expiry_for(acme_dir, host);
        let previous = st
            .db
            .get_setting(&format!("cert:{host}"))
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok());
        let prev_status = previous
            .as_ref()
            .and_then(|v| v.get("status"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let prev_expiry = previous
            .as_ref()
            .and_then(|v| v.get("expires"))
            .and_then(|v| v.as_i64());
        let healthy =
            prev_status == "issued" && prev_expiry.map(|e| e > now + 7 * 86400).unwrap_or(false);
        let (status, expires) = match (expiry, error) {
            (Some(exp), _) => {
                let status = if exp - now < 14 * 86400 {
                    "expiring"
                } else {
                    "issued"
                };
                (status, Some(exp))
            }
            (None, Some(_)) if healthy => ("issued", prev_expiry),
            (None, Some(_)) => ("error", prev_expiry),
            (None, None) => ("issued", prev_expiry),
        };
        let value = serde_json::json!({
            "status": status,
            "expires": expires,
            "checked_at": now,
            "error": error,
        });
        let _ = st
            .db
            .set_setting(&format!("cert:{host}"), &value.to_string());
    }
}

/// Remove partial uploads and build leftovers older than a day.
fn sweep_tmp(data: &std::path::Path) {
    let dir = data.join("tmp");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in rd.flatten() {
        let path = entry.path();
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .map(|d| d.as_secs() > 24 * 3600)
            .unwrap_or(false);
        if old {
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

fn backup_db(st: &AppState, keep: usize) {
    let dir = st.data.join("backups");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
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

fn init_token(
    db: &Db,
    data: &std::path::Path,
    given: Option<&str>,
    public_url: &str,
) -> Result<()> {
    if let Some(t) = given {
        match db.single_user()? {
            Some(u) => {
                db.add_token(&crate::api::hash(t), "cli", Some(&u.id))?;
                tracing::info!("token registered for {}", u.email);
            }
            None => {
                db.add_token(&crate::api::hash(t), "cli", None)?;
                tracing::warn!(
                    "token stored without a user (inactive); it becomes usable when the first dashboard user is created"
                );
            }
        }
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
        "\n  dok initial token: {token}\n  saved to {}\n  use: dok login {public_url} {token}\n  dashboard: {public_url}\n",
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

fn default_port(scheme: &str) -> u16 {
    if scheme == "https" {
        443
    } else {
        80
    }
}

fn split_authority(a: &str) -> (String, Option<u16>) {
    let a = a.trim();
    if let Some(rest) = a.strip_prefix('[') {
        if let Some((host, tail)) = rest.split_once(']') {
            let port = tail.strip_prefix(':').and_then(|p| p.parse().ok());
            return (format!("[{host}]"), port);
        }
    }
    if a.matches(':').count() > 1 {
        // Unbracketed IPv6: no port.
        return (a.to_string(), None);
    }
    match a.rsplit_once(':') {
        Some((host, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
            (host.to_string(), p.parse().ok())
        }
        _ => (a.to_string(), None),
    }
}

fn build_origin(scheme: &str, host: &str, port: u16) -> String {
    if port == default_port(scheme) {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{port}")
    }
}

/// Normalise `--dashboard-origin`: accept `https://host`, `host`, or
/// `host:port`. Bare IPs become `localhost` because WebAuthn RP IDs must be
/// domain names. The listening port is added once, never twice.
fn normalize_origin(
    raw: Option<&str>,
    public_host: &str,
    http_port: u16,
    tls_port: u16,
    scheme: &str,
) -> std::result::Result<String, String> {
    let is_ip = |h: &str| {
        h.trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok()
    };
    let (scheme, authority, explicit_port) = match raw {
        Some(r) => {
            let r = r.trim().trim_end_matches('/');
            if r.contains("://") {
                let u = url::Url::parse(r)
                    .map_err(|e| format!("invalid --dashboard-origin {r}: {e}"))?;
                let host = u
                    .host_str()
                    .ok_or_else(|| format!("invalid --dashboard-origin {r}: missing host"))?;
                let host = if is_ip(host) {
                    "localhost".to_string()
                } else {
                    host.to_string()
                };
                let port = u.port().or_else(|| {
                    // Local origins without a port use the port dok listens on.
                    if (host == "localhost" || host == "[::1]") && u.scheme() == scheme {
                        Some(if scheme == "https" {
                            tls_port
                        } else {
                            http_port
                        })
                    } else {
                        None
                    }
                });
                (u.scheme().to_string(), host, port)
            } else {
                let (host, port) = split_authority(r);
                let host = if is_ip(&host) {
                    "localhost".to_string()
                } else {
                    host
                };
                (scheme.to_string(), host, port)
            }
        }
        None => {
            let mut host = public_host.trim().to_string();
            if host.is_empty() || host == "0.0.0.0" || host == "::" {
                host = "localhost".into();
            }
            let (host, port) = split_authority(&host);
            let host = if is_ip(&host) {
                "localhost".to_string()
            } else {
                host
            };
            (scheme.to_string(), host, port)
        }
    };
    if authority.is_empty() {
        return Err("invalid --dashboard-origin: missing host".into());
    }
    let port = explicit_port.unwrap_or(if scheme == "https" {
        tls_port
    } else {
        http_port
    });
    Ok(build_origin(&scheme, &authority, port))
}

/// Add the listening port to a plain-HTTP public URL when it has none.
fn ensure_port(url: &str, port: u16, scheme: &str) -> String {
    let Ok(mut u) = url::Url::parse(url) else {
        return url.to_string();
    };
    if u.port().is_none() && u.scheme() == scheme && port != default_port(scheme) {
        let _ = u.set_port(Some(port));
    }
    u.to_string().trim_end_matches('/').to_string()
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::{ensure_port, normalize_origin};

    fn origin(raw: Option<&str>, host: &str) -> String {
        normalize_origin(raw, host, 8080, 443, "http").unwrap()
    }

    #[test]
    fn dashboard_origin_accepts_hosts_and_ports() {
        // A bare dok serve must work: IPs become a valid WebAuthn RP ID.
        assert_eq!(origin(None, "127.0.0.1"), "http://localhost:8080");
        assert_eq!(origin(None, "0.0.0.0"), "http://localhost:8080");
        assert_eq!(origin(None, "localhost"), "http://localhost:8080");
        assert_eq!(origin(None, "[::1]"), "http://localhost:8080");
        // Ports are never appended twice.
        assert_eq!(origin(None, "localhost:9000"), "http://localhost:9000");
        assert_eq!(
            origin(Some("localhost:9000"), "127.0.0.1"),
            "http://localhost:9000"
        );
        assert_eq!(
            origin(Some("http://127.0.0.1:7000"), "127.0.0.1"),
            "http://localhost:7000"
        );
        assert_eq!(
            origin(Some("http://localhost"), "127.0.0.1"),
            "http://localhost:8080"
        );
        assert_eq!(
            origin(Some("https://dok.example.com"), "127.0.0.1"),
            "https://dok.example.com"
        );
        assert_eq!(
            origin(Some("dok.example.com:8443"), "127.0.0.1"),
            "http://dok.example.com:8443"
        );
    }

    #[test]
    fn public_url_gets_one_port() {
        assert_eq!(
            ensure_port("http://127.0.0.1", 8080, "http"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            ensure_port("http://127.0.0.1:9000", 8080, "http"),
            "http://127.0.0.1:9000"
        );
        assert_eq!(
            ensure_port("https://dok.example.com", 8080, "http"),
            "https://dok.example.com"
        );
    }
}
