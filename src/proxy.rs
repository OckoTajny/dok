use crate::state::AppState;
use axum::body::Body;
use axum::http::{header, HeaderName, HeaderValue, Method, Request, Response, StatusCode};
use base64::Engine;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioIo;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};
use tokio_util::io::ReaderStream;

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

/// Parse `Host`/authority: IPv6 brackets, optional port, trailing dot, case.
pub fn host_of(req: &Request<Body>) -> String {
    let raw = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    parse_authority(raw)
}

fn parse_authority(s: &str) -> String {
    let s = s.trim();
    let host = if s.starts_with('[') {
        match s.find(']') {
            Some(i) => &s[..=i],
            None => s,
        }
    } else {
        s.split(':').next().unwrap_or(s)
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

#[derive(Clone)]
pub struct Route {
    pub app: crate::db::App,
    pub deploy: Option<crate::db::Deploy>,
}

pub fn app_for(st: &AppState, req: &Request<Body>) -> Option<Route> {
    let host = host_of(req);
    if host.is_empty() {
        return None;
    }
    find_route(st, &host)
}

pub async fn serve_app(
    st: AppState,
    mut req: Request<Body>,
    route: Route,
    request_id: &str,
) -> Response<Body> {
    let started = Instant::now();
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let ip = client_ip(&st, &req);
    let host = host_of(&req);
    let app = route.app.clone();
    let deploy = route.deploy.clone();
    let settings = match st.db.proxy_settings(&app.name) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("proxy settings for {}: {e}", app.name);
            return crate::ui::branded_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Temporarily unavailable",
                Some(&app.name),
                "Project routing settings could not be read. The project is kept safe until they are repaired.",
                last_good_at(&st, &app.name),
                request_id,
            );
        }
    };

    if let Some(r) = redirect_for(&st, &req, &host, &settings, ip.as_deref()) {
        log_request(&st, &app.name, &method, &path, &r, started, ip.as_deref());
        return with_request_id(apply_headers(r, &settings), request_id);
    }
    // Reject before Argon2 verification: an attacker must not be able to spend
    // CPU (or spray usernames for fresh buckets) on an unauthenticated route.
    let basic_ip_key = format!("basic-ip:{}", ip.as_deref().unwrap_or("unknown"));
    let basic_user_key = format!(
        "basic:{}:{}",
        ip.as_deref().unwrap_or("unknown"),
        basic_auth_user(&req).unwrap_or_else(|| "-".to_string())
    );
    if settings.basic_auth_hash.is_some()
        && (st.rate.blocked(&basic_ip_key, 30, 60) || st.rate.blocked(&basic_user_key, 10, 60))
    {
        let limited = Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from("dok: too many authentication attempts\n"))
            .unwrap();
        log_request(
            &st,
            &app.name,
            &method,
            &path,
            &limited,
            started,
            ip.as_deref(),
        );
        return with_request_id(apply_headers(limited, &settings), request_id);
    }
    if let Some(r) = basic_auth_challenge(&req, &settings) {
        // Only failed attempts count towards the limiter.
        st.rate.hit(&basic_ip_key);
        st.rate.hit(&basic_user_key);
        log_request(&st, &app.name, &method, &path, &r, started, ip.as_deref());
        return with_request_id(apply_headers(r, &settings), request_id);
    }
    if let Some(max) = settings.max_body_mb {
        let limit = max.saturating_mul(1024 * 1024);
        let too_big = req
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|len| len > limit)
            .unwrap_or(false);
        let has_body = !matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS");
        if too_big {
            let r = crate::ui::branded_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Payload too large",
                Some(&app.name),
                &format!("This project accepts requests up to {max} MB."),
                last_good_at(&st, &app.name),
                request_id,
            );
            log_request(&st, &app.name, &method, &path, &r, started, ip.as_deref());
            return with_request_id(apply_headers(r, &settings), request_id);
        }
        // Chunked uploads without a length are streamed through a byte-limited
        // body instead of being buffered in RAM; a cap that could be bypassed
        // by omitting Content-Length is still enforced.
        if has_body {
            let (parts, body) = req.into_parts();
            let limited = http_body_util::Limited::new(body, limit as usize);
            req = Request::from_parts(parts, Body::new(limited));
        }
    }

    let kind = deploy
        .as_ref()
        .map(|d| {
            if d.image.is_some() {
                "docker"
            } else {
                "static"
            }
        })
        .unwrap_or(app.kind.as_str());
    let dir_override = deploy.as_ref().and_then(|d| d.dir.clone());
    let port_override = deploy.as_ref().and_then(|d| d.host_port);
    let accept_encoding = req
        .headers()
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let resp = match kind {
        "static" => {
            let dir = dir_override
                .clone()
                .or_else(|| app.dir.clone())
                .unwrap_or_default();
            let method = req.method().clone();
            let path = req.uri().path().to_string();
            let inm = req
                .headers()
                .get(header::IF_NONE_MATCH)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            match serve_static(
                Path::new(&dir),
                &method,
                &path,
                inm.as_deref(),
                settings.compression,
                &accept_encoding,
            )
            .await
            {
                Some(r) => r,
                None => crate::ui::branded_error(
                    StatusCode::NOT_FOUND,
                    "Not found",
                    Some(&app.name),
                    "This file does not exist in the current deploy.",
                    last_good_at(&st, &app.name),
                    request_id,
                ),
            }
        }
        _ => {
            let r = match port_override.or(app.host_port) {
                Some(port) => forward(&st, req, port, &app.name, request_id).await,
                None => crate::ui::branded_error(
                    StatusCode::BAD_GATEWAY,
                    "Bad gateway",
                    Some(&app.name),
                    "The project has no running container. Check the deploy log or redeploy.",
                    last_good_at(&st, &app.name),
                    request_id,
                ),
            };
            maybe_compress(r, settings.compression, &accept_encoding).await
        }
    };
    log_request(
        &st,
        &app.name,
        &method,
        &path,
        &resp,
        started,
        ip.as_deref(),
    );
    with_request_id(apply_headers(resp, &settings), request_id)
}

