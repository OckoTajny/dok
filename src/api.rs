use crate::deploy::{self, DeployOutcome};
use crate::proxy;
use crate::state::AppState;
use axum::extract::{DefaultBodyLimit, Extension, Path, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
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

const MAX_UPLOAD: usize = 512 * 1024 * 1024;

#[derive(Clone)]
pub struct Caller {
    /// None means a legacy token created before the first user existed; it has
    /// full access (the live single-tenant install keeps working).
    pub user: Option<String>,
}

pub fn router(st: AppState) -> Router {
    Router::new().fallback(dispatch).with_state(st)
}

async fn dispatch(State(st): State<AppState>, req: Request) -> Response {
    // A routed app always wins: its own /api/... paths must reach the app.
    if let Some(app) = proxy::app_for(&st, &req) {
        return proxy::serve_app(st, req, app).await;
    }

    let path = req.uri().path().to_string();
    if path == "/api" || path.starts_with("/api/") {
        return match api_routes(st).oneshot(req).await {
            Ok(r) => r,
            Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal\n").into_response(),
        };
    }

    if let Some(r) = crate::auth::handle(&st, req).await {
        return r;
    }
    proxy::landing(&st)
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
        .route("/api/v1/projects/{app}/access-log", get(access_log))
        .route("/api/v1/projects/{app}/proxy", get(get_proxy).put(put_proxy))
        .route("/api/v1/projects/{app}/logs", get(logs))
        .route("/api/v1/projects/{app}/logs/stream", get(logs_stream))
        // Old app API, kept as aliases.
        .route("/api/v1/apps", get(list_projects))
        .route("/api/v1/apps/{app}", get(get_project).delete(delete_project))
        .route("/api/v1/apps/{app}/deploy", post(deploy_upload_named))
        .route("/api/v1/apps/{app}/logs", get(logs))
        .route("/api/v1/apps/{app}/logs/stream", get(logs_stream))
        .route("/api/v1/apps/{app}/deploys", get(list_deployments))
        .route("/api/v1/apps/{app}/deployments", get(list_deployments))
        .route("/api/v1/apps/{app}/rollback/{deploy}", post(rollback))
        .route("/api/v1/apps/{app}/promote/{deploy}", post(promote))
        .route("/api/v1/apps/{app}/env", put(put_env_root).get(get_env_root))
        .route("/api/v1/apps/{app}/domain", post(set_domain))
        .route("/api/v1/apps/{app}/repo", post(set_repo))
        .route("/api/v1/apps/{app}/git-init", post(git_init))
        .route("/api/v1/apps/{app}/redeploy", post(redeploy))
        // Certificates, teams, and the spec upload endpoint.
        .route("/api/v1/certificates", get(list_certificates))
        .route(
            "/api/v1/certificates/{host}/renew",
            post(renew_certificate),
        )
        .route("/api/v1/teams/{team}/invites", post(create_invite))
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
        Ok(Some(user)) => {
            req.extensions_mut().insert(Caller { user });
            next.run(req).await
        }
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

fn internal(e: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": e.to_string()})),
    )
        .into_response()
}

/// Read access: members and viewers may read; non-members get 404 so project
/// names outside the caller's teams are not disclosed.
fn project_for(st: &AppState, caller: &Caller, name: &str) -> Result<crate::db::App, Response> {
    let app = st.db.get_app(name).ok().flatten().ok_or_else(not_found)?;
    match (&caller.user, &app.team_id) {
        (None, _) => Ok(app),
        (Some(_), None) => Ok(app), // legacy project without a team
        (Some(uid), Some(tid)) => {
            let role = st.db.role(tid, uid).ok().flatten();
            if crate::db::can_read(role.as_deref()) {
                Ok(app)
            } else {
                Err(not_found())
            }
        }
    }
}

fn require_write(st: &AppState, caller: &Caller, app: &crate::db::App) -> Result<(), Response> {
    let Some(uid) = &caller.user else {
        return Ok(());
    };
    match &app.team_id {
        None => Ok(()),
        Some(tid) => {
            let role = st.db.role(tid, uid).ok().flatten();
            if crate::db::can_write(role.as_deref()) {
                Ok(())
            } else if crate::db::can_read(role.as_deref()) {
                Err(forbidden())
            } else {
                Err(not_found())
            }
        }
    }
}

fn team_for_caller(
    st: &AppState,
    caller: &Caller,
    wanted: Option<&str>,
) -> Result<Option<String>, Response> {
    let Some(uid) = &caller.user else {
        return Ok(None);
    };
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
        return Ok(Some(team.id.clone()));
    }
    Ok(teams
        .into_iter()
        .find(|t| {
            let role = st.db.role(&t.id, uid).ok().flatten();
            crate::db::can_write(role.as_deref())
        })
        .map(|t| t.id))
}

