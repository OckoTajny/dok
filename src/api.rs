use crate::deploy::{self, DeployOutcome};
use crate::proxy;
use crate::state::AppState;
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

pub fn router(st: AppState) -> Router {
    let public = Router::new().route("/health", get(health));

    let protected = Router::new()
        .route("/apps", get(list_apps))
        .route("/apps/{app}", get(get_app).delete(delete_app))
        .route("/apps/{app}/deploy", post(deploy_upload))
        .route("/apps/{app}/logs", get(logs))
        .route("/apps/{app}/env", put(put_env).get(get_env))
        .route("/apps/{app}/domain", post(set_domain))
        .route("/apps/{app}/repo", post(set_repo))
        .route("/apps/{app}/git-init", post(git_init))
        .route("/apps/{app}/redeploy", post(redeploy))
        .route_layer(middleware::from_fn_with_state(st.clone(), require_token));

    Router::new()
        .nest("/api/v1", public.merge(protected))
        .route("/api/v1/hooks/github/{app}", post(github_hook))
        .fallback(proxy_fallback)
        .layer(DefaultBodyLimit::max(512 * 1024 * 1024))
        .with_state(st)
}

async fn proxy_fallback(State(st): State<AppState>, req: Request) -> Response {
    if let Some(app) = proxy::app_for(&st, &req) {
        return proxy::serve_app(st, req, app).await;
    }
    if let Some(r) = crate::auth::handle(&st, req).await {
        return r;
    }
    proxy::landing(&st)
}

async fn health() -> impl IntoResponse {
    Json(json!({"ok": true, "version": env!("CARGO_PKG_VERSION")}))
}

fn hash_token(t: &str) -> String {
    format!("{:x}", Sha256::digest(t.as_bytes()))
}

async fn require_token(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| st.db.token_exists(&hash_token(t)).unwrap_or(false))
        .unwrap_or(false);
    if !ok {
        return (StatusCode::UNAUTHORIZED, "missing or invalid token\n").into_response();
    }
    next.run(req).await
}

async fn list_apps(State(st): State<AppState>) -> Response {
    match st.db.list_apps() {
        Ok(apps) => Json(json!({ "apps": apps })).into_response(),
        Err(e) => internal(e),
    }
}

async fn get_app(State(st): State<AppState>, Path(app): Path<String>) -> Response {
    match st.db.get_app(&app) {
        Ok(Some(a)) => Json(a).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "no such app\n").into_response(),
        Err(e) => internal(e),
    }
}