fn with_request_id(mut resp: Response<Body>, request_id: &str) -> Response<Body> {
    if let Ok(v) = HeaderValue::from_str(request_id) {
        resp.headers_mut().insert("x-request-id", v);
    }
    resp
}

fn last_good_at(st: &AppState, app: &str) -> Option<i64> {
    st.db
        .deploys(app, 20)
        .ok()?
        .into_iter()
        .find(|d| d.status == "ok" && d.env == "production")
        .map(|d| d.created_at)
}

fn log_request(
    st: &AppState,
    app: &str,
    method: &str,
    path: &str,
    resp: &Response<Body>,
    started: Instant,
    ip: Option<&str>,
) {
    st.db.log_access(
        app,
        method,
        path,
        resp.status().as_u16(),
        started.elapsed().as_millis() as i64,
        ip,
    );
}

fn redirect_for(
    st: &AppState,
    req: &Request<Body>,
    host: &str,
    settings: &crate::db::ProxySettings,
    peer: Option<&str>,
) -> Option<Response<Body>> {
    if host.is_empty() {
        return None;
    }
    let https = st.edge
        || (peer.map(|p| st.trusted_peer(p)).unwrap_or(false)
            && req
                .headers()
                .get("x-forwarded-proto")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.eq_ignore_ascii_case("https"))
                .unwrap_or(false));
    let forwarded_https = https;
    let local =
        host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host.starts_with("127.");
    if settings.force_https && !https && !forwarded_https && !local {
        let pq = req
            .uri()
            .path_and_query()
            .map(|v| v.as_str())
            .unwrap_or("/");
        return Some(redirect_301(&format!("https://{host}{pq}")));
    }
    let redirect = settings.redirect.as_deref().unwrap_or("off");
    let apex = host.strip_prefix("www.").unwrap_or(host);
    let target = match redirect {
        "www-to-apex" if host.starts_with("www.") => Some(apex.to_string()),
        "apex-to-www" if !host.starts_with("www.") => Some(format!("www.{host}")),
        _ => None,
    };
    target.map(|t| {
        let scheme = if https || forwarded_https {
            "https"
        } else {
            "http"
        };
        let pq = req
            .uri()
            .path_and_query()
            .map(|v| v.as_str())
            .unwrap_or("/");
        redirect_301(&format!("{scheme}://{t}{pq}"))
    })
}

