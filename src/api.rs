use crate::deploy::{self, DeployOutcome};
use crate::proxy;
use crate::state::AppState;
use axum::extract::{DefaultBodyLimit, Extension, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tower::ServiceExt;

const MAX_UPLOAD: usize = 200 * 1024 * 1024;
const MAX_WEBHOOK: usize = 1024 * 1024;

#[derive(Clone)]
pub struct Caller {
    pub user: String,
}

pub fn router(st: AppState) -> Router {
    Router::new().fallback(dispatch).with_state(st)
}

async fn dispatch(State(st): State<AppState>, req: Request) -> Response {
    let request_id = new_request_id();
    let host = proxy::host_of(&req);
    let mut resp = if st.is_control_host(&host) {
        // The dashboard, auth and token API are only served on the control
        // host. Unknown project hosts get a generic 404 without a project list.
        let path = req.uri().path().to_string();
        if path == "/api" || path.starts_with("/api/") {
            match api_routes(st.clone()).oneshot(req).await {
                Ok(r) => r,
                Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal\n").into_response(),
            }
        } else if let Some(r) = crate::auth::handle(&st, req).await {
            r
        } else if st.auth.is_none() {
            crate::ui::page_not_configured(&st)
        } else {
            proxy::generic_not_found(&st, &request_id)
        }
    } else {
        // A routed app always wins: its own /api/... paths must reach the app.
        if let Some(app) = proxy::app_for(&st, &req) {
            proxy::serve_app(st, req, app, &request_id).await
        } else {
            proxy::generic_not_found(&st, &request_id)
        }
    };
    if let Ok(v) = axum::http::HeaderValue::from_str(&request_id) {
        resp.headers_mut().entry("x-request-id").or_insert(v);
    }
    resp
}

fn new_request_id() -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    id[..12].to_string()
}

fn api_routes(st: AppState) -> Router {
    let public = Router::new().route("/api/v1/health", get(health));

    let protected = Router::new()
        // New project API
        .route("/api/v1/projects", get(list_projects).post(create_project))
        .route(
            "/api/v1/projects/{app}",
            get(get_project).delete(delete_project),
        )
        .route("/api/v1/projects/{app}/deploy", post(deploy_upload_named))
        .route("/api/v1/projects/{app}/deployments", get(list_deployments))
        .route("/api/v1/projects/{app}/promote/{deploy}", post(promote))
        .route("/api/v1/projects/{app}/rollback/{deploy}", post(rollback))
        .route(
            "/api/v1/projects/{app}/env/{env}",
            get(get_env).put(put_env),
        )
        .route(
            "/api/v1/projects/{app}/domains",
            get(list_domains).post(add_domain),
        )
        .route(
            "/api/v1/projects/{app}/domains/{host}",
            delete(remove_domain),
        )
        .route("/api/v1/projects/{app}/metrics", get(metrics))
        .route("/api/v1/projects/{app}/redeploy", post(redeploy))
        .route("/api/v1/projects/{app}/access-log", get(access_log))
        .route(
            "/api/v1/projects/{app}/proxy",
            get(get_proxy).put(put_proxy),
        )
        .route("/api/v1/whoami", get(whoami))
        .route("/api/v1/tokens", get(list_tokens).post(create_token))
        .route("/api/v1/tokens/{hash}", delete(revoke_token))
        .route(
            "/api/v1/projects/{app}/env/{env}/{key}",
            delete(delete_env_key),
        )
        .route(
            "/api/v1/projects/{app}/webhook",
            get(get_webhook).put(put_webhook),
        )
        .route("/api/v1/projects/{app}/logs", get(logs))
        .route("/api/v1/projects/{app}/logs/stream", get(logs_stream))
        // Old app API, kept as aliases.
        .route("/api/v1/apps", get(list_projects))
        .route(
            "/api/v1/apps/{app}",
            get(get_project).delete(delete_project),
        )
        .route("/api/v1/apps/{app}/deploy", post(deploy_upload_named))
        .route("/api/v1/apps/{app}/logs", get(logs))
        .route("/api/v1/apps/{app}/logs/stream", get(logs_stream))
        .route("/api/v1/apps/{app}/deploys", get(list_deployments))
        .route("/api/v1/apps/{app}/deployments", get(list_deployments))
        .route("/api/v1/apps/{app}/rollback/{deploy}", post(rollback))
        .route("/api/v1/apps/{app}/promote/{deploy}", post(promote))
        .route(
            "/api/v1/apps/{app}/env",
            put(put_env_root).get(get_env_root),
        )
        .route("/api/v1/apps/{app}/domain", post(set_domain))
        .route("/api/v1/apps/{app}/repo", post(set_repo))
        .route("/api/v1/apps/{app}/git-init", post(git_init))
        .route("/api/v1/apps/{app}/redeploy", post(redeploy))
        // Certificates, teams, and the spec upload endpoint.
        .route("/api/v1/certificates", get(list_certificates))
        .route("/api/v1/certificates/{host}/renew", post(renew_certificate))
        .route("/api/v1/teams", get(list_teams).post(create_team_api))
        .route("/api/v1/teams/{team}/members", get(list_members))
        .route(
            "/api/v1/teams/{team}/members/{user}",
            put(put_member).delete(delete_member),
        )
        .route(
            "/api/v1/teams/{team}/invites",
            get(list_invites).post(create_invite),
        )
        .route(
            "/api/v1/teams/{team}/invites/{token}",
            delete(revoke_invite),
        )
        .route("/api/v1/deploy", post(deploy_root))
        .route_layer(middleware::from_fn_with_state(st.clone(), require_token));

    public
        .merge(protected)
        .route("/api/v1/hooks/github/{app}", post(github_hook))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD))
        .with_state(st)
}

async fn health() -> impl IntoResponse {
    Json(json!({"ok": true, "version": env!("CARGO_PKG_VERSION")}))
}

fn hash_token(t: &str) -> String {
    format!("{:x}", Sha256::digest(t.as_bytes()))
}

async fn require_token(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    let Some(token) = token else {
        return (StatusCode::UNAUTHORIZED, "missing token\n").into_response();
    };
    match st.db.token_user(&hash_token(&token)) {
        Ok(Some(Some(user))) => {
            if !st.rate.allow(&format!("api:{user}"), 600, 60) {
                return (StatusCode::TOO_MANY_REQUESTS, "too many requests\n").into_response();
            }
            req.extensions_mut().insert(Caller { user });
            next.run(req).await
        }
        Ok(Some(None)) => (
            StatusCode::UNAUTHORIZED,
            "token is not bound to a user; finish dashboard setup or issue a new token\n",
        )
            .into_response(),
        _ => (StatusCode::UNAUTHORIZED, "invalid token\n").into_response(),
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "no such project\n").into_response()
}

fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, "you need owner or admin rights\n").into_response()
}

fn bad(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, format!("{msg}\n")).into_response()
}

fn domain_conflict(st: &AppState, app: &str, host: &str) -> Result<(), Response> {
    if st.reserved_host(host) || st.is_base_subdomain(host) {
        return Err((StatusCode::CONFLICT, "host is reserved for the dashboard\n").into_response());
    }
    if let Ok(Some(other)) = st.db.domain_owner(host) {
        if other != app {
            return Err(
                (StatusCode::CONFLICT, "domain belongs to another project\n").into_response(),
            );
        }
    }
    if let Ok(Some(other)) = st.db.app_domain_owner(host) {
        if other != app {
            return Err(
                (StatusCode::CONFLICT, "domain belongs to another project\n").into_response(),
            );
        }
    }
    Ok(())
}

fn internal(e: anyhow::Error) -> Response {
    tracing::error!("internal error: {e:#}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "internal error; see the server log"})),
    )
        .into_response()
}

/// Read access: members and viewers may read; non-members get 404 so project
/// names outside the caller's teams are not disclosed.
fn project_for(st: &AppState, caller: &Caller, name: &str) -> Result<crate::db::App, Response> {
    let app = st.db.get_app(name).ok().flatten().ok_or_else(not_found)?;
    let tid = app.team_id.as_deref().ok_or_else(not_found)?;
    let role = st.db.role(tid, &caller.user).ok().flatten();
    if crate::db::can_read(role.as_deref()) {
        Ok(app)
    } else {
        Err(not_found())
    }
}

fn require_write(st: &AppState, caller: &Caller, app: &crate::db::App) -> Result<(), Response> {
    let Some(tid) = &app.team_id else {
        return Err(forbidden());
    };
    let role = st.db.role(tid, &caller.user).ok().flatten();
    if crate::db::can_write(role.as_deref()) {
        Ok(())
    } else if crate::db::can_read(role.as_deref()) {
        Err(forbidden())
    } else {
        Err(not_found())
    }
}

fn team_for_caller(
    st: &AppState,
    caller: &Caller,
    wanted: Option<&str>,
) -> Result<String, Response> {
    let uid = &caller.user;
    let teams = st.db.teams_for(uid).unwrap_or_default();
    if let Some(w) = wanted {
        let team = teams
            .iter()
            .find(|t| t.slug == w || t.id == w)
            .ok_or_else(not_found)?;
        let role = st.db.role(&team.id, uid).ok().flatten();
        if !crate::db::can_write(role.as_deref()) {
            return Err(forbidden());
        }
        return Ok(team.id.clone());
    }
    teams
        .into_iter()
        .find(|t| {
            let role = st.db.role(&t.id, uid).ok().flatten();
            crate::db::can_write(role.as_deref())
        })
        .map(|t| t.id)
        .ok_or_else(forbidden)
}

fn author_of(st: &AppState, caller: &Caller) -> Option<String> {
    st.db.user(&caller.user).ok().flatten().map(|u| u.email)
}

