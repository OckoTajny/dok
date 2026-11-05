use crate::db::{Db, User};
use crate::state::AppState;
use axum::body::Body;
use axum::http::{header, HeaderValue, Request, Response, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use url::Url;
use uuid::Uuid;
use webauthn_rs::prelude::*;

pub struct AuthState {
    web: Webauthn,
    secure: bool,
    pending_reg: Mutex<HashMap<String, PendingReg>>,
    pending_auth: Mutex<HashMap<String, (PasskeyAuthentication, i64)>>,
}

struct PendingReg {
    state: PasskeyRegistration,
    user_id: String,
    email: String,
    name: String,
    team: Option<String>,
    invite: Option<String>,
    exp: i64,
}

const COOKIE: &str = "dok_session";
const SESSION_TTL: i64 = 60 * 60 * 24 * 30;

impl AuthState {
    pub fn new(origin: Url) -> anyhow::Result<Self> {
        let rp_id = origin
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("origin has no host"))?
            .to_string();
        let web = WebauthnBuilder::new(&rp_id, &origin)?
            .rp_name("dok")
            .build()?;
        Ok(Self {
            web,
            secure: origin.scheme() == "https",
            pending_reg: Mutex::new(HashMap::new()),
            pending_auth: Mutex::new(HashMap::new()),
        })
    }

    fn start_session(&self, db: &Db, user_id: &str) -> String {
        let id = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let csrf = Uuid::new_v4().simple().to_string();
        let _ = db.create_session(&hash(&id), user_id, &csrf, SESSION_TTL);
        id
    }

    pub fn session(&self, db: &Db, req: &Request<Body>) -> Option<Session> {
        let id = cookie(req, COOKIE)?;
        let h = hash(&id);
        match db.session(&h) {
            Ok(Some((user_id, csrf, exp))) if exp > crate::db::now() => {
                let user = db.user(&user_id).ok().flatten()?;
                Some(Session { user, csrf })
            }
            _ => None,
        }
    }

    fn cookie_header(&self, value: &str, max_age: i64) -> HeaderValue {
        let mut c = format!("{COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}");
        if self.secure {
            c.push_str("; Secure");
        }
        HeaderValue::from_str(&c).unwrap()
    }
}

pub struct Session {
    pub user: User,
    pub csrf: String,
}

fn client_ip(st: &AppState, req: &Request<Body>) -> String {
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_else(|| "unknown".into());
    if st.trusted_peer(&peer) {
        if let Some(ip) = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|s| s.trim())
            .filter(|s| s.parse::<std::net::IpAddr>().is_ok())
        {
            return ip.to_string();
        }
    }
    peer
}

fn hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}

fn cookie(req: &Request<Body>, name: &str) -> Option<String> {
    let h = req.headers().get(header::COOKIE)?.to_str().ok()?;
    for part in h.split(';') {
        if let Some((k, v)) = part.trim().split_once('=') {
            if k == name {
                return Some(v.to_string());
            }
        }
    }
    None
}

pub async fn handle(st: &AppState, req: Request<Body>) -> Option<Response<Body>> {
    let auth = st.auth.as_ref()?;
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let session = auth.session(&st.db, &req);
    let first_run = st.db.user_count().ok()? == 0;
    if (path.starts_with("/auth/setup")
        || path.starts_with("/auth/login")
        || path.starts_with("/auth/join"))
        && !st.rate.allow(&format!("auth:{}", client_ip(st, &req)), 10, 60)
    {
        return Some((StatusCode::TOO_MANY_REQUESTS, "too many attempts\n").into_response());
    }

    match (method.as_str(), path.as_str()) {
        ("GET", "/") => {
            if session.is_some() {
                Some(redirect("/dashboard"))
            } else if first_run {
                Some(crate::ui::page_setup())
            } else {
                Some(crate::ui::page_login())
            }
        }
        ("GET", "/login") => Some(crate::ui::page_login()),
        ("GET", p) if p.starts_with("/join/") => {
            let token = p.trim_start_matches("/join/").to_string();
            match (st.db.invite_details(&token), &session) {
                (Ok(Some((team, role, inviter))), Some(session)) => {
                    Some(crate::ui::page_join_signed_in(
                        &token,
                        &team,
                        &role,
                        inviter.as_deref(),
                        &session.csrf,
                    ))
                }
                (Ok(Some((team, role, inviter))), None) => {
                    Some(crate::ui::page_join(&token, &team, &role, inviter.as_deref()))
                }
                _ => Some(crate::ui::page_join_invalid()),
            }
        }
        ("POST", "/auth/setup/start") => Some(setup_start(st, auth, req).await),
        ("POST", "/auth/setup/finish") => Some(register_finish(st, auth, req).await),
        ("POST", "/auth/join/start") => Some(join_start(st, auth, req).await),
        ("POST", "/auth/join/accept") => Some(join_accept(st, req, session.as_ref()).await),
        ("POST", "/auth/login/start") => Some(login_start(st, auth)),
        ("POST", "/auth/login/finish") => Some(login_finish(st, auth, req).await),
        ("POST", "/auth/logout") => Some(logout(auth, &st.db, &req, session.as_ref())),
        ("GET", "/auth/status") => Some(
            Json(json!({
                "authenticated": session.is_some(),
                "first_run": first_run,
            }))
            .into_response(),
        ),
        _ => crate::ui::handle(st, auth, req, session).await,
    }
}