fn redirect_301(location: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::MOVED_PERMANENTLY)
        .header(header::LOCATION, location)
        .body(Body::empty())
        .unwrap()
}

fn basic_auth_user(req: &Request<Body>) -> Option<String> {
    req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.split_once(':').map(|(u, _)| u.to_string()))
}

fn basic_auth_challenge(
    req: &Request<Body>,
    settings: &crate::db::ProxySettings,
) -> Option<Response<Body>> {
    let (Some(user), Some(hash)) = (
        settings.basic_auth_user.as_deref(),
        settings.basic_auth_hash.as_deref(),
    ) else {
        return None;
    };
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| {
            s.split_once(':')
                .map(|(u, p)| (u.to_string(), p.to_string()))
        })
        .map(|(u, p)| ct_eq(u.as_bytes(), user.as_bytes()) && crate::db::verify_password(hash, &p))
        .unwrap_or(false);
    if ok {
        return None;
    }
    Some(
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(header::WWW_AUTHENTICATE, "Basic realm=\"dok\"")
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from("dok: authentication required\n"))
            .unwrap(),
    )
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn apply_headers(mut resp: Response<Body>, settings: &crate::db::ProxySettings) -> Response<Body> {
    for (name, value) in &settings.headers {
        if !crate::api::safe_header(name) {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            resp.headers_mut().insert(n, v);
        }
    }
    resp
}

#[cfg(test)]
fn find_app(st: &AppState, host: &str) -> Option<crate::db::App> {
    find_route(st, host)
        .filter(|r| r.deploy.is_none())
        .map(|r| r.app)
}

fn find_route(st: &AppState, host: &str) -> Option<Route> {
    // The dashboard host, the base apex and every `<anything>.<base>` name is
    // reserved for the control plane and must never resolve to user code.
    if st.reserved_host(host) || st.is_control_host(host) {
        return None;
    }
    if let Ok(Some(app)) = st.db.app_by_domain(host) {
        return Some(Route { app, deploy: None });
    }
    if let Some(base) = &st.base_domain {
        let base = base.to_ascii_lowercase();
        if let Some(prefix) = host.strip_suffix(&format!(".{base}")) {
            if prefix.is_empty() || prefix.contains('.') {
                return None;
            }
            // `<app>-<deploy8>` points at one preview/earlier artifact.
            if let Some((candidate, short)) = prefix.rsplit_once('-') {
                if short.len() == 8 && short.chars().all(|c| c.is_ascii_hexdigit()) {
                    if let Ok(Some(app)) = st.db.get_app(candidate) {
                        if let Ok(Some(deploy)) = st.db.deploy_by_prefix(&app.name, short) {
                            return Some(Route {
                                app,
                                deploy: Some(deploy),
                            });
                        }
                    }
                }
            }
            if let Ok(Some(app)) = st.db.get_app(prefix) {
                // A project with an explicit custom domain is only served
                // on that domain, never on its default subdomain.
                if app.domain.is_none() {
                    return Some(Route { app, deploy: None });
                }
            }
        }
    }
    None
}

/// Forward the app's cookies, dropping only dok's control-plane cookies.
fn filter_cookie_header(value: &HeaderValue) -> Option<HeaderValue> {
    let raw = value.to_str().ok()?;
    let kept: Vec<&str> = raw
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .filter(|p| {
            let name = p.split('=').next().unwrap_or("").trim();
            !matches!(name, "dok_session" | "dok_flash" | "dok_invite_once")
        })
        .collect();
    if kept.is_empty() {
        None
    } else {
        HeaderValue::from_str(&kept.join("; ")).ok()
    }
}