async fn list_projects(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Response {
    let mut apps = Vec::new();
    for team in st.db.teams_for(&caller.user).unwrap_or_default() {
        apps.extend(st.db.apps_for_team(&team.id).unwrap_or_default());
    }
    apps.sort_by(|a, b| a.name.cmp(&b.name));
    Json(json!({"apps": apps, "projects": apps})).into_response()
}

#[derive(Deserialize)]
struct NewProject {
    name: String,
    team: Option<String>,
}

async fn create_project(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Json(b): Json<NewProject>,
) -> Response {
    if !valid_name(&b.name) {
        return bad("invalid project name");
    }
    let team_id = match team_for_caller(&st, &caller, b.team.as_deref()) {
        Ok(t) => t,
        Err(r) => return r,
    };
    let app = crate::db::App {
        name: b.name.clone(),
        team_id: Some(team_id),
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
        updated_at: crate::db::now(),
    };
    match st.db.create_app_if_absent(&app) {
        Ok(true) => {
            st.sync_traefik();
            Json(app).into_response()
        }
        Ok(false) => match st.db.get_app(&b.name) {
            Ok(Some(existing)) => {
                if let Err(r) = require_write(&st, &caller, &existing) {
                    return r;
                }
                (StatusCode::CONFLICT, "project already exists\n").into_response()
            }
            _ => internal(anyhow::anyhow!("project disappeared")),
        },
        Err(e) => internal(e),
    }
}

async fn get_project(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
) -> Response {
    match project_for(&st, &caller, &app) {
        Ok(a) => {
            let mut v = serde_json::to_value(&a).unwrap_or_else(|_| json!({}));
            v["url"] = json!(project_url(&st, &a));
            Json(v).into_response()
        }
        Err(r) => r,
    }
}

async fn delete_project(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = require_write(&st, &caller, &a) {
        return r;
    }
    deploy::stop_container(&app, "production").await;
    deploy::stop_container(&app, "preview").await;
    deploy::stop_all_previews(&app).await;
    let _ = tokio::fs::remove_dir_all(st.builds_dir().join(&app)).await;
    let _ = tokio::fs::remove_dir_all(st.data.join("repos").join(format!("{app}.git"))).await;
    match st.db.delete_app(&app) {
        Ok(_) => {
            st.sync_traefik();
            Json(json!({"deleted": app})).into_response()
        }
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct DeployQuery {
    port: Option<u16>,
    domain: Option<String>,
    env: Option<String>,
}

fn normalize_env(env: &str) -> Option<&'static str> {
    match env {
        "preview" => Some("preview"),
        "" | "production" => Some("production"),
        _ => None,
    }
}

async fn deploy_upload_named(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
    Query(q): Query<DeployQuery>,
    headers: HeaderMap,
    req: Request,
) -> Response {
    let mut q = q;
    if let Some(env) = headers.get("x-dok-env").and_then(|v| v.to_str().ok()) {
        if !env.is_empty() {
            q.env = Some(env.to_string());
        }
    }
    upload(st, caller, app, q, req).await
}

async fn deploy_root(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    req: Request,
) -> Response {
    let app = headers
        .get("x-dok-project")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let env = headers
        .get("x-dok-env")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("production")
        .to_string();
    upload(
        st,
        caller,
        app,
        DeployQuery {
            port: None,
            domain: None,
            env: Some(env),
        },
        req,
    )
    .await
}

async fn upload(
    st: AppState,
    caller: Caller,
    app: String,
    q: DeployQuery,
    req: Request,
) -> Response {
    if !valid_name(&app) {
        return bad("invalid app name");
    }
    if !st.rate.allow(&format!("deploy:{}", caller.user), 20, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "deploy rate limit exceeded\n",
        )
            .into_response();
    }
    let env = match normalize_env(q.env.as_deref().unwrap_or("production")) {
        Some(e) => e,
        None => return bad("env must be production or preview"),
    };

    let existing = match st.db.get_app(&app) {
        Ok(a) => a,
        Err(e) => return internal(e),
    };
    let team_id = if let Some(a) = &existing {
        if let Err(r) = require_write(&st, &caller, a) {
            return r;
        }
        a.team_id.clone()
    } else {
        match team_for_caller(&st, &caller, None) {
            Ok(t) => Some(t),
            Err(r) => return r,
        }
    };

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
        Err(e) => {
            let _ = tokio::fs::remove_file(&archive).await;
            return internal(e);
        }
    };
    let (archive2, dir2) = (archive.clone(), dir.clone());
    let extracted =
        tokio::task::spawn_blocking(move || deploy::extract_tarball(&archive2, &dir2)).await;
    let _ = tokio::fs::remove_file(&archive).await;
    match extracted {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return internal(e),
        Err(e) => return internal(e.into()),
    }

    if existing.is_none() {
        let row = crate::db::App {
            name: app.clone(),
            team_id,
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
            updated_at: crate::db::now(),
        };
        match st.db.create_app_if_absent(&row) {
            Ok(true) => {}
            Ok(false) => {
                // Another concurrent first push won the race; it must also be
                // a project this caller is allowed to write to.
                match st.db.get_app(&app) {
                    Ok(Some(a)) => {
                        if let Err(r) = require_write(&st, &caller, &a) {
                            return r;
                        }
                    }
                    _ => return internal(anyhow::anyhow!("project disappeared")),
                }
            }
            Err(e) => return internal(e),
        }
    }

    let author = author_of(&st, &caller);
    match deploy::deploy_dir(&st, &app, &dir, q.port, q.domain, env, author.as_deref()).await {
        Ok(out) => Json(out).into_response(),
        Err(e) => {
            if env == "production" {
                let _ = st.db.set_status(&app, "failed");
            }
            internal(e)
        }
    }
}

fn over_upload_limit(total: usize) -> bool {
    total > MAX_UPLOAD
}

async fn save_body(req: Request, path: &PathBuf) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    let mut file = tokio::fs::File::create(path).await?;
    let mut stream = req.into_body().into_data_stream();
    let mut total = 0usize;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        total += chunk.len();
        if over_upload_limit(total) {
            let _ = tokio::fs::remove_file(path).await;
            anyhow::bail!("upload exceeds {} MB", MAX_UPLOAD / (1024 * 1024));
        }
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    Ok(())
}

#[derive(Deserialize)]
struct LogQuery {
    env: Option<String>,
    limit: Option<usize>,
    deploy: Option<String>,
}

async fn logs(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
    Query(q): Query<LogQuery>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let env = normalize_env(q.env.as_deref().unwrap_or("production")).unwrap_or("production");
    let selected = q
        .deploy
        .as_deref()
        .and_then(|id| st.db.deploy(&app, id).ok().flatten());
    let build = match &selected {
        Some(d) => st.db.deploy_log(&app, &d.id).ok().flatten(),
        None => st.db.last_deploy_log(&app).ok().flatten(),
    };
    let kind = selected
        .as_ref()
        .map(|d| {
            if d.image.is_some() {
                "docker"
            } else {
                "static"
            }
        })
        .unwrap_or(a.kind.as_str());
    let container = selected.as_ref().and_then(|d| d.container.clone());
    let runtime = if kind == "docker" {
        deploy::container_logs(&app, env, container.as_deref(), 200).await
    } else {
        "no container (static app)".to_string()
    };
    Json(json!({
        "build": build.unwrap_or_default(),
        "runtime": runtime,
        "env": env,
        "deploy": selected.map(|d| d.id),
    }))
    .into_response()
}

async fn logs_stream(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
    Query(q): Query<LogQuery>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let deploy = q
        .deploy
        .as_deref()
        .and_then(|id| st.db.deploy(&app, id).ok().flatten());
    let env = deploy
        .as_ref()
        .map(|d| d.env.clone())
        .or_else(|| q.env.as_deref().and_then(normalize_env).map(str::to_string))
        .unwrap_or_else(|| "production".into());
    let kind = deploy
        .as_ref()
        .map(|d| {
            if d.image.is_some() {
                "docker"
            } else {
                "static"
            }
        })
        .unwrap_or(a.kind.as_str());
    let container = deploy
        .as_ref()
        .and_then(|d| d.container.clone())
        .or_else(|| {
            st.db
                .deploys_env(&app, Some(&env), 1)
                .unwrap_or_default()
                .first()
                .and_then(|d| d.container.clone())
        });
    log_stream_response(&st, &app, &env, kind, container.as_deref()).await
}

