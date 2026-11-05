use crate::state::AppState;
use axum::body::Body;
use axum::http::{header, HeaderValue, Request, Response, StatusCode};
use hyper_util::client::legacy::Client;
use std::path::{Component, Path, PathBuf};

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

pub fn app_for(st: &AppState, req: &Request<Body>) -> Option<crate::db::App> {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    find_app(st, &host)
}

pub async fn serve_app(st: AppState, req: Request<Body>, app: crate::db::App) -> Response<Body> {
    match app.kind.as_str() {
        "static" => {
            let dir = app.dir.clone().unwrap_or_default();
            let path = req.uri().path().to_string();
            match serve_static(Path::new(&dir), &path).await {
                Some(r) => r,
                None => landing(&st),
            }
        }
        _ => match app.host_port {
            Some(port) => forward(&st, req, port).await,
            None => bad_gateway("app has no running container"),
        },
    }
}

fn find_app(st: &AppState, host: &str) -> Option<crate::db::App> {
    if host.is_empty() {
        return None;
    }
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

async fn forward(st: &AppState, req: Request<Body>, port: u16) -> Response<Body> {
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
            if name == "host" || HOP_BY_HOP.contains(&name) {
                continue;
            }
            h.insert(k.clone(), v.clone());
        }
        if let Some(host) = parts.headers.get(header::HOST).cloned() {
            h.insert("x-forwarded-host", host);
        }
        if let Some(ip) = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0.ip().to_string())
        {
            h.insert("x-forwarded-for", HeaderValue::from_str(&ip).unwrap());
        }
        h.insert("x-forwarded-proto", HeaderValue::from_static("http"));
    }

    let client: &Client<_, Body> = &st.client;
    match client.request(out.body(body).unwrap()).await {
        Ok(resp) => {
            let (parts, body) = resp.into_parts();
            let mut b = Response::builder().status(parts.status);
            {
                let h = b.headers_mut().unwrap();
                for (k, v) in parts.headers.iter() {
                    if HOP_BY_HOP.contains(&k.as_str()) {
                        continue;
                    }
                    h.insert(k.clone(), v.clone());
                }
            }
            b.body(Body::new(body))
                .unwrap_or_else(|_| bad_gateway("bad upstream response"))
        }
        Err(e) => bad_gateway(&format!("upstream error: {e}")),
    }
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

async fn serve_static(root: &Path, url_path: &str) -> Option<Response<Body>> {
    let root = root.canonicalize().ok()?;
    let mut path = match safe_join(&root, url_path) {
        Some(p) => p,
        None => return Some(Response::builder().status(StatusCode::BAD_REQUEST).body(Body::empty()).unwrap()),
    };
    if path.is_dir() {
        path.push("index.html");
    }
    if !path.exists() {
        // SPA fallback.
        let index = root.join("index.html");
        if url_path != "/" && index.exists() && !url_path.contains('.') {
            path = index;
        } else {
            return None;
        }
    }
    let canonical = path.canonicalize().ok()?;
    if !canonical.starts_with(&root) {
        return Some(Response::builder().status(StatusCode::FORBIDDEN).body(Body::empty()).unwrap());
    }
    let bytes = tokio::fs::read(&canonical).await.ok()?;
    let mime = mime_guess::from_path(&canonical).first_or_octet_stream();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CACHE_CONTROL, "public, max-age=60")
        .body(Body::from(bytes))
        .ok()
}

fn bad_gateway(msg: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(format!("dok: {msg}\n")))
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
    use super::safe_join;
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
}