/// Resolve the client IP, trusting `X-Forwarded-For` only from a configured
/// proxy peer. The access log and auth limiter must not all collapse to the
/// Traefik container address in the documented deployment.
fn client_ip(st: &AppState, req: &Request<Body>) -> Option<String> {
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip().to_string())?;
    if st.trusted_peer(&peer) {
        if let Some(ip) = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(str::trim)
            .filter(|s| s.parse::<std::net::IpAddr>().is_ok())
        {
            return Some(ip.to_string());
        }
    }
    Some(peer)
}

fn is_upgrade(req: &Request<Body>) -> bool {
    req.headers()
        .get(header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_ascii_lowercase().contains("upgrade"))
        .unwrap_or(false)
        && req.headers().contains_key(header::UPGRADE)
}

async fn forward(
    st: &AppState,
    mut req: Request<Body>,
    port: u16,
    app: &str,
    request_id: &str,
) -> Response<Body> {
    let upgrade = is_upgrade(&req);
    let client_upgrade = if upgrade {
        Some(hyper::upgrade::on(&mut req))
    } else {
        None
    };

    let (parts, body) = req.into_parts();
    let pq = parts
        .uri
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or("/");
    let uri: axum::http::Uri = match format!("http://127.0.0.1:{port}{pq}").parse() {
        Ok(u) => u,
        Err(_) => {
            return crate::ui::branded_error(
                StatusCode::BAD_GATEWAY,
                "Bad gateway",
                Some(app),
                "The upstream request could not be built.",
                last_good_at(st, app),
                request_id,
            )
        }
    };

    let mut out = Request::builder().method(parts.method.clone()).uri(uri);
    {
        let h = out.headers_mut().unwrap();
        for (k, v) in parts.headers.iter() {
            let name = k.as_str();
            if !upgrade && HOP_BY_HOP.contains(&name) {
                continue;
            }
            // Never leak dok's own control-plane cookies to user code, but do
            // forward the application's cookies: hosted apps with sessions
            // must be able to log their users in.
            if name == "cookie" {
                if let Some(filtered) = filter_cookie_header(v) {
                    h.append(k.clone(), filtered);
                }
                continue;
            }
            // `Proxy-Authorization` is hop-by-hop proxy auth, never app auth.
            if name == "proxy-authorization" {
                continue;
            }
            h.append(k.clone(), v.clone());
        }
        if let Some(hv) = parts.headers.get(header::HOST) {
            h.entry("x-forwarded-host").or_insert_with(|| hv.clone());
        }
        let peer = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0.ip().to_string());
        match (parts.headers.get("x-forwarded-for"), peer) {
            (Some(prev), Some(ip)) => {
                if let Ok(v) =
                    HeaderValue::from_str(&format!("{}, {ip}", prev.to_str().unwrap_or("")))
                {
                    h.insert("x-forwarded-for", v);
                }
            }
            (None, Some(ip)) => {
                if let Ok(v) = HeaderValue::from_str(&ip) {
                    h.insert("x-forwarded-for", v);
                }
            }
            _ => {}
        }
        h.insert(
            "x-forwarded-proto",
            HeaderValue::from_str(&st.scheme).unwrap(),
        );
    }

    let client: &Client<_, Body> = &st.client;
    let resp = match tokio::time::timeout(
        Duration::from_secs(st.upstream_timeout),
        client.request(out.body(body).unwrap()),
    )
    .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            tracing::warn!("upstream error for {app}: {e}");
            return crate::ui::branded_error(
                StatusCode::BAD_GATEWAY,
                "Bad gateway",
                Some(app),
                "The project did not accept the connection. It may be restarting.",
                last_good_at(st, app),
                request_id,
            );
        }
        Err(_) => {
            return crate::ui::branded_error(
                StatusCode::GATEWAY_TIMEOUT,
                "Gateway timeout",
                Some(app),
                "The project took too long to answer.",
                last_good_at(st, app),
                request_id,
            )
        }
    };

    let mut resp = resp;
    if resp.status() == StatusCode::SWITCHING_PROTOCOLS {
        let upstream = hyper::upgrade::on(&mut resp);
        if let Some(downstream) = client_upgrade {
            tokio::spawn(async move {
                if let (Ok(down), Ok(up)) = (downstream.await, upstream.await) {
                    let mut down = TokioIo::new(down);
                    let mut up = TokioIo::new(up);
                    let _ = tokio::io::copy_bidirectional(&mut down, &mut up).await;
                }
            });
        }
    }

    let (parts, body) = resp.into_parts();
    let mut b = Response::builder().status(parts.status);
    {
        let h = b.headers_mut().unwrap();
        let keep_hop = parts.status == StatusCode::SWITCHING_PROTOCOLS;
        for (k, v) in parts.headers.iter() {
            if !keep_hop && HOP_BY_HOP.contains(&k.as_str()) {
                continue;
            }
            h.append(k.clone(), v.clone());
        }
    }
    b.body(Body::new(body)).unwrap_or_else(|_| {
        crate::ui::branded_error(
            StatusCode::BAD_GATEWAY,
            "Bad gateway",
            Some(app),
            "The upstream response was invalid.",
            last_good_at(st, app),
            request_id,
        )
    })
}