#[derive(Deserialize)]
struct SetupBody {
    email: String,
    name: String,
    team: Option<String>,
    invite: Option<String>,
}

async fn setup_start(st: &AppState, auth: &AuthState, req: Request<Body>) -> Response<Body> {
    if st.db.user_count().unwrap_or(0) != 0 {
        return forbidden("this instance is already set up; use an invite link");
    }
    let b: SetupBody = match read_json(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    register_start(
        st,
        auth,
        &b.email,
        &b.name,
        b.team.or_else(|| Some("Personal".into())),
        b.invite,
        None,
    )
}

async fn join_start(st: &AppState, auth: &AuthState, req: Request<Body>) -> Response<Body> {
    #[derive(Deserialize)]
    struct JoinBody {
        token: String,
        email: String,
        name: String,
    }
    let b: JoinBody = match read_json(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    if st.db.invite(&b.token).ok().flatten().is_none() {
        return bad("invite is invalid or expired");
    }
    register_start(st, auth, &b.email, &b.name, None, Some(b.token), None)
}

async fn join_accept(
    st: &AppState,
    req: Request<Body>,
    session: Option<&Session>,
) -> Response<Body> {
    let Some(session) = session else {
        return redirect("/");
    };
    let ip = client_ip(st, &req);
    if !st.rate.allow(&format!("join:{ip}"), 10, 60) {
        return (StatusCode::TOO_MANY_REQUESTS, "too many attempts\n").into_response();
    }
    let bytes = match axum::body::to_bytes(req.into_body(), 64 * 1024).await {
        Ok(b) => b,
        Err(_) => return bad("body too large"),
    };
    let form: HashMap<String, String> =
        match serde_urlencoded::from_bytes::<HashMap<String, String>>(&bytes) {
            Ok(f) => f,
            Err(_) => return bad("bad form body"),
        };
    if form.get("csrf").map(String::as_str) != Some(session.csrf.as_str()) {
        return forbidden("invalid CSRF token");
    }
    let token = form.get("token").cloned().unwrap_or_default();
    match st.db.use_invite(&token, &session.user.id) {
        Ok(Some(team_id)) => {
            let slug = st
                .db
                .team(&team_id)
                .ok()
                .flatten()
                .map(|t| t.slug)
                .unwrap_or_else(|| "personal".into());
            crate::ui::redirect_flash(
                &format!("/dashboard/team?team={slug}"),
                true,
                "You joined the team.",
            )
        }
        _ => crate::ui::page_join_invalid(),
    }
}

fn register_start(
    st: &AppState,
    auth: &AuthState,
    email: &str,
    name: &str,
    team: Option<String>,
    invite: Option<String>,
    _unused: Option<()>,
) -> Response<Body> {
    let email = email.trim().to_ascii_lowercase();
    if !email.contains('@') || name.trim().is_empty() {
        return bad("name and a valid email are required");
    }
    if st.db.user_by_email(&email).ok().flatten().is_some() {
        return bad("a user with that email already exists; log in instead");
    }
    let user_id = Uuid::new_v4().to_string();
    let uid = Uuid::parse_str(&user_id).unwrap();
    let exclude = None;
    match auth
        .web
        .start_passkey_registration(uid, &email, name, exclude)
    {
        Ok((ccr, state)) => {
            let id = Uuid::new_v4().simple().to_string();
            let now = crate::db::now();
            let mut p = auth.pending_reg.lock().unwrap();
            p.retain(|_, v| v.exp > now);
            if p.len() >= 64 {
                if let Some(k) = p.iter().min_by_key(|(_, v)| v.exp).map(|(k, _)| k.clone()) {
                    p.remove(&k);
                }
            }
            p.insert(
                id.clone(),
                PendingReg {
                    state,
                    user_id,
                    email,
                    name: name.to_string(),
                    team,
                    invite,
                    exp: now + 600,
                },
            );
            let mut v = serde_json::to_value(ccr).unwrap_or_default();
            v["id"] = json!(id);
            Json(v).into_response()
        }
        Err(e) => bad(&format!("cannot start registration: {e}")),
    }
}

#[derive(Deserialize)]
struct FinishBody {
    id: String,
    credential: Value,
}

async fn register_finish(st: &AppState, auth: &AuthState, req: Request<Body>) -> Response<Body> {
    let body: FinishBody = match read_json(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let cred: RegisterPublicKeyCredential = match serde_json::from_value(body.credential) {
        Ok(c) => c,
        Err(e) => return bad(&format!("bad credential: {e}")),
    };
    let pending = {
        let now = crate::db::now();
        let mut p = auth.pending_reg.lock().unwrap();
        p.remove(&body.id).filter(|v| v.exp > now)
    };
    let Some(p) = pending else {
        return bad("registration expired");
    };

    let passkey = match auth.web.finish_passkey_registration(&cred, &p.state) {
        Ok(k) => k,
        Err(e) => return bad(&format!("registration failed: {e}")),
    };
    let data = match serde_json::to_string(&passkey) {
        Ok(d) => d,
        Err(e) => return bad(&format!("cannot store passkey: {e}")),
    };

    let user = if let Some(invite) = &p.invite {
        match st.db.use_invite(invite, &p.user_id) {
            Ok(Some(_)) => {}
            _ => return bad("invite is no longer valid"),
        }
        match st.db.create_user_with_id(&p.user_id, &p.email, &p.name) {
            Ok(u) => u,
            Err(e) => return bad(&format!("cannot create user: {e}")),
        }
    } else {
        if !st.db.claim_first_user().unwrap_or(false) {
            return forbidden("this instance is already set up; use an invite link");
        }
        let u = match st.db.create_user_with_id(&p.user_id, &p.email, &p.name) {
            Ok(u) => u,
            Err(e) => return bad(&format!("cannot create user: {e}")),
        };
        if st.db.user_count().unwrap_or(0) == 1 {
            let team = p.team.unwrap_or_else(|| "Personal".into());
            let slug = slugify(&team);
            match st.db.create_team(&team, &slug, &u.id) {
                Ok(t) => {
                    let _ = st.db.assign_orphan_apps(&t.id);
                }
                Err(e) => return bad(&format!("cannot create team: {e}")),
            }
            let _ = st.db.assign_orphan_tokens(&u.id);
        }
        u
    };
    if let Err(e) = st.db.add_passkey(&user.id, &data) {
        return bad(&format!("cannot store passkey: {e}"));
    }
    let sid = auth.start_session(&st.db, &user.id);
    with_cookie(auth, sid, json!({"ok": true}))
}

fn login_start(st: &AppState, auth: &AuthState) -> Response<Body> {
    let mut keys = Vec::new();
    for (_, data) in st.db.all_passkeys().unwrap_or_default() {
        if let Ok(k) = serde_json::from_str::<Passkey>(&data) {
            keys.push(k);
        }
    }
    if keys.is_empty() {
        return Json(json!({"setup": true})).into_response();
    }
    match auth.web.start_passkey_authentication(&keys) {
        Ok((rcr, state)) => {
            let id = Uuid::new_v4().simple().to_string();
            let now = crate::db::now();
            let mut p = auth.pending_auth.lock().unwrap();
            p.retain(|_, (_, exp)| *exp > now);
            if p.len() >= 64 {
                if let Some(k) = p.iter().min_by_key(|(_, (_, exp))| *exp).map(|(k, _)| k.clone()) {
                    p.remove(&k);
                }
            }
            p.insert(id.clone(), (state, now + 600));
            let mut v = serde_json::to_value(rcr).unwrap_or_default();
            v["id"] = json!(id);
            Json(v).into_response()
        }
        Err(e) => bad(&format!("cannot start login: {e}")),
    }
}

async fn login_finish(st: &AppState, auth: &AuthState, req: Request<Body>) -> Response<Body> {
    let body: FinishBody = match read_json(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let cred: PublicKeyCredential = match serde_json::from_value(body.credential) {
        Ok(c) => c,
        Err(e) => return bad(&format!("bad credential: {e}")),
    };
    let state = {
        let now = crate::db::now();
        let mut p = auth.pending_auth.lock().unwrap();
        p.remove(&body.id)
            .and_then(|(s, exp)| if exp > now { Some(s) } else { None })
    };
    let Some(state) = state else {
        return bad("login expired");
    };
    let result = match auth.web.finish_passkey_authentication(&cred, &state) {
        Ok(r) => r,
        Err(e) => return bad(&format!("login failed: {e}")),
    };
    let cred_id = result.cred_id().clone();
    let mut found: Option<String> = None;
    for (user_id, data) in st.db.all_passkeys().unwrap_or_default() {
        if let Ok(k) = serde_json::from_str::<Passkey>(&data) {
            if k.cred_id() == &cred_id {
                let mut keys = load_keys(&st.db, &user_id);
                for key in keys.iter_mut() {
                    key.update_credential(&result);
                }
                save_keys(&st.db, &user_id, &keys);
                found = Some(user_id);
                break;
            }
        }
    }
    let Some(user_id) = found else {
        return bad("unknown credential");
    };
    let sid = auth.start_session(&st.db, &user_id);
    with_cookie(auth, sid, json!({"ok": true}))
}

fn load_keys(db: &Db, user_id: &str) -> Vec<Passkey> {
    db.passkeys_for(user_id)
        .unwrap_or_default()
        .iter()
        .filter_map(|d| serde_json::from_str::<Passkey>(d).ok())
        .collect()
}

fn save_keys(db: &Db, user_id: &str, keys: &[Passkey]) {
    let data: Vec<String> = keys
        .iter()
        .filter_map(|k| serde_json::to_string(k).ok())
        .collect();
    let _ = db.replace_passkeys(user_id, &data);
}

fn logout(
    auth: &AuthState,
    db: &Db,
    req: &Request<Body>,
    session: Option<&Session>,
) -> Response<Body> {
    if let Some(session) = session {
        let csrf = req
            .headers()
            .get("x-csrf")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if csrf != session.csrf {
            return forbidden("invalid CSRF token");
        }
    }
    if let Some(id) = cookie(req, COOKIE) {
        let _ = db.delete_session(&hash(&id));
    }
    let mut r = Json(json!({"ok": true})).into_response();
    r.headers_mut()
        .insert(header::SET_COOKIE, auth.cookie_header("", 0));
    r
}

async fn read_json<T: for<'de> Deserialize<'de>>(req: Request<Body>) -> Result<T, Response<Body>> {
    let bytes = axum::body::to_bytes(req.into_body(), 64 * 1024)
        .await
        .map_err(|_| bad("body too large"))?;
    serde_json::from_slice(&bytes).map_err(|e| bad(&format!("bad json: {e}")))
}

fn with_cookie(auth: &AuthState, session: String, body: Value) -> Response<Body> {
    let mut r = Json(body).into_response();
    r.headers_mut()
        .insert(header::SET_COOKIE, auth.cookie_header(&session, SESSION_TTL));
    r
}

pub fn redirect(to: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, to)
        .body(Body::empty())
        .unwrap()
}

fn forbidden(msg: &str) -> Response<Body> {
    (StatusCode::FORBIDDEN, format!("{msg}\n")).into_response()
}

fn bad(msg: &str) -> Response<Body> {
    (StatusCode::BAD_REQUEST, format!("{msg}\n")).into_response()
}

/// Escape untrusted text for HTML text nodes and quoted attributes.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "team".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{esc, setup_start, slugify, AuthState};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use url::Url;

    #[test]
    fn slugs() {
        assert_eq!(slugify("My Team"), "my-team");
        assert_eq!(slugify("  --a__b--  "), "a-b");
        assert_eq!(slugify("???"), "team");
    }

    #[tokio::test]
    async fn setup_is_closed_after_first_user() {
        let path = std::env::temp_dir().join(format!("dok-auth-{}.db", uuid::Uuid::new_v4()));
        let db = crate::db::Db::open(&path).unwrap();
        db.create_user("first@example.com", "First").unwrap();
        let data = std::env::temp_dir().join(format!("dok-auth-data-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data).unwrap();
        let st = crate::state::AppState::new(db, data, None, "http://localhost".into());
        let auth = AuthState::new(Url::parse("http://localhost").unwrap()).unwrap();
        let req = Request::builder()
            .method("POST")
            .uri("/auth/setup/start")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"email":"second@example.com","name":"Second"}"#))
            .unwrap();
        let resp = setup_start(&st, &auth, req).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn escapes_html() {
        assert_eq!(esc("<a href=\"x\">&'</a>"), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;");
    }
}
