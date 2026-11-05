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

pub fn app_for(st: &AppState, req: &Request<Body>) -> Option<crate::db::App> {
    let host = host_of(req);
    if host.is_empty() {
        return None;
    }
    find_app(st, &host)
}

pub async fn serve_app(st: AppState, req: Request<Body>, app: crate::db::App) -> Response<Body> {
    let started = Instant::now();
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let ip = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip().to_string());
    let host = host_of(&req);
    let settings = st.db.proxy_settings(&app.name);

    if let Some(r) = redirect_for(&st, &req, &host, &settings) {
        log_request(&st, &app.name, &method, &path, &r, started, ip.as_deref());
        return apply_headers(r, &settings);
    }
    if let Some(r) = basic_auth_challenge(&req, &settings) {
        log_request(&st, &app.name, &method, &path, &r, started, ip.as_deref());
        return apply_headers(r, &settings);
    }
    if let Some(max) = settings.max_body_mb {
        let too_big = req
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|len| len > max.saturating_mul(1024 * 1024))
            .unwrap_or(false);
        if too_big {
            let r = Response::builder()
                .status(StatusCode::PAYLOAD_TOO_LARGE)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(Body::from("dok: request body too large\n"))
                .unwrap();
            log_request(&st, &app.name, &method, &path, &r, started, ip.as_deref());
            return apply_headers(r, &settings);
        }
    }

    let resp = match app.kind.as_str() {
        "static" => {
            let dir = app.dir.clone().unwrap_or_default();
            let method = req.method().clone();
            let path = req.uri().path().to_string();
            let inm = req
                .headers()
                .get(header::IF_NONE_MATCH)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let accepts_gzip = req
                .headers()
                .get(header::ACCEPT_ENCODING)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.contains("gzip"))
                .unwrap_or(false);
            match serve_static(
                Path::new(&dir),
                &method,
                &path,
                inm.as_deref(),
                settings.compression,
                accepts_gzip,
            )
            .await
            {
                Some(r) => r,
                None => landing(&st),
            }
        }
        _ => match app.host_port {
            Some(port) => forward(&st, req, port).await,
            None => bad_gateway("app has no running container"),
        },
    };
    log_request(&st, &app.name, &method, &path, &resp, started, ip.as_deref());
    apply_headers(resp, &settings)
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
) -> Option<Response<Body>> {
    if host.is_empty() {
        return None;
    }
    let https = st.scheme == "https";
    let forwarded_https = req
        .headers()
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("https"))
        .unwrap_or(false);
    let local = host == "localhost"
        || host == "127.0.0.1"
        || host == "[::1]"
        || host.starts_with("127.");
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
        let scheme = if https || forwarded_https { "https" } else { "http" };
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
        .and_then(|s| s.split_once(':').map(|(u, p)| (u.to_string(), p.to_string())))
        .map(|(u, p)| u == user && crate::db::verify_password(hash, &p))
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

fn apply_headers(mut resp: Response<Body>, settings: &crate::db::ProxySettings) -> Response<Body> {
    for (name, value) in &settings.headers {
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            resp.headers_mut().insert(n, v);
        }
    }
    resp
}

fn find_app(st: &AppState, host: &str) -> Option<crate::db::App> {
    if let Ok(Some(app)) = st.db.app_by_domain(host) {
        return Some(app);
    }
    if let Some(base) = &st.base_domain {
        let base = base.to_ascii_lowercase();
        if let Some(prefix) = host.strip_suffix(&format!(".{base}")) {
            if !prefix.is_empty() && !prefix.contains('.') {
                if let Ok(Some(app)) = st.db.get_app(prefix) {
                    return Some(app);
                }
            }
        }
    }
    None
}

fn is_upgrade(req: &Request<Body>) -> bool {
    req.headers()
        .get(header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_ascii_lowercase().contains("upgrade"))
        .unwrap_or(false)
        && req.headers().contains_key(header::UPGRADE)
}

async fn forward(st: &AppState, mut req: Request<Body>, port: u16) -> Response<Body> {
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
        Err(_) => return bad_gateway("bad upstream uri"),
    };

    let mut out = Request::builder().method(parts.method.clone()).uri(uri);
    {
        let h = out.headers_mut().unwrap();
        for (k, v) in parts.headers.iter() {
            let name = k.as_str();
            if !upgrade && HOP_BY_HOP.contains(&name) {
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
                if let Ok(v) = HeaderValue::from_str(&format!(
                    "{}, {ip}",
                    prev.to_str().unwrap_or("")
                )) {
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
        h.insert("x-forwarded-proto", HeaderValue::from_str(&st.scheme).unwrap());
    }

    let client: &Client<_, Body> = &st.client;
    let resp = match tokio::time::timeout(
        Duration::from_secs(st.upstream_timeout),
        client.request(out.body(body).unwrap()),
    )
    .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return bad_gateway(&format!("upstream error: {e}")),
        Err(_) => return gateway_timeout(),
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
    b.body(Body::new(body))
        .unwrap_or_else(|_| bad_gateway("bad upstream response"))
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

async fn serve_static(
    root: &Path,
    method: &Method,
    url_path: &str,
    if_none_match: Option<&str>,
    compress: bool,
    accepts_gzip: bool,
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
    let is_html = mime.type_() == mime_guess::mime::TEXT
        && mime.subtype() == mime_guess::mime::HTML;
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
    if compress && accepts_gzip && compressible && meta.len() <= 5 * 1024 * 1024 {
        use std::io::Write;
        let bytes = tokio::fs::read(&canonical).await.ok()?;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        if enc.write_all(&bytes).is_ok() {
            if let Ok(gz) = enc.finish() {
                let len = gz.len();
                let mut resp = builder
                    .header(header::CONTENT_ENCODING, "gzip")
                    .header(header::VARY, "Accept-Encoding")
                    .body(Body::from(gz))
                    .ok()?;
                resp.headers_mut()
                    .insert(header::CONTENT_LENGTH, HeaderValue::from(len));
                return Some(resp);
            }
        }
    }
    let file = tokio::fs::File::open(&canonical).await.ok()?;
    let stream = ReaderStream::new(file);
    builder.body(Body::from_stream(stream)).ok()
}

fn bad_gateway(msg: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(format!("dok: {msg}\n")))
        .unwrap()
}

fn gateway_timeout() -> Response<Body> {
    Response::builder()
        .status(StatusCode::GATEWAY_TIMEOUT)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from("dok: upstream timeout\n"))
        .unwrap()
}

pub fn landing(st: &AppState) -> Response<Body> {
    let mut names = Vec::new();
    if let Ok(apps) = st.db.list_apps() {
        for a in apps {
            names.push(a.name);
        }
    }
    let list = names
        .iter()
        .map(|n| format!("<li>{n}</li>"))
        .collect::<String>();
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>dok</title>\
         <style>body{{font:15px system-ui;margin:3rem auto;max-width:38rem;color:#111}}\
         code{{background:#f4f4f4;padding:.1rem .3rem;border-radius:4px}}</style>\
         <h1>dok</h1><p>No app is routed to this host.</p>\
         <p>Apps: <ul>{list}</ul></p>\
         <p>CLI: <code>dok push</code></p>"
    );
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(html))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::{parse_authority, safe_join};
    use std::path::Path;

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
}