fn hidden_path(url_path: &str) -> bool {
    url_path
        .trim_start_matches('/')
        .split('/')
        .any(|part| part.starts_with('.') && part != ".well-known")
}

fn safe_join(root: &Path, url_path: &str) -> Option<PathBuf> {
    let mut out = root.to_path_buf();
    for comp in Path::new(url_path.trim_start_matches('/')).components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

fn encode_body(bytes: &[u8], accept_encoding: &str) -> Option<(&'static str, Vec<u8>)> {
    use std::io::Write;
    let accepts_br = accept_encoding
        .split(',')
        .any(|e| e.trim().split(';').next().unwrap_or("") == "br");
    let accepts_gzip = accept_encoding.contains("gzip");
    if accepts_br {
        let mut out = Vec::new();
        {
            let mut enc = brotli::CompressorWriter::new(&mut out, 4096, 3, 22);
            enc.write_all(bytes).ok()?;
        }
        return Some(("br", out));
    }
    if accepts_gzip {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(bytes).ok()?;
        return Some(("gzip", enc.finish().ok()?));
    }
    None
}

fn compressible_type(content_type: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    ct.starts_with("text/")
        || ct.contains("json")
        || ct.contains("javascript")
        || ct.contains("xml")
        || ct.contains("svg")
}

/// Compress a buffered upstream response (containers included), respecting the
/// client's Accept-Encoding and the project's compression setting.
async fn maybe_compress(
    resp: Response<Body>,
    enabled: bool,
    accept_encoding: &str,
) -> Response<Body> {
    if !enabled
        || resp.headers().contains_key(header::CONTENT_ENCODING)
        || resp.headers().get(header::CONTENT_LENGTH).is_none()
    {
        return resp;
    }
    let declared = resp
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(u64::MAX);
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if declared > 2 * 1024 * 1024 || !compressible_type(content_type) {
        return resp;
    }
    let (parts, body) = resp.into_parts();
    let bytes = match axum::body::to_bytes(body, 2 * 1024 * 1024 + 1).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return crate::ui::branded_error(
                StatusCode::BAD_GATEWAY,
                "Bad gateway",
                None,
                "The upstream response could not be read.",
                None,
                "",
            )
        }
    };
    let Some((encoding, data)) = encode_body(&bytes, accept_encoding) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    let len = data.len();
    let mut out = Response::from_parts(parts, Body::from(data));
    out.headers_mut()
        .insert(header::CONTENT_ENCODING, HeaderValue::from_static(encoding));
    out.headers_mut()
        .insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    if let Ok(len) = HeaderValue::from_str(&len.to_string()) {
        out.headers_mut().insert(header::CONTENT_LENGTH, len);
    }
    out
}