/// Shared live container log stream for token API and session dashboard.
pub async fn log_stream_response(
    st: &AppState,
    app: &str,
    env: &str,
    kind: &str,
    container: Option<&str>,
) -> Response {
    if kind != "docker" {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(axum::body::Body::from("no container (static app)\n"))
            .unwrap();
    }
    if !crate::deploy::docker_available().await {
        return (StatusCode::SERVICE_UNAVAILABLE, "docker unavailable\n").into_response();
    }
    let permit = match st.stream_gate.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return (StatusCode::TOO_MANY_REQUESTS, "too many log streams\n").into_response(),
    };
    let name = container
        .map(str::to_string)
        .unwrap_or_else(|| deploy::container_name(app, env));
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(64);
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        use std::io::{BufRead, BufReader};
        use std::process::Stdio;
        let mut child = match std::process::Command::new("docker")
            .args(["logs", "-f", "--tail", "100", &name])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.blocking_send(Err(e));
                return;
            }
        };
        let stderr = child.stderr.take();
        let err_tx = tx.clone();
        if let Some(err) = stderr {
            std::thread::spawn(move || {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    if err_tx
                        .blocking_send(Ok(bytes::Bytes::from(format!("{line}\n"))))
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        if let Some(out) = child.stdout.take() {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if tx
                    .blocking_send(Ok(bytes::Bytes::from(format!("{line}\n"))))
                    .is_err()
                {
                    let _ = child.kill();
                    break;
                }
            }
        }
        let _ = child.wait();
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("x-accel-buffering", "no")
        .body(axum::body::Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .unwrap()
}

async fn list_deployments(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
    Query(q): Query<LogQuery>,
) -> Response {
    if let Err(r) = project_for(&st, &caller, &app) {
        return r;
    }
    let env = q.env.as_deref().filter(|e| !e.is_empty());
    match st.db.deploys_env(&app, env, 50) {
        Ok(d) => {
            let deployments = d.clone();
            Json(json!({"deploys": d, "deployments": deployments})).into_response()
        }
        Err(e) => internal(e),
    }
}

async fn promote(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path((app, deploy_id)): Path<(String, String)>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = require_write(&st, &caller, &a) {
        return r;
    }
    let author = author_of(&st, &caller);
    match deploy::promote(&st, &app, &deploy_id, author.as_deref()).await {
        Ok(out) => Json(out).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn rollback(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path((app, deploy_id)): Path<(String, String)>,
) -> Response {
    promote(State(st), Extension(caller), Path((app, deploy_id))).await
}

async fn put_env(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path((app, env)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = require_write(&st, &caller, &a) {
        return r;
    }
    if !matches!(env.as_str(), "production" | "preview" | "development") {
        return bad("env must be production, preview or development");
    }
    let (vars, replace) = match parse_env_body(body) {
        Ok(v) => v,
        Err(msg) => return bad(msg),
    };
    put_env_impl(&st, &app, &env, vars, replace)
}

async fn put_env_root(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    put_env(
        State(st),
        Extension(caller),
        Path((app, "production".into())),
        Json(body),
    )
    .await
}

fn parse_env_body(
    body: Value,
) -> Result<(HashMap<String, (String, Option<bool>)>, bool), &'static str> {
    let (vars, replace) = if body.get("vars").is_some() {
        let replace = body
            .get("replace")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        (
            body.get("vars").cloned().unwrap_or_else(|| json!({})),
            replace,
        )
    } else {
        (body.clone(), false)
    };
    let map = vars.as_object().ok_or("env body must be an object")?;
    let mut out = HashMap::new();
    for (k, v) in map {
        let (value, secret) = match v {
            Value::String(s) => (s.clone(), None),
            Value::Object(o) => {
                let value = o
                    .get("value")
                    .and_then(Value::as_str)
                    .ok_or("object env values need a string value field")?;
                (value.to_string(), o.get("secret").and_then(Value::as_bool))
            }
            _ => return Err("env values must be strings or {value, secret} objects"),
        };
        out.insert(k.clone(), (value, secret));
    }
    // `dok env --secret KEY=VALUE` and simple API clients can also send
    // {"vars": {...}, "secrets": ["KEY"]}.
    if let Some(secrets) = body.get("secrets").and_then(Value::as_array) {
        for key in secrets.iter().filter_map(Value::as_str) {
            if let Some((_, secret)) = out.get_mut(key) {
                *secret = Some(true);
            }
        }
    }
    Ok((out, replace))
}

fn put_env_impl(
    st: &AppState,
    app: &str,
    env: &str,
    vars: HashMap<String, (String, Option<bool>)>,
    replace: bool,
) -> Response {
    let existing_rows = st.db.env(app, env).unwrap_or_default();
    let existing: HashMap<String, bool> = existing_rows
        .iter()
        .map(|(k, _, s)| (k.clone(), *s))
        .collect();
    for (k, (v, _)) in &vars {
        if !valid_env_key(k) {
            return bad("env keys must be letters, digits and _, and cannot start with a digit");
        }
        if v.len() > 64 * 1024 {
            return bad("env value too large");
        }
    }
    if replace {
        for (k, _, _) in &existing_rows {
            if !vars.contains_key(k) {
                let _ = st.db.delete_env(app, env, k);
            }
        }
    }
    let kv: Vec<(String, String, bool)> = vars
        .into_iter()
        .map(|(k, (v, explicit))| {
            let secret = explicit.unwrap_or_else(|| existing.get(&k).copied().unwrap_or(false));
            (k, v, secret)
        })
        .collect();
    match st.db.put_env(app, env, &kv) {
        Ok(_) => Json(json!({
            "ok": true,
            "env": env,
            "keys": st.db.env(app, env).unwrap_or_default()
                .into_iter().map(|(k, _, _)| k).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(e) => internal(e),
    }
}

async fn get_env(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path((app, env)): Path<(String, String)>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !matches!(env.as_str(), "production" | "preview" | "development") {
        return bad("env must be production, preview or development");
    }
    let role = a
        .team_id
        .as_deref()
        .and_then(|tid| st.db.role(tid, &caller.user).ok().flatten());
    let can_see_values = crate::db::can_write(role.as_deref());
    match st.db.env(&app, &env) {
        Ok(kv) => {
            // Secret values are write-only even for owner/admin, and inherited
            // production secrets stay masked in preview/development.
            let specific: HashMap<String, bool> =
                kv.iter().map(|(k, _, s)| (k.clone(), *s)).collect();
            let production: HashMap<String, bool> = st
                .db
                .env(&app, "production")
                .unwrap_or_default()
                .into_iter()
                .map(|(k, _, s)| (k, s))
                .collect();
            let is_secret = |k: &str| {
                specific
                    .get(k)
                    .or_else(|| production.get(k))
                    .copied()
                    .unwrap_or(false)
            };
            let env_map: HashMap<String, String> = kv
                .into_iter()
                .map(|(k, v, secret)| {
                    let v = if can_see_values && !secret {
                        v
                    } else {
                        "***".into()
                    };
                    (k, v)
                })
                .collect();
            let resolved = st
                .db
                .env_resolved(&app, &env)
                .unwrap_or_default()
                .into_iter()
                .map(|(k, v)| {
                    if can_see_values && !is_secret(&k) {
                        (k, v)
                    } else {
                        (k, "***".to_string())
                    }
                })
                .collect::<HashMap<_, _>>();
            Json(json!({
                "env": env_map,
                "resolved": resolved,
                "values_visible": can_see_values,
                "secrets_masked": true,
            }))
            .into_response()
        }
        Err(e) => internal(e),
    }
}

async fn get_env_root(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
) -> Response {
    get_env(
        State(st),
        Extension(caller),
        Path((app, "production".into())),
    )
    .await
}

async fn whoami(State(st): State<AppState>, Extension(caller): Extension<Caller>) -> Response {
    let user = st.db.user(&caller.user).ok().flatten();
    let teams = st.db.teams_for(&caller.user).unwrap_or_default();
    Json(json!({
        "user": user,
        "teams": teams,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct TokenBody {
    #[serde(default)]
    name: String,
}

async fn list_tokens(State(st): State<AppState>, Extension(caller): Extension<Caller>) -> Response {
    let tokens: Vec<Value> = st
        .db
        .tokens_for_user(&caller.user)
        .unwrap_or_default()
        .into_iter()
        .map(|(hash, name, created_at)| json!({"id": hash, "name": name, "created_at": created_at}))
        .collect();
    Json(json!({"tokens": tokens})).into_response()
}

async fn create_token(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Json(b): Json<TokenBody>,
) -> Response {
    let name = b.name.trim();
    let name: String = if name.is_empty() {
        "cli".into()
    } else {
        name.chars().take(64).collect()
    };
    match st.db.create_token_for_user(&caller.user, &name) {
        Ok(token) => Json(json!({
            "token": token,
            "name": name,
            "login": format!("dok login {} {token}", st.public_url),
        }))
        .into_response(),
        Err(e) => internal(e),
    }
}

async fn revoke_token(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(hash): Path<String>,
) -> Response {
    match st.db.revoke_token_for_user(&caller.user, &hash) {
        Ok(0) => not_found(),
        Ok(_) => Json(json!({"revoked": hash})).into_response(),
        Err(e) => internal(e),
    }
}

async fn delete_env_key(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path((app, env, key)): Path<(String, String, String)>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = require_write(&st, &caller, &a) {
        return r;
    }
    if !matches!(env.as_str(), "production" | "preview" | "development") {
        return bad("env must be production, preview or development");
    }
    match st.db.delete_env(&app, &env, &key) {
        Ok(_) => Json(json!({"deleted": key, "env": env})).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct WebhookBody {
    secret: Option<String>,
    branch: Option<String>,
}

async fn get_webhook(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = require_write(&st, &caller, &a) {
        return r;
    }
    let secret = st
        .db
        .get_setting(&format!("webhook_secret:{app}"))
        .ok()
        .flatten();
    let branch = st
        .db
        .get_setting(&format!("webhook_branch:{app}"))
        .ok()
        .flatten();
    Json(json!({"secret": secret, "branch": branch, "configured": secret.is_some()}))
        .into_response()
}

async fn put_webhook(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
    Json(b): Json<WebhookBody>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = require_write(&st, &caller, &a) {
        return r;
    }
    if let Some(secret) = b.secret.as_deref().filter(|s| !s.is_empty()) {
        if secret.len() < 16 {
            return bad("webhook secret must be at least 16 characters");
        }
        if let Err(e) = st.db.set_setting(&format!("webhook_secret:{app}"), secret) {
            return internal(e);
        }
    }
    if let Some(branch) = b.branch.as_deref() {
        if !branch.is_empty() && !valid_branch(branch) {
            return bad("invalid branch name");
        }
        let _ = st.db.set_setting(&format!("webhook_branch:{app}"), branch);
    }
    Json(json!({"ok": true})).into_response()
}

#[derive(Deserialize)]
struct DomainBody {
    domain: String,
}

async fn set_domain(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
    Json(b): Json<DomainBody>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = require_write(&st, &caller, &a) {
        return r;
    }
    if !valid_host(&b.domain) {
        return bad("invalid domain");
    }
    let domain = b.domain.trim_end_matches('.').to_ascii_lowercase();
    if let Err(r) = domain_conflict(&st, &app, &domain) {
        return r;
    }
    match st.db.set_domain(&app, &domain) {
        Ok(_) => {
            st.sync_traefik();
            Json(json!({"ok": true, "domain": domain})).into_response()
        }
        Err(e) => internal(e),
    }
}

async fn list_domains(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
) -> Response {
    if let Err(r) = project_for(&st, &caller, &app) {
        return r;
    }
    match st.db.get_app(&app) {
        Ok(Some(row)) => {
            let mut domains = st.db.domains_for(&app).unwrap_or_default();
            let primary = row.domain.clone();
            if let Some(p) = &primary {
                if !domains.contains(p) {
                    domains.push(p.clone());
                }
            }
            domains.sort();
            domains.dedup();
            Json(json!({"domains": domains, "primary": primary})).into_response()
        }
        Ok(None) => not_found(),
        Err(e) => internal(e),
    }
}

async fn add_domain(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
    Json(b): Json<DomainBody>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = require_write(&st, &caller, &a) {
        return r;
    }
    let host = b.domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if !valid_host(&host) {
        return bad("invalid domain");
    }
    if let Err(r) = domain_conflict(&st, &app, &host) {
        return r;
    }
    match st.db.add_domain(&app, &host) {
        Ok(_) => {
            st.sync_traefik();
            Json(json!({"ok": true, "domain": host})).into_response()
        }
        Err(e) => internal(e),
    }
}

async fn remove_domain(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path((app, host)): Path<(String, String)>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = require_write(&st, &caller, &a) {
        return r;
    }
    let additional = st.db.remove_domain(&app, &host).unwrap_or(0);
    let primary =
        a.domain.as_deref() == Some(host.trim_end_matches('.').to_ascii_lowercase().as_str());
    if primary {
        if let Err(e) = st.db.clear_domain(&app) {
            return internal(e);
        }
    }
    if additional == 0 && !primary {
        return not_found();
    }
    st.sync_traefik();
    Json(json!({"removed": host, "primary": primary})).into_response()
}

async fn set_repo(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
    Json(b): Json<DomainBody>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = require_write(&st, &caller, &a) {
        return r;
    }
    if !valid_repo_url(&b.domain) {
        return bad("repo must be https://, ssh:// or git@host:path");
    }
    match st.db.set_repo(&app, &b.domain) {
        Ok(_) => Json(json!({"ok": true, "repo": b.domain})).into_response(),
        Err(e) => internal(e),
    }
}

async fn metrics(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let all = st.db.deploys(&app, 20).unwrap_or_default();
    let (total, production, preview) =
        st.db
            .deploy_counts(&app)
            .unwrap_or((all.len() as i64, 0, 0));
    let latest = all
        .first()
        .map(|d| json!({"id": d.id, "status": d.status, "env": d.env, "created_at": d.created_at}));
    let mut domains = st.db.domains_for(&app).unwrap_or_default().len();
    if a.domain.is_some()
        && !a
            .domain
            .as_ref()
            .map(|d| st.db.domains_for(&app).unwrap_or_default().contains(d))
            .unwrap_or(false)
    {
        domains += 1;
    }
    let stats = deploy::container_stats(&app, "production", None).await;
    Json(json!({
        "project": a.name,
        "status": a.status,
        "kind": a.kind,
        "production_deploy": a.production_deploy,
        "deployments": {"total": total, "production": production, "preview": preview},
        "latest": latest,
        "domains": domains,
        "env_keys": {
            "production": st.db.env(&app, "production").unwrap_or_default().len(),
            "preview": st.db.env(&app, "preview").unwrap_or_default().len(),
            "development": st.db.env(&app, "development").unwrap_or_default().len(),
        },
        "runtime": stats,
    }))
    .into_response()
}

async fn access_log(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
    Query(q): Query<LogQuery>,
) -> Response {
    if let Err(r) = project_for(&st, &caller, &app) {
        return r;
    }
    let limit = q.limit.unwrap_or(50).min(200);
    match st.db.access_logs(&app, limit) {
        Ok(logs) => Json(json!({"requests": logs})).into_response(),
        Err(e) => internal(e),
    }
}

async fn get_proxy(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
) -> Response {
    if let Err(r) = project_for(&st, &caller, &app) {
        return r;
    }
    let s = st.db.proxy_settings(&app);
    let mut v = serde_json::to_value(&s).unwrap_or_else(|_| json!({}));
    v["basic_auth_configured"] = json!(s.basic_auth_hash.is_some());
    v["basic_auth_hash"] = json!(null);
    Json(v).into_response()
}

#[derive(Deserialize)]
struct ProxyBody {
    #[serde(flatten)]
    settings: crate::db::ProxySettings,
    basic_auth_password: Option<String>,
}

async fn put_proxy(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
    Json(mut b): Json<ProxyBody>,
) -> Response {
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = require_write(&st, &caller, &a) {
        return r;
    }
    if let Some(pw) = b.basic_auth_password.as_deref().filter(|p| !p.is_empty()) {
        b.settings.basic_auth_hash = Some(crate::db::hash_password(pw));
    } else if b.settings.basic_auth_hash.is_none() {
        // The dashboard/API never sends the stored hash back. Preserve it when
        // the caller only changes another setting.
        b.settings.basic_auth_hash = st.db.proxy_settings(&app).basic_auth_hash;
    }
    if b.settings
        .basic_auth_user
        .as_deref()
        .unwrap_or("")
        .is_empty()
    {
        b.settings.basic_auth_user = None;
        b.settings.basic_auth_hash = None;
    } else if b.settings.basic_auth_hash.is_none() {
        return bad("basic auth needs a password the first time it is enabled");
    }
    if b.settings.headers.len() > 20 {
        return bad("too many custom headers (max 20)");
    }
    for (k, v) in &b.settings.headers {
        if !safe_header(k)
            || k.len() > 100
            || k.contains(['\r', '\n', ':'])
            || v.len() > 2000
            || v.contains(['\r', '\n'])
            || HeaderName::from_bytes(k.as_bytes()).is_err()
            || HeaderValue::from_str(v).is_err()
        {
            return bad(&format!("invalid or reserved header: {k}"));
        }
    }
    if let Some(mb) = b.settings.max_body_mb {
        b.settings.max_body_mb = Some(mb.min(512));
        if b.settings.max_body_mb == Some(0) {
            b.settings.max_body_mb = None;
        }
    }
    match st.db.set_proxy_settings(&app, &b.settings) {
        Ok(_) => Json(json!({"ok": true})).into_response(),
        Err(e) => internal(e),
    }
}

pub fn project_url(st: &AppState, app: &crate::db::App) -> String {
    if let Some(d) = &app.domain {
        format!("{}://{d}", st.scheme)
    } else if let Some(base) = &st.base_domain {
        format!("{}://{}.{}", st.scheme, app.name, base)
    } else {
        st.public_url.clone()
    }
}

fn hosts_for_app(st: &AppState, app: &crate::db::App) -> Vec<String> {
    let mut set = HashSet::new();
    if let Some(d) = &app.domain {
        set.insert(d.clone());
    }
    if let Some(base) = &st.base_domain {
        set.insert(format!("{}.{}", app.name, base));
    }
    for h in st.db.domains_for(&app.name).unwrap_or_default() {
        set.insert(h);
    }
    let mut v: Vec<_> = set.into_iter().collect();
    v.sort();
    v
}

pub fn cert_info(st: &AppState, host: &str) -> Value {
    let stored = st
        .db
        .get_setting(&format!("cert:{host}"))
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    match stored {
        Some(mut v) => {
            v["host"] = json!(host);
            v
        }
        None => json!({"host": host, "status": "pending", "expires": null,
                       "note": "issued by the built-in edge when enabled"}),
    }
}

async fn list_certificates(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Response {
    let mut hosts = Vec::new();
    let mut apps = Vec::new();
    for team in st.db.teams_for(&caller.user).unwrap_or_default() {
        apps.extend(st.db.apps_for_team(&team.id).unwrap_or_default());
    }
    let mut seen = HashSet::new();
    for a in &apps {
        for h in hosts_for_app(&st, a) {
            if seen.insert(h.clone()) {
                let mut v = cert_info(&st, &h);
                v["project"] = json!(a.name);
                hosts.push(v);
            }
        }
    }
    Json(json!({"certificates": hosts})).into_response()
}

async fn renew_certificate(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(host): Path<String>,
) -> Response {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let mut owner = None;
    for a in st.db.list_apps().unwrap_or_default() {
        if hosts_for_app(&st, &a).contains(&host) {
            owner = Some(a);
            break;
        }
    }
    let Some(app) = owner else {
        return not_found();
    };
    if let Ok(a) = project_for(&st, &caller, &app.name) {
        if let Err(r) = require_write(&st, &caller, &a) {
            return r;
        }
    } else {
        return not_found();
    }
    match request_cert_renewal(&st, &host) {
        Ok(status) => Json(json!({"ok": true, "host": host, "status": status, "renewing": true}))
            .into_response(),
        Err(e) => internal(e),
    }
}

/// Shared by the token API and the dashboard Renew button: marks the host and
/// bumps the watchdog value the edge supervisor watches.
pub fn request_cert_renewal(st: &AppState, host: &str) -> anyhow::Result<String> {
    let now = crate::db::now();
    let previous = st
        .db
        .get_setting(&format!("cert:{host}"))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str::<Value>(&v).ok());
    let prev_status = previous
        .as_ref()
        .and_then(|v| v.get("status"))
        .and_then(|v| v.as_str())
        .unwrap_or("pending");
    let prev_expiry = previous
        .as_ref()
        .and_then(|v| v.get("expires"))
        .and_then(|v| v.as_i64());
    let healthy = matches!(prev_status, "issued" | "expiring")
        && prev_expiry.map(|e| e > now).unwrap_or(true);
    let status = if healthy {
        prev_status.to_string()
    } else {
        "renewing".to_string()
    };
    let value = json!({
        "status": status,
        "requested_at": now,
        "renew_requested_at": now,
        "expires": prev_expiry,
    });
    st.db.set_setting("cert_renew", &now.to_string())?;
    st.db.set_setting("cert_renew_host", host)?;
    st.db
        .set_setting(&format!("cert:{host}"), &value.to_string())?;
    Ok(status)
}

fn lookup_team(st: &AppState, name: &str) -> Result<crate::db::Team, Response> {
    st.db
        .team(name)
        .ok()
        .flatten()
        .or_else(|| st.db.team_by_slug(name).ok().flatten())
        .ok_or_else(not_found)
}

fn team_role(
    st: &AppState,
    caller: &Caller,
    team: &crate::db::Team,
) -> Result<Option<String>, Response> {
    let role = st.db.role(&team.id, &caller.user).ok().flatten();
    if crate::db::can_read(role.as_deref()) {
        Ok(role)
    } else {
        Err(not_found())
    }
}

async fn list_teams(State(st): State<AppState>, Extension(caller): Extension<Caller>) -> Response {
    let teams = st.db.teams_for(&caller.user).unwrap_or_default();
    Json(json!({"teams": teams})).into_response()
}

#[derive(Deserialize)]
struct NewTeam {
    name: String,
}

async fn create_team_api(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Json(b): Json<NewTeam>,
) -> Response {
    let name = b.name.trim();
    if name.is_empty() || name.len() > 64 {
        return bad("team name must be 1-64 characters");
    }
    let base = crate::auth::slugify(name);
    let mut slug = base.clone();
    let mut n = 1;
    while st.db.team_by_slug(&slug).ok().flatten().is_some() {
        n += 1;
        slug = format!("{base}-{n}");
        if n > 50 {
            return (StatusCode::CONFLICT, "team slug is taken\n").into_response();
        }
    }
    match st.db.create_team(name, &slug, &caller.user) {
        Ok(t) => Json(t).into_response(),
        Err(e) => internal(e),
    }
}

async fn list_members(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(team): Path<String>,
) -> Response {
    let t = match lookup_team(&st, &team) {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Err(r) = team_role(&st, &caller, &t) {
        return r;
    }
    let members: Vec<Value> = st
        .db
        .members(&t.id)
        .unwrap_or_default()
        .into_iter()
        .map(|(u, role)| json!({"id": u.id, "email": u.email, "name": u.name, "role": role}))
        .collect();
    Json(json!({"members": members})).into_response()
}

#[derive(Deserialize)]
struct RoleBody {
    role: String,
}

async fn put_member(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path((team, user)): Path<(String, String)>,
    Json(b): Json<RoleBody>,
) -> Response {
    let t = match lookup_team(&st, &team) {
        Ok(t) => t,
        Err(r) => return r,
    };
    let actor = match team_role(&st, &caller, &t) {
        Ok(r) => r,
        Err(r) => return r,
    };
    if actor.as_deref() != Some("owner") {
        return forbidden();
    }
    if !matches!(b.role.as_str(), "owner" | "admin" | "member" | "viewer") {
        return bad("invalid role");
    }
    match st.db.update_member_role_checked(&t.id, &user, &b.role) {
        Ok(crate::db::MemberChange::Changed) => {
            Json(json!({"ok": true, "role": b.role})).into_response()
        }
        Ok(crate::db::MemberChange::NotFound) => not_found(),
        Ok(crate::db::MemberChange::LastOwner) => {
            (StatusCode::CONFLICT, "cannot demote the last owner\n").into_response()
        }
        Err(e) => internal(e),
    }
}

async fn delete_member(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path((team, user)): Path<(String, String)>,
) -> Response {
    let t = match lookup_team(&st, &team) {
        Ok(t) => t,
        Err(r) => return r,
    };
    let actor = match team_role(&st, &caller, &t) {
        Ok(r) => r,
        Err(r) => return r,
    };
    if !crate::db::can_write(actor.as_deref()) {
        return forbidden();
    }
    let current = st.db.role(&t.id, &user).ok().flatten();
    match current.as_deref() {
        None => return not_found(),
        Some("owner") if actor.as_deref() != Some("owner") => return forbidden(),
        Some("admin") if actor.as_deref() != Some("owner") => return forbidden(),
        _ => {}
    }
    match st.db.remove_member_checked(&t.id, &user) {
        Ok(crate::db::MemberChange::Changed) => Json(json!({"removed": user})).into_response(),
        Ok(crate::db::MemberChange::NotFound) => not_found(),
        Ok(crate::db::MemberChange::LastOwner) => {
            (StatusCode::CONFLICT, "cannot remove the last owner\n").into_response()
        }
        Err(e) => internal(e),
    }
}

async fn list_invites(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(team): Path<String>,
) -> Response {
    let t = match lookup_team(&st, &team) {
        Ok(t) => t,
        Err(r) => return r,
    };
    let actor = match team_role(&st, &caller, &t) {
        Ok(r) => r,
        Err(r) => return r,
    };
    if !crate::db::can_write(actor.as_deref()) {
        return forbidden();
    }
    let now = crate::db::now();
    let invites: Vec<Value> = st
        .db
        .invites_for(&t.id)
        .unwrap_or_default()
        .into_iter()
        .map(
            |(token, role, created_at, expires_at, used_at, invited_by)| {
                json!({
                    "token": token,
                    "role": role,
                    "created_at": created_at,
                    "expires_at": expires_at,
                    "used": used_at.is_some(),
                    "expired": expires_at <= now,
                    "invited_by": invited_by,
                    "link": format!("{}/join/{token}", st.public_url.trim_end_matches('/')),
                })
            },
        )
        .collect();
    Json(json!({"invites": invites})).into_response()
}

async fn revoke_invite(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path((team, token)): Path<(String, String)>,
) -> Response {
    let t = match lookup_team(&st, &team) {
        Ok(t) => t,
        Err(r) => return r,
    };
    let actor = match team_role(&st, &caller, &t) {
        Ok(r) => r,
        Err(r) => return r,
    };
    if !crate::db::can_write(actor.as_deref()) {
        return forbidden();
    }
    match st.db.revoke_invite(&t.id, &token) {
        Ok(0) => not_found(),
        Ok(_) => Json(json!({"revoked": token})).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct InviteBody {
    #[serde(default = "default_role")]
    role: String,
}

fn default_role() -> String {
    "member".into()
}

async fn create_invite(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(team): Path<String>,
    Json(b): Json<InviteBody>,
) -> Response {
    let team = match st
        .db
        .team(&team)
        .ok()
        .flatten()
        .or_else(|| st.db.team_by_slug(&team).ok().flatten())
    {
        Some(t) => t,
        None => return not_found(),
    };
    let actor_role = st.db.role(&team.id, &caller.user).ok().flatten();
    if !crate::db::can_write(actor_role.as_deref()) {
        return forbidden();
    }
    let role = match b.role.as_str() {
        "owner" | "admin" | "member" | "viewer" => b.role,
        _ => "member".into(),
    };
    if !crate::db::can_grant(actor_role.as_deref(), &role) {
        return (StatusCode::FORBIDDEN, "you cannot grant that role\n").into_response();
    }
    match st
        .db
        .create_invite(&team.id, &role, 7 * 86400, Some(&caller.user))
    {
        Ok(token) => Json(json!({
            "token": token,
            "link": format!("{}/join/{token}", st.public_url.trim_end_matches('/')),
            "role": role,
        }))
        .into_response(),
        Err(e) => internal(e),
    }
}

async fn git_init(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
) -> Response {
    if !st.rate.allow(&format!("deploy:{}", caller.user), 20, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "deploy rate limit exceeded\n",
        )
            .into_response();
    }
    if !valid_name(&app) {
        return bad("invalid app name");
    }
    let existing = st.db.get_app(&app).ok().flatten();
    if let Some(a) = &existing {
        if let Err(r) = require_write(&st, &caller, a) {
            return r;
        }
    } else {
        let team_id = match team_for_caller(&st, &caller, None) {
            Ok(t) => t,
            Err(r) => return r,
        };
        let row = crate::db::App {
            name: app.clone(),
            team_id: Some(team_id),
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
            updated_at: crate::db::now(),
        };
        match st.db.create_app_if_absent(&row) {
            Ok(true) => {}
            Ok(false) => match st.db.get_app(&app) {
                Ok(Some(a)) => {
                    if let Err(r) = require_write(&st, &caller, &a) {
                        return r;
                    }
                }
                _ => return internal(anyhow::anyhow!("project disappeared")),
            },
            Err(e) => return internal(e),
        }
    }
    match init_bare_repo(&st, &app) {
        Ok((path, hook)) => {
            let _ = st.db.set_repo(&app, &path);
            let secret_key = format!("webhook_secret:{app}");
            let secret = st
                .db
                .get_setting(&secret_key)
                .ok()
                .flatten()
                .unwrap_or_else(|| {
                    let s = format!(
                        "{}{}",
                        uuid::Uuid::new_v4().simple(),
                        uuid::Uuid::new_v4().simple()
                    );
                    let _ = st.db.set_setting(&secret_key, &s);
                    let _ = st.db.set_setting(&format!("webhook_branch:{app}"), "main");
                    s
                });
            Json(json!({
                "app": app,
                "path": path,
                "remote": format!("ssh://<user>@<host>{path}"),
                "hint": format!("git remote add dok ssh://<user>@<host>{path} && git push dok main"),
                "hook": hook,
                "webhook_secret": secret,
                "branch": "main",
                "webhook_url": format!("{}/api/v1/hooks/github/{}", st.public_url.trim_end_matches('/'), app),
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
            .args(["init", "--bare", "--quiet", "-b", "main"])
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
        "#!/bin/sh\nunset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_QUARANTINE_PATH\nref=\"\"\nwhile read -r _old _new name; do ref=\"$name\"; done\nexec '{}' deploy-git '{}' --data '{}' --ref \"$ref\" >> '{}' 2>&1\n",
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
    Ok((
        dir.to_string_lossy().to_string(),
        hook_path.to_string_lossy().to_string(),
    ))
}

async fn redeploy(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
) -> Response {
    if !st.rate.allow(&format!("deploy:{}", caller.user), 20, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "deploy rate limit exceeded\n",
        )
            .into_response();
    }
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = require_write(&st, &caller, &a) {
        return r;
    }
    match deploy_from_repo(&st, &app).await {
        Ok(out) => Json(out).into_response(),
        Err(e) => internal(e),
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
    req: Request,
) -> Response {
    // Webhooks are a public endpoint: never buffer more than a small payload.
    let body = match axum::body::to_bytes(req.into_body(), MAX_WEBHOOK).await {
        Ok(b) => b,
        Err(_) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "webhook payload too large\n").into_response()
        }
    };
    if !st.rate.allow(&format!("hook:{app}"), 30, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "too many webhook deliveries\n",
        )
            .into_response();
    }
    let project_secret = st
        .db
        .get_setting(&format!("webhook_secret:{app}"))
        .ok()
        .flatten();
    let global = std::env::var("DOK_HOOK_SECRET")
        .ok()
        .filter(|s| !s.is_empty());
    match project_secret.as_deref() {
        Some(secret) => {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            let sig = headers
                .get("x-hub-signature-256")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let Some(hex) = sig.strip_prefix("sha256=") else {
                return (StatusCode::UNAUTHORIZED, "missing X-Hub-Signature-256\n").into_response();
            };
            let Some(expected) = decode_hex(hex) else {
                return (StatusCode::UNAUTHORIZED, "bad signature\n").into_response();
            };
            let mut mac = match Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
                Ok(m) => m,
                Err(_) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "bad secret\n").into_response()
                }
            };
            mac.update(&body);
            if mac.verify_slice(&expected).is_err() {
                return (StatusCode::UNAUTHORIZED, "bad signature\n").into_response();
            }
        }
        None => {
            // Legacy global secret path, kept for existing installs.
            let given = q
                .secret
                .or_else(|| {
                    headers
                        .get("x-dok-secret")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string)
                })
                .unwrap_or_default();
            match global {
                Some(exp) if constant_eq(&exp, &given) => {}
                _ => {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "webhook secret not configured for this project\n",
                    )
                        .into_response()
                }
            }
        }
    }

    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    let branch_setting = st
        .db
        .get_setting(&format!("webhook_branch:{app}"))
        .ok()
        .flatten()
        .filter(|b| !b.is_empty());
    let r#ref = payload["ref"].as_str().unwrap_or("");

    let pushed_branch = r#ref.strip_prefix("refs/heads/").filter(|b| !b.is_empty());
    if let Some(branch) = pushed_branch {
        if !valid_branch(branch) || branch.starts_with('-') {
            return bad("invalid branch name in webhook");
        }
    }
    let deploy_branch = pushed_branch
        .map(str::to_string)
        .or_else(|| branch_setting.clone());
    let env = git_deploy_env(branch_setting.as_deref(), pushed_branch);
    let out = match deploy_from_repo_branch_env(&st, &app, deploy_branch.as_deref(), env).await {
        Ok(out) => out,
        Err(e) => return internal(e),
    };
    let sha = payload["after"]
        .as_str()
        .or_else(|| payload["head_commit"]["id"].as_str())
        .unwrap_or("");
    if !sha.is_empty() {
        let message = payload["head_commit"]["message"]
            .as_str()
            .unwrap_or("webhook deploy");
        let author = payload["head_commit"]["author"]["name"]
            .as_str()
            .unwrap_or("webhook");
        let branch = r#ref.strip_prefix("refs/heads/").unwrap_or("");
        let _ = st
            .db
            .set_deploy_commit(&out.deploy_id, sha, message, author, branch);
    }
    Json(json!({"deploy": out, "branch": branch_setting})).into_response()
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
    }
    Some(out)
}

fn constant_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub async fn deploy_from_repo(st: &AppState, app: &str) -> anyhow::Result<DeployOutcome> {
    let branch = st
        .db
        .get_setting(&format!("webhook_branch:{app}"))
        .ok()
        .flatten()
        .filter(|b| !b.is_empty());
    deploy_from_repo_branch(st, app, branch.as_deref()).await
}

pub async fn deploy_from_repo_branch(
    st: &AppState,
    app: &str,
    branch: Option<&str>,
) -> anyhow::Result<DeployOutcome> {
    deploy_from_repo_branch_env(st, app, branch, "production").await
}

/// A push to the configured production branch deploys production; any other
/// branch pushed to a project with a configured branch becomes a preview.
pub fn git_deploy_env(
    production_branch: Option<&str>,
    pushed_branch: Option<&str>,
) -> &'static str {
    match (production_branch, pushed_branch) {
        (Some(prod), Some(pushed)) if prod != pushed => "preview",
        _ => "production",
    }
}

pub async fn deploy_from_repo_branch_env(
    st: &AppState,
    app: &str,
    branch: Option<&str>,
    env: &str,
) -> anyhow::Result<DeployOutcome> {
    let row = st
        .db
        .get_app(app)?
        .ok_or_else(|| anyhow::anyhow!("no such app"))?;
    let repo = row
        .repo
        .ok_or_else(|| anyhow::anyhow!("app has no repo; use `dok git {app} <url>`"))?;

    let local_repo = std::path::Path::new(&repo).is_absolute()
        && std::path::Path::new(&repo).starts_with(st.data.join("repos"));
    if !local_repo && !valid_repo_url(&repo) {
        anyhow::bail!(
            "repo must be a public https://, ssh:// or git@host:path (private/loopback hosts are refused)"
        );
    }
    let file_policy = if local_repo {
        "protocol.file.allow=always"
    } else {
        "protocol.file.allow=never"
    };
    let allow = if local_repo {
        "https:ssh:file"
    } else {
        "https:ssh"
    };
    let resolve_specs = if local_repo {
        Vec::new()
    } else {
        reject_private_repo_dns(&repo).await?
    };
    let git = |args: &[&str]| {
        let mut c = tokio::process::Command::new("git");
        c.args(["-c", "protocol.ext.allow=never", "-c", file_policy]);
        for spec in &resolve_specs {
            c.arg("-c").arg(format!("http.curloptResolve={spec}"));
        }
        c.args(args)
            .env("GIT_ALLOW_PROTOCOL", allow)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE");
        c
    };

    if !local_repo {
        let probe = git(&["ls-remote", "--heads", &repo]).output().await?;
        if !probe.status.success() {
            anyhow::bail!(
                "cannot reach repo: {}",
                String::from_utf8_lossy(&probe.stderr).trim()
            );
        }
    }

    let dir = deploy::new_build_dir(st, app)?;
    let mut clone = git(&["clone", "--depth", "1"]);
    if let Some(branch) = branch {
        clone.args(["--branch", branch]);
    }
    let out = clone.arg(&repo).arg(&dir).output().await?;
    if !out.status.success() {
        anyhow::bail!("git clone failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let head = tokio::process::Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(&dir)
        .env("GIT_ALLOW_PROTOCOL", allow)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .await?;
    if !head.status.success() {
        anyhow::bail!("repo has no commits yet; push a branch first");
    }
    let sha = String::from_utf8_lossy(&head.stdout).trim().to_string();
    let meta = tokio::process::Command::new("git")
        .args(["log", "-1", "--format=%s%x00%an"])
        .current_dir(&dir)
        .env("GIT_ALLOW_PROTOCOL", allow)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .await
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let (message, commit_author) = meta.split_once('\0').unwrap_or((meta.as_str(), ""));
    let branch = tokio::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(&dir)
        .env("GIT_ALLOW_PROTOCOL", allow)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .await
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "main".into());
    let out = deploy::deploy_dir(st, app, &dir, None, None, env, None).await?;
    if !sha.is_empty() {
        let _ = st
            .db
            .set_deploy_commit(&out.deploy_id, &sha, message, commit_author, &branch);
    }
    Ok(out)
}

/// Parse and validate the dashboard's "Name: value" header editor.
pub(crate) fn parse_proxy_headers(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| format!("Missing ':' in header line: {line}"))?;
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() || value.is_empty() {
            return Err(format!("Header line needs a name and value: {line}"));
        }
        if !safe_header(name) {
            return Err(format!(
                "Header {name} is managed by dok and cannot be overridden"
            ));
        }
        HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("Invalid header name: {name}"))?;
        HeaderValue::from_str(value).map_err(|_| format!("Invalid value for header {name}"))?;
        out.push((name.to_string(), value.to_string()));
        if out.len() > 20 {
            return Err("Too many custom headers (max 20)".into());
        }
    }
    Ok(out)
}

pub(crate) fn safe_header(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    !n.is_empty()
        && !matches!(
            n.as_str(),
            "content-length"
                | "transfer-encoding"
                | "content-encoding"
                | "connection"
                | "set-cookie"
                | "location"
                | "keep-alive"
                | "trailer"
                | "upgrade"
                | "te"
                | "host"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "date"
                | "server"
        )
}

pub(crate) fn valid_repo_url(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.len() > 500 || s.starts_with('-') || s.contains(char::is_whitespace) {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    if lower.contains("ext::")
        || lower.contains("upload-pack")
        || lower.contains("file://")
        || lower.contains("--")
    {
        return false;
    }
    if !(lower.starts_with("https://") || lower.starts_with("ssh://") || lower.starts_with("git@"))
    {
        return false;
    }
    let Some(host) = repo_host_of(&lower) else {
        return false;
    };
    // Never store credentials embedded in the URL (they would be shown back
    // in the dashboard and API).
    if let Some((_scheme, rest)) = lower.split_once("://") {
        if rest.contains('@') && rest.split('@').next().unwrap_or("").contains(':') {
            return false;
        }
    }
    if !(host.contains('.') || host == "localhost") {
        return false;
    }
    repo_host_allowed(&host)
}

pub(crate) fn repo_host_of(s: &str) -> Option<String> {
    let lower = s.trim().to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("ssh://"))
        .or_else(|| lower.strip_prefix("git@"))
        .unwrap_or(&lower);
    let hostport = rest.rsplit('@').next().unwrap_or(rest);
    let host = if let Some(bracketed) = hostport.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or("")
    } else {
        hostport.split(['/', ':']).next().unwrap_or("")
    };
    let host = host.trim_end_matches('.');
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn ip_is_public(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || v4.octets()[0] == 0)
        }
        std::net::IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                || v6.is_multicast())
        }
    }
}

/// Reject obvious SSRF targets: bare private/loopback/link-local addresses and
/// local/metadata names. An explicit allowlist opts an internal host back in.
fn repo_host_allowed(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    // Operators with an internal git server can opt in explicitly:
    // DOK_REPO_ALLOW_HOSTS=git.internal,10.0.0.5
    if let Ok(list) = std::env::var("DOK_REPO_ALLOW_HOSTS") {
        if !list.trim().is_empty() {
            return list
                .split(',')
                .map(|h| h.trim().trim_start_matches('[').trim_end_matches(']'))
                .any(|h| h.eq_ignore_ascii_case(host));
        }
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return ip_is_public(ip);
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    !(host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host == "metadata.google.internal"
        || host == "instance-data")
}

/// Resolve a public repo host before git connects and return curl resolve
/// specs so git/curl uses the checked addresses instead of resolving again
/// (which would allow DNS-rebinding into the metadata service).
async fn reject_private_repo_dns(repo: &str) -> anyhow::Result<Vec<String>> {
    let Some(host) = repo_host_of(repo) else {
        return Ok(Vec::new());
    };
    if let Ok(list) = std::env::var("DOK_REPO_ALLOW_HOSTS") {
        if list
            .split(',')
            .map(|h| h.trim())
            .any(|h| h.eq_ignore_ascii_case(&host))
        {
            return Ok(Vec::new());
        }
    }
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(Vec::new());
    }
    let https = repo
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("https://");
    let port = if https { 443 } else { 22 };
    let Ok(addrs) = tokio::net::lookup_host((host.as_str(), port)).await else {
        return Ok(Vec::new());
    };
    let mut ips = Vec::new();
    for addr in addrs {
        if !ip_is_public(addr.ip()) {
            anyhow::bail!("repo host {host} resolves to a private address");
        }
        ips.push(match addr.ip() {
            std::net::IpAddr::V4(v4) => v4.to_string(),
            std::net::IpAddr::V6(v6) => format!("[{v6}]"),
        });
    }
    if !https || ips.is_empty() {
        // Git's HTTPS transport uses libcurl, where this pin is enforced.
        // SSH host keys remain the authentication boundary.
        return Ok(Vec::new());
    }
    Ok(vec![format!("{host}:{port}:{}", ips.join(","))])
}

pub(crate) fn valid_branch(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'))
        && !s.starts_with('/')
        && !s.ends_with('/')
        && !s.contains("..")
}

/// Same rule as the dashboard: shell-safe variable names only.
pub(crate) fn valid_env_key(k: &str) -> bool {
    let mut chars = k.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(crate) fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

pub(crate) fn valid_host(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && s.contains('.')
}

pub fn hash(t: &str) -> String {
    hash_token(t)
}

#[cfg(test)]
mod tests {
    use super::{api_routes, constant_eq, hash, normalize_env, valid_host, valid_name};

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

    #[test]
    fn secrets_compare() {
        assert!(constant_eq("abc", "abc"));
        assert!(!constant_eq("abc", "abd"));
        assert!(!constant_eq("abc", "ab"));
    }

    #[test]
    fn repo_urls() {
        assert!(super::valid_repo_url("https://github.com/a/b.git"));
        assert!(super::valid_repo_url("ssh://git@example.com/a/b.git"));
        assert!(super::valid_repo_url("git@github.com:a/b.git"));
        assert!(!super::valid_repo_url("file:///etc/passwd"));
        assert!(!super::valid_repo_url("ext::sh -c id"));
        assert!(!super::valid_repo_url("--upload-pack=id"));
        assert!(!super::valid_repo_url("http://example.com/a.git"));
        assert!(!super::valid_repo_url("/tmp/repo"));
        // SSRF targets
        assert!(!super::valid_repo_url("https://127.0.0.1/a.git"));
        assert!(!super::valid_repo_url(
            "https://169.254.169.254/latest/meta-data"
        ));
        assert!(!super::valid_repo_url("https://10.0.0.5/a.git"));
        assert!(!super::valid_repo_url("https://192.168.1.10/a.git"));
        assert!(!super::valid_repo_url("ssh://git@[::1]/a.git"));
        assert!(!super::valid_repo_url("ssh://git@10.0.0.5/a.git"));
        assert!(!super::valid_repo_url("https://user:pw@192.168.0.2/a.git"));
        assert!(!super::valid_repo_url("git@localhost:a/b.git"));
        assert!(!super::valid_repo_url(
            "https://metadata.google.internal/a.git"
        ));
        assert!(!super::valid_repo_url("https://user:pw@example.com/a.git"));
        assert!(super::valid_repo_url("https://user@example.com/a.git"));
        assert!(super::valid_repo_url("ssh://git@8.8.8.8/a.git"));
    }

    #[test]
    fn deploy_envs() {
        assert_eq!(normalize_env("production"), Some("production"));
        assert_eq!(normalize_env("preview"), Some("preview"));
        assert_eq!(normalize_env(""), Some("production"));
        assert_eq!(normalize_env("staging"), None);
    }

    #[test]
    fn git_branches_map_to_envs() {
        assert_eq!(
            super::git_deploy_env(Some("main"), Some("main")),
            "production"
        );
        assert_eq!(super::git_deploy_env(Some("main"), Some("dev")), "preview");
        assert_eq!(super::git_deploy_env(None, Some("dev")), "production");
        assert_eq!(super::git_deploy_env(Some("main"), None), "production");
    }

    #[test]
    fn env_body_accepts_secret_objects_and_lists() {
        let body = serde_json::json!({
            "vars": {"PLAIN": "one", "SECRET": {"value": "two", "secret": true}, "KEEP": "three"},
            "secrets": ["KEEP"],
            "replace": true
        });
        let (vars, replace) = super::parse_env_body(body).unwrap();
        assert!(replace);
        assert_eq!(vars["PLAIN"], ("one".to_string(), None));
        assert_eq!(vars["SECRET"], ("two".to_string(), Some(true)));
        assert_eq!(vars["KEEP"], ("three".to_string(), Some(true)));
        assert!(super::valid_env_key("DATABASE_URL"));
        assert!(!super::valid_env_key("1BAD"));
        assert!(!super::valid_env_key("BAD\nKEY"));
    }

    #[test]
    fn upload_limits() {
        assert!(!super::over_upload_limit(200 * 1024 * 1024));
        assert!(super::over_upload_limit(200 * 1024 * 1024 + 1));
    }

    #[test]
    fn header_denylist() {
        assert!(super::safe_header("X-Frame-Options"));
        assert!(super::safe_header("Strict-Transport-Security"));
        assert!(!super::safe_header("Content-Length"));
        assert!(!super::safe_header("Set-Cookie"));
        assert!(!super::safe_header("Location"));
        assert!(!super::safe_header("Transfer-Encoding"));
    }

    // ---------- integration: authz across every route ----------

    use crate::db::{App, Db};
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use axum::response::Response;
    use serde_json::{json, Value};
    use std::path::PathBuf;
    use tower::ServiceExt;

    fn state() -> (AppState, PathBuf) {
        let path = std::env::temp_dir().join(format!("dok-api-{}.db", uuid::Uuid::new_v4()));
        let db = Db::open(&path).unwrap();
        let data = std::env::temp_dir().join(format!("dok-api-data-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data).unwrap();
        (
            AppState::new(db, data, Some("apps.test".into()), "http://dok.test".into()),
            path,
        )
    }

    fn seed(st: &AppState, name: &str, role: &str) -> (String, String, String) {
        let u = st
            .db
            .create_user(&format!("{name}@example.com"), name)
            .unwrap();
        let t = st
            .db
            .create_team(name, &format!("{name}-team"), &u.id)
            .unwrap();
        if role != "owner" {
            st.db.add_member(&t.id, &u.id, role).unwrap();
        }
        let token = format!("token-{name}");
        st.db.add_token(&hash(&token), name, Some(&u.id)).unwrap();
        (u.id, t.id, token)
    }

    fn project(st: &AppState, name: &str, team: &str) {
        st.db
            .upsert_app(&App {
                name: name.into(),
                team_id: Some(team.into()),
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
                updated_at: crate::db::now(),
            })
            .unwrap();
    }

    async fn call(
        st: &AppState,
        token: Option<&str>,
        method: Method,
        uri: &str,
        body: Option<&str>,
    ) -> Response {
        let mut b = Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        if body.is_some() {
            b = b.header("content-type", "application/json");
        }
        let req = b
            .body(
                body.map(|s| Body::from(s.to_string()))
                    .unwrap_or_else(Body::empty),
            )
            .unwrap();
        api_routes(st.clone()).oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn control_plane_is_host_gated() {
        let (st, path) = state();
        let foreign = Request::builder()
            .uri("/dashboard")
            .header("host", "foreign.test")
            .body(Body::empty())
            .unwrap();
        let r = super::dispatch(axum::extract::State(st.clone()), foreign).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        assert!(r.headers().get("x-request-id").is_some());
        let body = axum::body::to_bytes(r.into_body(), 256 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(!text.contains("Projects"));
        assert!(!text.contains("Sign in"));

        // Control host with no passkey origin explains the flag instead of 404.
        let control = Request::builder()
            .uri("/")
            .header("host", "dok.test")
            .body(Body::empty())
            .unwrap();
        let r = super::dispatch(axum::extract::State(st.clone()), control).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(r.into_body(), 256 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("--dashboard-origin"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn authz_across_routes() {
        let (st, path) = state();
        let (_a, ta, ta_tok) = seed(&st, "alice", "owner");
        let (_b, tb, tb_tok) = seed(&st, "bob", "owner");
        let (_c, _tc, tc_tok) = seed(&st, "carol", "member");
        st.db.add_member(&ta, &_c, "member").unwrap();
        project(&st, "app-a", &ta);
        project(&st, "app-b", &tb);

        // No token and unbound token fail closed.
        assert_eq!(
            call(&st, None, Method::GET, "/api/v1/projects", None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        st.db.add_token(&hash("dead-token"), "dead", None).unwrap();
        assert_eq!(
            call(
                &st,
                Some("dead-token"),
                Method::GET,
                "/api/v1/projects",
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );

        // Cross-team routes are invisible (404) for read and write.
        for (method, uri, body) in [
            (Method::GET, "/api/v1/projects/app-b", None),
            (Method::DELETE, "/api/v1/projects/app-b", None),
            (
                Method::PUT,
                "/api/v1/projects/app-b/env/production",
                Some("{}"),
            ),
            (
                Method::POST,
                "/api/v1/projects/app-b/domains",
                Some("{\"domain\":\"x.test\"}"),
            ),
            (Method::GET, "/api/v1/projects/app-b/proxy", None),
            (Method::PUT, "/api/v1/projects/app-b/proxy", Some("{}")),
            (Method::GET, "/api/v1/projects/app-b/metrics", None),
            (Method::GET, "/api/v1/projects/app-b/access-log", None),
            (Method::GET, "/api/v1/projects/app-b/logs", None),
            (Method::POST, "/api/v1/projects/app-b/promote/x", None),
            (Method::POST, "/api/v1/projects/app-b/rollback/x", None),
        ] {
            let r = call(&st, Some(&ta_tok), method.clone(), uri, body).await;
            assert_eq!(r.status(), StatusCode::NOT_FOUND, "{method} {uri}");
        }

        // Members may read their team, never write.
        assert_eq!(
            call(
                &st,
                Some(&tc_tok),
                Method::GET,
                "/api/v1/projects/app-a",
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        for (method, uri, body) in [
            (Method::DELETE, "/api/v1/projects/app-a", None),
            (
                Method::PUT,
                "/api/v1/projects/app-a/env/production",
                Some("{}"),
            ),
            (
                Method::POST,
                "/api/v1/projects/app-a/domains",
                Some("{\"domain\":\"x.test\"}"),
            ),
            (Method::PUT, "/api/v1/projects/app-a/proxy", Some("{}")),
            (Method::POST, "/api/v1/projects/app-a/redeploy", None),
        ] {
            let r = call(&st, Some(&tc_tok), method.clone(), uri, body).await;
            assert_eq!(r.status(), StatusCode::FORBIDDEN, "{method} {uri}");
        }

        // Owner still sees their own project.
        assert_eq!(
            call(
                &st,
                Some(&tb_tok),
                Method::GET,
                "/api/v1/projects/app-b",
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        let _ = std::fs::remove_file(path);
    }

    async fn body_json(r: Response) -> Value {
        let b = axum::body::to_bytes(r.into_body(), 256 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&b).unwrap_or_else(|_| json!({}))
    }

    #[tokio::test]
    async fn team_admin_endpoints() {
        let (st, path) = state();
        let (_u, _t, owner) = seed(&st, "olga", "owner");
        let (_b, _bt, bob) = seed(&st, "bob", "member");
        let r = call(
            &st,
            Some(&owner),
            Method::POST,
            "/api/v1/teams",
            Some("{\"name\":\"Second Team\"}"),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let team = body_json(r).await["id"].as_str().unwrap().to_string();

        let r = call(
            &st,
            Some(&owner),
            Method::GET,
            &format!("/api/v1/teams/{team}/members"),
            None,
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert!(body_json(r).await["members"].as_array().unwrap().len() == 1);

        // Non-members cannot see the team.
        assert_eq!(
            call(
                &st,
                Some(&bob),
                Method::GET,
                &format!("/api/v1/teams/{team}/members"),
                None
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );

        let alice = st.db.members(&team).unwrap()[0].0.id.clone();
        assert_eq!(
            call(
                &st,
                Some(&owner),
                Method::PUT,
                &format!("/api/v1/teams/{team}/members/{alice}"),
                Some("{\"role\":\"member\"}")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );

        let r = call(
            &st,
            Some(&owner),
            Method::POST,
            &format!("/api/v1/teams/{team}/invites"),
            Some("{\"role\":\"member\"}"),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let token = body_json(r).await["token"].as_str().unwrap().to_string();
        let r = call(
            &st,
            Some(&owner),
            Method::GET,
            &format!("/api/v1/teams/{team}/invites"),
            None,
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(
            call(
                &st,
                Some(&owner),
                Method::DELETE,
                &format!("/api/v1/teams/{team}/invites/{token}"),
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(
                &st,
                Some(&owner),
                Method::DELETE,
                &format!("/api/v1/teams/{team}/invites/{token}"),
                None
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn primary_domain_is_listed_and_removed() {
        let (st, path) = state();
        let (_u, team, token) = seed(&st, "dora", "owner");
        project(&st, "domapp", &team);
        let r = call(
            &st,
            Some(&token),
            Method::POST,
            "/api/v1/apps/domapp/domain",
            Some("{\"domain\":\"primary.test\"}"),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let body = body_json(
            call(
                &st,
                Some(&token),
                Method::GET,
                "/api/v1/projects/domapp/domains",
                None,
            )
            .await,
        )
        .await;
        assert_eq!(body["primary"], "primary.test");
        assert!(body["domains"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d == "primary.test"));

        let r = call(
            &st,
            Some(&token),
            Method::DELETE,
            "/api/v1/projects/domapp/domains/primary.test",
            None,
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let body = body_json(
            call(
                &st,
                Some(&token),
                Method::GET,
                "/api/v1/projects/domapp/domains",
                None,
            )
            .await,
        )
        .await;
        assert!(body["primary"].is_null());
        assert!(!body["domains"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d == "primary.test"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn certificate_renewal_marks_watchdog() {
        let (st, path) = state();
        let _ = seed(&st, "cert", "owner");
        let status = super::request_cert_renewal(&st, "cert.apps.test").unwrap();
        assert_eq!(status, "renewing");
        let now: i64 = st
            .db
            .get_setting("cert_renew")
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        assert!(now > 0);
        let value: Value =
            serde_json::from_str(&st.db.get_setting("cert:cert.apps.test").unwrap().unwrap())
                .unwrap();
        assert_eq!(value["status"], "renewing");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn webhook_requires_signature() {
        let (st, path) = state();
        let (_u, team, token) = seed(&st, "hook", "owner");
        project(&st, "hookapp", &team);
        st.db
            .set_setting("webhook_secret:hookapp", "0123456789abcdef")
            .unwrap();
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/hooks/github/hookapp")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let r = api_routes(st.clone()).oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        let _ = token;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn proxy_update_preserves_basic_auth_hash() {
        let (st, path) = state();
        let (_u, team, owner) = seed(&st, "proxy-owner", "owner");
        project(&st, "proxied", &team);
        let r = call(
            &st,
            Some(&owner),
            Method::PUT,
            "/api/v1/projects/proxied/proxy",
            Some(r#"{"basic_auth_user":"u","basic_auth_password":"pw"}"#),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let first = st.db.proxy_settings("proxied").basic_auth_hash.clone();
        assert!(first.is_some());
        let r = call(
            &st,
            Some(&owner),
            Method::PUT,
            "/api/v1/projects/proxied/proxy",
            Some(r#"{"basic_auth_user":"u","headers":[["X-Test","1"]],"compression":true}"#),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let settings = st.db.proxy_settings("proxied");
        assert_eq!(settings.basic_auth_hash, first);
        assert!(settings.compression);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn env_values_are_role_scoped_and_replace_works() {
        let (st, path) = state();
        let (_u, team, owner) = seed(&st, "erin", "owner");
        let (_v, _t, viewer) = seed(&st, "vic", "viewer");
        st.db.add_member(&team, &_v, "viewer").unwrap();
        project(&st, "envapp", &team);

        let r = call(
            &st,
            Some(&owner),
            Method::PUT,
            "/api/v1/projects/envapp/env/production",
            Some("{\"A\":\"1\",\"B\":\"2\"}"),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);

        let body = axum::body::to_bytes(
            call(
                &st,
                Some(&viewer),
                Method::GET,
                "/api/v1/projects/envapp/env/production",
                None,
            )
            .await
            .into_body(),
            64 * 1024,
        )
        .await
        .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("\"***\""), "{text}");
        assert!(!text.contains("\"1\""), "{text}");

        let r = call(
            &st,
            Some(&owner),
            Method::PUT,
            "/api/v1/projects/envapp/env/production",
            Some("{\"SECRET\":{\"value\":\"s3cret\",\"secret\":true}}"),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let body = axum::body::to_bytes(
            call(
                &st,
                Some(&owner),
                Method::GET,
                "/api/v1/projects/envapp/env/production",
                None,
            )
            .await
            .into_body(),
            64 * 1024,
        )
        .await
        .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.contains("s3cret"), "secret leaked to owner: {text}");
        assert!(text.contains("\"***\""), "{text}");

        let r = call(
            &st,
            Some(&owner),
            Method::PUT,
            "/api/v1/projects/envapp/env/production",
            Some("{\"replace\":true,\"vars\":{\"B\":\"3\"}}"),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let keys = st.db.env("envapp", "production").unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].0, "B");

        // Removing a domain that is not owned by this project is 404.
        let r = call(
            &st,
            Some(&owner),
            Method::DELETE,
            "/api/v1/projects/envapp/domains/nope.test",
            None,
        )
        .await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn dashboard_host_cannot_be_claimed() {
        let (st, path) = state();
        let (_u, team, token) = seed(&st, "dana", "owner");
        project(&st, "hostapp", &team);

        for host in ["dok.test", "apps.test", "x.apps.test", "hostapp.apps.test"] {
            let body = format!("{{\"domain\":\"{host}\"}}");
            let r = call(
                &st,
                Some(&token),
                Method::POST,
                "/api/v1/projects/hostapp/domains",
                Some(&body),
            )
            .await;
            assert_eq!(r.status(), StatusCode::CONFLICT, "{host}");
        }

        let r = call(
            &st,
            Some(&token),
            Method::POST,
            "/api/v1/projects/hostapp/domains",
            Some("{\"domain\":\"custom.test\"}"),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);

        // A second project cannot steal an owned domain.
        project(&st, "other", &team);
        let r = call(
            &st,
            Some(&token),
            Method::POST,
            "/api/v1/projects/other/domains",
            Some("{\"domain\":\"custom.test\"}"),
        )
        .await;
        assert_eq!(r.status(), StatusCode::CONFLICT);
        let _ = std::fs::remove_file(path);
    }
}