fn author_of(st: &AppState, caller: &Caller) -> Option<String> {
    caller
        .user
        .as_ref()
        .and_then(|uid| st.db.user(uid).ok().flatten())
        .map(|u| u.email)
}

async fn list_projects(State(st): State<AppState>, Extension(caller): Extension<Caller>) -> Response {
    let apps = match &caller.user {
        None => st.db.list_apps().unwrap_or_default(),
        Some(uid) => {
            let mut out = Vec::new();
            for team in st.db.teams_for(uid).unwrap_or_default() {
                out.extend(st.db.apps_for_team(&team.id).unwrap_or_default());
            }
            // Legacy projects without a team stay visible.
            for a in st.db.list_apps().unwrap_or_default() {
                if a.team_id.is_none() && !out.iter().any(|x| x.name == a.name) {
                    out.push(a);
                }
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            out
        }
    };
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
    if st.db.get_app(&b.name).ok().flatten().is_some() {
        return (StatusCode::CONFLICT, "project already exists\n").into_response();
    }
    let team_id = match team_for_caller(&st, &caller, b.team.as_deref()) {
        Ok(t) => t,
        Err(r) => return r,
    };
    let app = crate::db::App {
        name: b.name.clone(),
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
    match st.db.upsert_app(&app) {
        Ok(_) => {
            st.sync_traefik();
            Json(app).into_response()
        }
        Err(e) => internal(e),
    }
}

async fn get_project(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
) -> Response {
    match project_for(&st, &caller, &app) {
        Ok(a) => Json(a).into_response(),
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
    let _ = tokio::fs::remove_dir_all(st.builds_dir().join(&app)).await;
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
    let env = match normalize_env(q.env.as_deref().unwrap_or("production")) {
        Some(e) => e,
        None => return bad("env must be production or preview"),
    };

    let existing = match st.db.get_app(&app) {
        Ok(a) => a,
        Err(e) => return internal(e),
    };
    let mut team_id = None;
    if let Some(a) = &existing {
        if let Err(r) = require_write(&st, &caller, a) {
            return r;
        }
        team_id = a.team_id.clone();
    } else if caller.user.is_some() {
        match team_for_caller(&st, &caller, None) {
            Ok(t) => team_id = t,
            Err(r) => return r,
        }
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
        if let Err(e) = st.db.upsert_app(&row) {
            return internal(e);
        }
    }

    let author = author_of(&st, &caller);
    match deploy::deploy_dir(
        &st,
        &app,
        &dir,
        q.port,
        q.domain,
        env,
        author.as_deref(),
    )
    .await
    {
        Ok(out) => Json(out).into_response(),
        Err(e) => {
            if env == "production" {
                let _ = st.db.set_status(&app, "failed");
            }
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

#[derive(Deserialize)]
struct LogQuery {
    env: Option<String>,
    limit: Option<usize>,
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
    let build = st.db.last_deploy_log(&app).unwrap_or_default();
    let runtime = if a.kind == "docker" {
        deploy::container_logs(&app, env, 200).await
    } else {
        "no container (static app)".to_string()
    };
    Json(json!({"build": build.unwrap_or_default(), "runtime": runtime, "env": env}))
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
    let env = normalize_env(q.env.as_deref().unwrap_or("production")).unwrap_or("production");
    if a.kind != "docker" {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(axum::body::Body::from("no container (static app)\n"))
            .unwrap();
    }
    if !crate::deploy::docker_available().await {
        return (StatusCode::SERVICE_UNAVAILABLE, "docker unavailable\n").into_response();
    }

    let name = deploy::container_name(&app, env);
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(64);
    tokio::task::spawn_blocking(move || {
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
    Json(vars): Json<HashMap<String, String>>,
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
    put_env_impl(&st, &app, &env, vars)
}

async fn put_env_root(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
    Json(vars): Json<HashMap<String, String>>,
) -> Response {
    put_env(State(st), Extension(caller), Path((app, "production".into())), Json(vars)).await
}

fn put_env_impl(
    st: &AppState,
    app: &str,
    env: &str,
    vars: HashMap<String, String>,
) -> Response {
    let existing: HashMap<String, bool> = st
        .db
        .env(app, env)
        .unwrap_or_default()
        .into_iter()
        .map(|(k, _, s)| (k, s))
        .collect();
    for (_, v) in &vars {
        if v.len() > 64 * 1024 {
            return bad("env value too large");
        }
    }
    let kv: Vec<(String, String, bool)> = vars
        .into_iter()
        .map(|(k, v)| {
            let secret = existing.get(&k).copied().unwrap_or(false);
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
    if let Err(r) = project_for(&st, &caller, &app) {
        return r;
    }
    if !matches!(env.as_str(), "production" | "preview" | "development") {
        return bad("env must be production, preview or development");
    }
    match st.db.env(&app, &env) {
        Ok(kv) => Json(json!({
            "env": kv.into_iter().map(|(k, v, _)| (k, v)).collect::<HashMap<_, _>>(),
            "resolved": st.db.env_resolved(&app, &env).unwrap_or_default()
                .into_iter().collect::<HashMap<_, _>>(),
        }))
        .into_response(),
        Err(e) => internal(e),
    }
}

async fn get_env_root(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(app): Path<String>,
) -> Response {
    get_env(State(st), Extension(caller), Path((app, "production".into()))).await
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
    match st.db.domains_for(&app) {
        Ok(d) => Json(json!({"domains": d})).into_response(),
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
    if let Ok(Some(other)) = st.db.domain_owner(&host) {
        if other != app {
            return (StatusCode::CONFLICT, "domain belongs to another project\n").into_response();
        }
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
    match st.db.remove_domain(&host) {
        Ok(_) => {
            st.sync_traefik();
            Json(json!({"removed": host})).into_response()
        }
        Err(e) => internal(e),
    }
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
    let all = st.db.deploys(&app, 100).unwrap_or_default();
    let production = all.iter().filter(|d| d.env == "production").count();
    let preview = all.iter().filter(|d| d.env == "preview").count();
    let latest = all.first().map(|d| {
        json!({"id": d.id, "status": d.status, "env": d.env, "created_at": d.created_at})
    });
    Json(json!({
        "project": a.name,
        "status": a.status,
        "kind": a.kind,
        "production_deploy": a.production_deploy,
        "deployments": {"total": all.len(), "production": production, "preview": preview},
        "latest": latest,
        "domains": st.db.domains_for(&app).unwrap_or_default().len(),
        "env_keys": {
            "production": st.db.env(&app, "production").unwrap_or_default().len(),
            "preview": st.db.env(&app, "preview").unwrap_or_default().len(),
        },
        "requests": st.db.access_logs(&app, 1).unwrap_or_default().len(),
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
    }
    if b.settings.basic_auth_user.as_deref().unwrap_or("").is_empty() {
        b.settings.basic_auth_user = None;
        b.settings.basic_auth_hash = None;
    }
    b.settings.headers.retain(|(k, v)| {
        !k.is_empty()
            && k.len() <= 100
            && !k.contains(['\r', '\n', ':'])
            && v.len() <= 2000
            && !v.contains(['\r', '\n'])
    });
    b.settings.headers.truncate(20);
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
    let apps = match &caller.user {
        None => st.db.list_apps().unwrap_or_default(),
        Some(uid) => {
            let mut out = Vec::new();
            for team in st.db.teams_for(uid).unwrap_or_default() {
                out.extend(st.db.apps_for_team(&team.id).unwrap_or_default());
            }
            for a in st.db.list_apps().unwrap_or_default() {
                if a.team_id.is_none() && !out.iter().any(|x| x.name == a.name) {
                    out.push(a);
                }
            }
            out
        }
    };
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
    let value = json!({"status": "pending", "requested_at": crate::db::now(), "expires": null});
    match st
        .db
        .set_setting(&format!("cert:{host}"), &value.to_string())
    {
        Ok(_) => Json(json!({"ok": true, "host": host, "status": "pending"})).into_response(),
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
    let team = match st.db.team(&team).ok().flatten().or_else(|| {
        st.db.team_by_slug(&team).ok().flatten()
    }) {
        Some(t) => t,
        None => return not_found(),
    };
    if let Some(uid) = &caller.user {
        let role = st.db.role(&team.id, uid).ok().flatten();
        if !crate::db::can_write(role.as_deref()) {
            return forbidden();
        }
    }
    let role = match b.role.as_str() {
        "owner" | "admin" | "member" | "viewer" => b.role,
        _ => "member".into(),
    };
    match st.db.create_invite(&team.id, &role, 7 * 86400) {
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
        if let Err(e) = st.db.upsert_app(&row) {
            return internal(e);
        }
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
        "#!/bin/sh\nunset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_QUARANTINE_PATH\nexec '{}' deploy-git '{}' --data '{}' >> '{}' 2>&1\n",
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
    let a = match project_for(&st, &caller, &app) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = require_write(&st, &caller, &a) {
        return r;
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
        Some(exp) if !exp.is_empty() && constant_eq(&exp, &given) => {}
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

fn constant_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
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
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .await?;
    if !out.status.success() {
        anyhow::bail!("git clone failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let head = tokio::process::Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(&dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .await?;
    if !head.status.success() {
        anyhow::bail!("repo has no commits yet; push a branch first");
    }
    deploy::deploy_dir(st, app, &dir, None, None, "production", None).await
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
    use super::{constant_eq, normalize_env, valid_host, valid_name};

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
    fn deploy_envs() {
        assert_eq!(normalize_env("production"), Some("production"));
        assert_eq!(normalize_env("preview"), Some("preview"));
        assert_eq!(normalize_env(""), Some("production"));
        assert_eq!(normalize_env("staging"), None);
    }
}