async fn serve_static(
    root: &Path,
    method: &Method,
    url_path: &str,
    if_none_match: Option<&str>,
    compress: bool,
    accept_encoding: &str,
) -> Option<Response<Body>> {
    if method != Method::GET && method != Method::HEAD {
        return Some(
            Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .header(header::ALLOW, "GET, HEAD")
                .body(Body::empty())
                .unwrap(),
        );
    }
    if hidden_path(url_path) {
        return Some(
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .unwrap(),
        );
    }
    let root = root.canonicalize().ok()?;
    let mut path = match safe_join(&root, url_path) {
        Some(p) => p,
        None => {
            return Some(
                Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::empty())
                    .unwrap(),
            )
        }
    };
    let mut meta = tokio::fs::metadata(&path).await.ok();
    if meta.as_ref().map(|m| m.is_dir()).unwrap_or(false) {
        path.push("index.html");
        meta = tokio::fs::metadata(&path).await.ok();
    }
    if meta.is_none() {
        let index = root.join("index.html");
        if url_path != "/" && !url_path.contains('.') && tokio::fs::metadata(&index).await.is_ok() {
            path = index;
            meta = tokio::fs::metadata(&path).await.ok();
        }
    }
    let meta = meta?;
    let canonical = path.canonicalize().ok()?;
    if !canonical.starts_with(&root) || !meta.is_file() {
        return Some(
            Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::empty())
                .unwrap(),
        );
    }

    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let etag = format!("\"{:x}-{:x}\"", meta.len(), mtime);
    if if_none_match
        .map(|v| v.split(',').any(|t| t.trim() == etag))
        .unwrap_or(false)
    {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::ETAG, etag)
            .body(Body::empty())
            .ok();
    }

    let mime = mime_guess::from_path(&canonical).first_or_octet_stream();
    let is_html =
        mime.type_() == mime_guess::mime::TEXT && mime.subtype() == mime_guess::mime::HTML;
    let cache = if is_html {
        "no-cache"
    } else {
        "public, max-age=300"
    };
    let compressible = mime.type_() == mime_guess::mime::TEXT
        || mime.subtype() == mime_guess::mime::JSON
        || mime.subtype() == mime_guess::mime::JAVASCRIPT;
    let builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CACHE_CONTROL, cache)
        .header(header::ETAG, etag)
        .header(header::CONTENT_LENGTH, meta.len());

    if method == Method::HEAD {
        return builder.body(Body::empty()).ok();
    }
    if compress && compressible && meta.len() <= 5 * 1024 * 1024 {
        let bytes = tokio::fs::read(&canonical).await.ok()?;
        if let Some((encoding, data)) = encode_body(&bytes, accept_encoding) {
            let len = data.len();
            let mut resp = builder
                .header(header::CONTENT_ENCODING, encoding)
                .header(header::VARY, "Accept-Encoding")
                .body(Body::from(data))
                .ok()?;
            resp.headers_mut()
                .insert(header::CONTENT_LENGTH, HeaderValue::from(len));
            return Some(resp);
        }
    }
    let file = tokio::fs::File::open(&canonical).await.ok()?;
    let stream = ReaderStream::new(file);
    builder.body(Body::from_stream(stream)).ok()
}

pub fn generic_not_found(st: &AppState, request_id: &str) -> Response<Body> {
    crate::ui::branded_error_with_back(
        StatusCode::NOT_FOUND,
        "Not found",
        None,
        "No project is routed to this host. Apps are served on their own domains; open the dashboard to see your projects.",
        None,
        request_id,
        Some(&st.public_url),
    )
}

#[cfg(test)]
mod tests {
    use super::{hidden_path, parse_authority, safe_join};

    #[test]
    fn forwards_app_cookies_but_not_dok_session() {
        let value = axum::http::HeaderValue::from_static(
            "dok_session=secret; theme=dark; dok_flash=abc; app_session=xyz",
        );
        let filtered = super::filter_cookie_header(&value).unwrap();
        assert_eq!(filtered, "theme=dark; app_session=xyz");
        let only_dok = axum::http::HeaderValue::from_static("dok_session=secret");
        assert!(super::filter_cookie_header(&only_dok).is_none());
    }