async fn delete_app(State(st): State<AppState>, Path(app): Path<String>) -> Response {
    deploy::stop_container(&app).await;
    match st.db.delete_app(&app) {
        Ok(_) => Json(json!({"deleted": app})).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct DeployQuery {
    port: Option<u16>,
    domain: Option<String>,
}

async fn deploy_upload(
    State(st): State<AppState>,
    Path(app): Path<String>,
    Query(q): Query<DeployQuery>,
    req: Request,
) -> Response {
    if !valid_name(&app) {
        return (StatusCode::BAD_REQUEST, "invalid app name\n").into_response();
    }
    let tmp = st.data.join("tmp");
    if let Err(e) = tokio::fs::create_dir_all(&tmp).await {
        return internal(e.into());
    }
    let archive = tmp.join(format!("{}.tar.gz", uuid::Uuid::new_v4().simple()));

    if let Err(e) = save_body(req, &archive).await {
        let _ = tokio::fs::remove_file(&archive).await;
        return internal(e);
    }

    let dir = match deploy::new_build_dir(&st, &app) {
        Ok(d) => d,
        Err(e) => return internal(e),
    };
    let (archive2, dir2) = (archive.clone(), dir.clone());
    let extracted = tokio::task::spawn_blocking(move || {
        deploy::extract_tarball(&archive2, &dir2)
    })
    .await;

    let _ = tokio::fs::remove_file(&archive).await;
    match extracted {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return internal(e),
        Err(e) => return internal(e.into()),
    }

    match deploy::deploy_dir(&st, &app, &dir, q.port, q.domain).await {
        Ok(out) => Json(out).into_response(),
        Err(e) => {
            let _ = st.db.set_status(&app, "failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}

async fn save_body(req: Request, path: &PathBuf) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    let mut file = tokio::fs::File::create(path).await?;
    let mut stream = req.into_body().into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    Ok(())
}

async fn logs(State(st): State<AppState>, Path(app): Path<String>) -> Response {
    let build = st.db.last_deploy_log(&app).unwrap_or_default();
    let runtime = deploy::container_logs(&app, 200).await;
    Json(json!({"build": build.unwrap_or_default(), "runtime": runtime})).into_response()
}

async fn put_env(
    State(st): State<AppState>,
    Path(app): Path<String>,
    Json(vars): Json<HashMap<String, String>>,
) -> Response {
    let kv: Vec<(String, String)> = vars.into_iter().collect();
    match st.db.put_env(&app, &kv) {
        Ok(_) => Json(json!({"ok": true, "keys": st.db.env(&app).unwrap_or_default()
            .into_iter().map(|(k, _)| k).collect::<Vec<_>>()})).into_response(),
        Err(e) => internal(e),
    }
}

async fn get_env(State(st): State<AppState>, Path(app): Path<String>) -> Response {
    match st.db.env(&app) {
        Ok(kv) => Json(json!({"env": kv.into_iter().collect::<HashMap<_, _>>()})).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct DomainBody {
    domain: String,
}

async fn set_domain(
    State(st): State<AppState>,
    Path(app): Path<String>,
    Json(b): Json<DomainBody>,
) -> Response {
    if !valid_host(&b.domain) {
        return (StatusCode::BAD_REQUEST, "invalid domain\n").into_response();
    }
    match st.db.set_domain(&app, &b.domain) {
        Ok(_) => Json(json!({"ok": true, "domain": b.domain})).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct RepoBody {
    repo: String,
}

async fn set_repo(
    State(st): State<AppState>,
    Path(app): Path<String>,
    Json(b): Json<RepoBody>,
) -> Response {
    match st.db.set_repo(&app, &b.repo) {
        Ok(_) => Json(json!({"ok": true, "repo": b.repo})).into_response(),
        Err(e) => internal(e),
    }
}

async fn git_init(State(st): State<AppState>, Path(app): Path<String>) -> Response {
    if !matches!(st.db.get_app(&app), Ok(Some(_))) {
        return (StatusCode::NOT_FOUND, "no such app\n").into_response();
    }
    match init_bare_repo(&st, &app) {
        Ok((path, hook)) => {
            let _ = st.db.set_repo(&app, &path);
            Json(json!({
                "app": app,
                "path": path,
                "remote": format!("ssh://<user>@<host>{path}"),
                "hint": format!("git remote add dok ssh://<user>@<host>{path} && git push dok main"),
                "hook": hook,
            }))
            .into_response()
        }
        Err(e) => internal(e),
    }
}

fn init_bare_repo(st: &AppState, app: &str) -> anyhow::Result<(String, String)> {
    let dir = st.data.join("repos").join(format!("{app}.git"));
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !dir.exists() {
        let out = std::process::Command::new("git")
            .args(["init", "--bare", "--quiet"])
            .arg(&dir)
            .output()?;
        if !out.status.success() {
            anyhow::bail!("git init failed: {}", String::from_utf8_lossy(&out.stderr));
        }
    }
    let exe = std::env::current_exe()?;
    let log = st.data.join("logs").join(format!("{app}.log"));
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let hook_body = format!(
        "#!/bin/sh\nexec '{}' deploy-git '{}' --data '{}' >> '{}' 2>&1\n",
        exe.display(),
        app,
        st.data.display(),
        log.display()
    );
    let hook_path = dir.join("hooks/post-receive");
    std::fs::write(&hook_path, hook_body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok((dir.to_string_lossy().to_string(), hook_path.to_string_lossy().to_string()))
}

async fn redeploy(State(st): State<AppState>, Path(app): Path<String>) -> Response {
    match deploy_from_repo(&st, &app).await {
        Ok(out) => Json(out).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct HookQuery {
    secret: Option<String>,
}

async fn github_hook(
    State(st): State<AppState>,
    Path(app): Path<String>,
    Query(q): Query<HookQuery>,
    headers: HeaderMap,
) -> Response {
    let expected = std::env::var("DOK_HOOK_SECRET").ok();
    let given = q
        .secret
        .or_else(|| {
            headers
                .get("x-dok-secret")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        })
        .unwrap_or_default();
    match expected {
        Some(exp) if !exp.is_empty() && exp == given => {}
        _ => return (StatusCode::UNAUTHORIZED, "bad hook secret\n").into_response(),
    }
    match deploy_from_repo(&st, &app).await {
        Ok(out) => Json(out).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn deploy_from_repo(st: &AppState, app: &str) -> anyhow::Result<DeployOutcome> {
    let row = st
        .db
        .get_app(app)?
        .ok_or_else(|| anyhow::anyhow!("no such app"))?;
    let repo = row
        .repo
        .ok_or_else(|| anyhow::anyhow!("app has no repo; use `dok git {app} <url>`"))?;

    let dir = deploy::new_build_dir(st, app)?;
    let out = tokio::process::Command::new("git")
        .args(["clone", "--depth", "1", &repo])
        .arg(&dir)
        .output()
        .await?;
    if !out.status.success() {
        anyhow::bail!("git clone failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    deploy::deploy_dir(st, app, &dir, None, None).await
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

fn valid_host(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && s.contains('.')
}

fn internal(e: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": e.to_string()})),
    )
        .into_response()
}

pub fn hash(t: &str) -> String {
    hash_token(t)
}

#[cfg(test)]
mod tests {
    use super::{valid_host, valid_name};

    #[test]
    fn app_names() {
        assert!(valid_name("my-app"));
        assert!(valid_name("app123"));
        assert!(!valid_name("My-App"));
        assert!(!valid_name("-app"));
        assert!(!valid_name("app-"));
        assert!(!valid_name(""));
        assert!(!valid_name("a/b"));
    }

    #[test]
    fn domains() {
        assert!(valid_host("app.example.com"));
        assert!(!valid_host("localhost"));
        assert!(!valid_host("bad host"));
        assert!(!valid_host(""));
    }
}