    #[test]
    fn basic_auth_limiter_key_includes_username() {
        let req = axum::http::Request::builder()
            .header(
                axum::http::header::AUTHORIZATION,
                format!(
                    "Basic {}",
                    base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        "alice:wrong"
                    )
                ),
            )
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(super::basic_auth_user(&req).as_deref(), Some("alice"));
    }
    use std::path::Path;

    #[test]
    fn routing_never_serves_control_plane() {
        let path = std::env::temp_dir().join(format!("dok-proxy-{}.db", uuid::Uuid::new_v4()));
        let db = crate::db::Db::open(&path).unwrap();
        let base = crate::db::App {
            name: "plain".into(),
            team_id: Some("t".into()),
            kind: "static".into(),
            image: None,
            memory_mb: None,
            cpus: None,
            host_port: None,
            container_port: None,
            domain: None,
            status: "new".into(),
            dir: None,
            repo: None,
            production_deploy: None,
            framework: None,
            updated_at: 0,
        };
        db.upsert_app(&base).unwrap();
        let mut custom = base.clone();
        custom.name = "custom".into();
        custom.domain = Some("custom.test".into());
        db.upsert_app(&custom).unwrap();
        let data = std::env::temp_dir().join(format!("dok-proxy-data-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data).unwrap();
        let st = crate::state::AppState::new(
            db,
            data,
            Some("apps.test".into()),
            "http://dok.test".into(),
        );
        assert!(super::find_app(&st, "plain.apps.test").is_some());
        assert!(super::find_app(&st, "dok.test").is_none());
        assert!(super::find_app(&st, "apps.test").is_none());
        assert!(super::find_app(&st, "custom.apps.test").is_none());
        assert!(super::find_app(&st, "custom.test").is_some());
        assert!(super::find_app(&st, "anything.apps.test").is_none());
        let id = "abcdef1234567890";
        st.db
            .add_deploy(id, "plain", "ok", Some("/tmp/preview"), "preview", None)
            .unwrap();
        let route = super::find_route(&st, "plain-abcdef12.apps.test").unwrap();
        assert_eq!(route.deploy.unwrap().id, id);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn hides_dotfiles() {
        assert!(hidden_path("/.env"));
        assert!(hidden_path("/assets/.git/config"));
        assert!(!hidden_path("/.well-known/acme-challenge/x"));
        assert!(!hidden_path("/index.html"));
    }

    #[test]
    fn joins_normal_paths() {
        let p = safe_join(Path::new("/srv/app"), "/assets/app.js").unwrap();
        assert_eq!(p, Path::new("/srv/app/assets/app.js"));
    }

    #[test]
    fn rejects_traversal() {
        assert!(safe_join(Path::new("/srv/app"), "/../etc/passwd").is_none());
        assert!(safe_join(Path::new("/srv/app"), "/a/../../b").is_none());
    }

    #[test]
    fn ignores_dot_segments() {
        let p = safe_join(Path::new("/srv/app"), "/./a/./b").unwrap();
        assert_eq!(p, Path::new("/srv/app/a/b"));
    }

    #[test]
    fn parses_hosts() {
        assert_eq!(parse_authority("Example.COM:8080"), "example.com");
        assert_eq!(parse_authority("example.com."), "example.com");
        assert_eq!(parse_authority("[::1]:8080"), "[::1]");
        assert_eq!(parse_authority("[::1]"), "[::1]");
        assert_eq!(parse_authority(""), "");
    }

    #[test]
    fn negotiates_brotli_and_gzip() {
        let input = b"hello hello hello hello hello";
        let (enc, data) = super::encode_body(input, "gzip, deflate").unwrap();
        assert_eq!(enc, "gzip");
        assert!(!data.is_empty());
        let (enc, data) = super::encode_body(input, "br, gzip").unwrap();
        assert_eq!(enc, "br");
        assert!(!data.is_empty());
        assert!(super::encode_body(input, "identity").is_none());
        assert!(super::compressible_type("text/html; charset=utf-8"));
        assert!(super::compressible_type("application/json"));
        assert!(!super::compressible_type("image/png"));
    }
}
