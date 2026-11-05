use crate::db::Db;
use crate::state::AppState;
use axum::body::Body;
use axum::http::{header, HeaderValue, Request, Response, StatusCode};
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
    pending_reg: Mutex<HashMap<String, (PasskeyRegistration, i64)>>,
    pending_auth: Mutex<HashMap<String, (PasskeyAuthentication, i64)>>,
}

const COOKIE: &str = "dok_session";
const SESSION_TTL: i64 = 60 * 60 * 24 * 30;
const PASSKEY_KEY: &str = "passkeys";
const USER_KEY: &str = "user_id";

impl AuthState {
    pub fn new(origin: Url) -> anyhow::Result<Self> {
        let rp_id = origin
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("origin has no host"))?
            .to_string();
        let web = WebauthnBuilder::new(&rp_id, &origin)?
            .rp_name("dok")
            .build()?;
        let secure = origin.scheme() == "https";
        Ok(Self {
            web,
            secure,
            pending_reg: Mutex::new(HashMap::new()),
            pending_auth: Mutex::new(HashMap::new()),
        })
    }

    fn passkeys(&self, db: &Db) -> Vec<Passkey> {
        db.get_setting(PASSKEY_KEY)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save_passkeys(&self, db: &Db, keys: &[Passkey]) -> anyhow::Result<()> {
        db.set_setting(PASSKEY_KEY, &serde_json::to_string(keys)?)?;
        Ok(())
    }

    fn user_id(&self, db: &Db) -> Uuid {
        if let Ok(Some(v)) = db.get_setting(USER_KEY) {
            if let Ok(u) = Uuid::parse_str(&v) {
                return u;
            }
        }
        let u = Uuid::new_v4();
        let _ = db.set_setting(USER_KEY, &u.to_string());
        u
    }

    fn start_session(&self, db: &Db) -> String {
        let id = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let csrf = Uuid::new_v4().simple().to_string();
        let _ = db.create_session(&hash(&id), &csrf, SESSION_TTL);
        id
    }

    fn session(&self, db: &Db, req: &Request<Body>) -> Option<Session> {
        let id = cookie(req, COOKIE)?;
        let h = hash(&id);
        match db.session(&h) {
            Ok(Some((csrf, exp))) if exp > crate::db::now() => Some(Session { csrf }),
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

struct Session {
    csrf: String,
}

fn hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}

fn cookie(req: &Request<Body>, name: &str) -> Option<String> {
    let h = req.headers().get(header::COOKIE)?.to_str().ok()?;
    for part in h.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
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

    match (method.as_str(), path.as_str()) {
        ("GET", "/") => {
            if session.is_some() {
                Some(redirect("/dashboard"))
            } else {
                Some(html(&login_page(auth.passkeys(&st.db).is_empty())))
            }
        }
        ("GET", "/dashboard") => {
            let Some(s) = session else {
                return Some(redirect("/"));
            };
            Some(html(&dashboard_page(st, &s)))
        }
        ("POST", "/auth/register/start") => {
            Some(register_start(st, auth, session.is_some()).await)
        }
        ("POST", "/auth/register/finish") => Some(register_finish(st, auth, req).await),
        ("POST", "/auth/login/start") => Some(login_start(st, auth)),
        ("POST", "/auth/login/finish") => Some(login_finish(st, auth, req).await),
        ("POST", "/auth/logout") => Some(logout(auth, &st.db, &req)),
        ("GET", "/auth/status") => Some(
            Json(json!({
                "authenticated": session.is_some(),
                "has_passkeys": !auth.passkeys(&st.db).is_empty(),
            }))
            .into_response(),
        ),
        (m, p) if m == "POST" && p.starts_with("/dashboard/") => {
            Some(dashboard_action(st, req, session).await)
        }
        (m, p) if m == "GET" && p.starts_with("/dashboard/logs/") => {
            let Some(_s) = session else {
                return Some(redirect("/"));
            };
            let app = p.trim_start_matches("/dashboard/logs/").to_string();
            Some(html(&logs_page(st, &app).await))
        }
        _ => None,
    }
}

async fn dashboard_action(
    st: &AppState,
    req: Request<Body>,
    session: Option<Session>,
) -> Response<Body> {
    let Some(s) = session else {
        return redirect("/");
    };
    let path = req.uri().path().to_string();
    let body = match axum::body::to_bytes(req.into_body(), 64 * 1024).await {
        Ok(b) => b,
        Err(_) => return bad("body too large"),
    };
    let form: HashMap<String, String> =
        serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if form.get("csrf").map(|v| v.as_str()) != Some(s.csrf.as_str()) {
        return (StatusCode::FORBIDDEN, "bad csrf token\n").into_response();
    }

    let result = if let Some(app) = path.strip_prefix("/dashboard/redeploy/") {
        crate::api::deploy_from_repo(st, app).await.map(|_| ())
    } else if let Some(rest) = path.strip_prefix("/dashboard/rollback/") {
        match rest.split_once('/') {
            Some((app, deploy)) => crate::deploy::rollback(st, app, deploy).await.map(|_| ()),
            None => Err(anyhow::anyhow!("bad rollback path")),
        }
    } else {
        Err(anyhow::anyhow!("unknown action"))
    };

    match result {
        Ok(_) => redirect("/dashboard"),
        Err(e) => html(&format!(
            "<p>Action failed: {}</p><p><a href=\"/dashboard\">back</a></p>",
            esc(&e.to_string())
        )),
    }
}

async fn register_start(st: &AppState, auth: &AuthState, authed: bool) -> Response<Body> {
    let keys = auth.passkeys(&st.db);
    if !keys.is_empty() && !authed {
        return (StatusCode::FORBIDDEN, "already set up; log in first\n").into_response();
    }
    let user = auth.user_id(&st.db);
    let exclude = if keys.is_empty() {
        None
    } else {
        Some(keys.iter().map(|k| k.cred_id().clone()).collect())
    };
    match auth
        .web
        .start_passkey_registration(user, "dok", "dok admin", exclude)
    {
        Ok((ccr, state)) => {
            let id = Uuid::new_v4().simple().to_string();
            let now = crate::db::now();
            let mut pending = auth.pending_reg.lock().unwrap();
            pending.retain(|_, (_, exp)| *exp > now);
            if pending.len() > 64 {
                pending.clear();
            }
            pending.insert(id.clone(), (state, now + 300));
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
    let body = match read_json(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let cred: RegisterPublicKeyCredential = match serde_json::from_value(body.credential) {
        Ok(c) => c,
        Err(e) => return bad(&format!("bad credential: {e}")),
    };
    let state = {
        let now = crate::db::now();
        let mut p = auth.pending_reg.lock().unwrap();
        p.remove(&body.id)
            .and_then(|(s, exp)| if exp > now { Some(s) } else { None })
    };
    let Some(state) = state else {
        return bad("registration expired");
    };
    match auth.web.finish_passkey_registration(&cred, &state) {
        Ok(passkey) => {
            let mut keys = auth.passkeys(&st.db);
            keys.push(passkey);
            if let Err(e) = auth.save_passkeys(&st.db, &keys) {
                return bad(&format!("cannot save passkey: {e}"));
            }
            let id = auth.start_session(&st.db);
            with_cookie(auth, id, json!({"ok": true}))
        }
        Err(e) => bad(&format!("registration failed: {e}")),
    }
}

fn login_start(st: &AppState, auth: &AuthState) -> Response<Body> {
    let keys = auth.passkeys(&st.db);
    if keys.is_empty() {
        return Json(json!({"setup": true})).into_response();
    }
    match auth.web.start_passkey_authentication(&keys) {
        Ok((rcr, state)) => {
            let id = Uuid::new_v4().simple().to_string();
            let now = crate::db::now();
            let mut pending = auth.pending_auth.lock().unwrap();
            pending.retain(|_, (_, exp)| *exp > now);
            if pending.len() > 64 {
                pending.clear();
            }
            pending.insert(id.clone(), (state, now + 300));
            let mut v = serde_json::to_value(rcr).unwrap_or_default();
            v["id"] = json!(id);
            Json(v).into_response()
        }
        Err(e) => bad(&format!("cannot start login: {e}")),
    }
}

async fn login_finish(st: &AppState, auth: &AuthState, req: Request<Body>) -> Response<Body> {
    let body = match read_json(req).await {
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
    match auth.web.finish_passkey_authentication(&cred, &state) {
        Ok(result) => {
            let mut keys = auth.passkeys(&st.db);
            for k in keys.iter_mut() {
                k.update_credential(&result);
            }
            let _ = auth.save_passkeys(&st.db, &keys);
            let id = auth.start_session(&st.db);
            with_cookie(auth, id, json!({"ok": true}))
        }
        Err(e) => bad(&format!("login failed: {e}")),
    }
}

fn logout(auth: &AuthState, db: &Db, req: &Request<Body>) -> Response<Body> {
    if let Some(id) = cookie(req, COOKIE) {
        let _ = db.delete_session(&hash(&id));
    }
    let mut r = Json(json!({"ok": true})).into_response();
    r.headers_mut()
        .insert(header::SET_COOKIE, auth.cookie_header("", 0));
    r
}

async fn read_json(req: Request<Body>) -> Result<FinishBody, Response<Body>> {
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

fn redirect(to: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, to)
        .body(Body::empty())
        .unwrap()
}

fn html(body: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn bad(msg: &str) -> Response<Body> {
    (StatusCode::BAD_REQUEST, format!("{msg}\n")).into_response()
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn dashboard_page(st: &AppState, session: &Session) -> String {
    let apps = st.db.list_apps().unwrap_or_default();
    let mut rows = String::new();
    for a in &apps {
        let target = a
            .domain
            .clone()
            .map(|d| format!("https://{d}"))
            .unwrap_or_else(|| "—".into());
        let history = st
            .db
            .deploys(&a.name, 3)
            .unwrap_or_default()
            .iter()
            .map(|d| {
                format!(
                    "<form method=post action=\"/dashboard/rollback/{}/{}\" style=\"display:inline\">\
                     <input type=hidden name=csrf value=\"{}\">\
                     <button class=mini>rollback {}</button></form>",
                    esc(&a.name),
                    esc(&d.id),
                    session.csrf,
                    esc(&d.status)
                )
            })
            .collect::<String>();
        rows.push_str(&format!(
            "<tr><td><b>{}</b><div class=sub>{} · {}</div></td>\
             <td><code>{}</code></td>\
             <td class=actions>\
             <form method=post action=\"/dashboard/redeploy/{}\" style=\"display:inline\">\
             <input type=hidden name=csrf value=\"{}\">\
             <button>redeploy</button></form>\
             <a class=btn href=\"/dashboard/logs/{}\">logs</a>{}</td></tr>",
            esc(&a.name),
            esc(&a.kind),
            esc(&a.status),
            esc(&target),
            esc(&a.name),
            session.csrf,
            esc(&a.name),
            history
        ));
    }
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>dok</title>
<style>
:root{{color-scheme:light dark}}
body{{font:15px/1.5 system-ui,sans-serif;max-width:64rem;margin:3rem auto;padding:0 1rem}}
h1{{font-size:1.4rem;margin:0}}
table{{width:100%;border-collapse:collapse;margin-top:1.5rem}}
td,th{{text-align:left;padding:.6rem .6rem;border-bottom:1px solid #8883;vertical-align:top}}
th{{font-weight:600;opacity:.7;font-size:.85rem}}
code{{background:#8881;padding:.1rem .3rem;border-radius:4px;font-size:.9em}}
.sub{{opacity:.6;font-size:.85rem}}
.bar{{display:flex;justify-content:space-between;align-items:center}}
button,.btn{{font:inherit;font-size:.85rem;padding:.35rem .7rem;border-radius:6px;border:1px solid #8886;background:transparent;cursor:pointer;color:inherit;text-decoration:none;display:inline-block}}
.mini{{font-size:.75rem;padding:.2rem .45rem;opacity:.8}}
.actions{{white-space:nowrap}}
.actions>*{{margin-right:.3rem}}
</style></head><body>
<div class="bar"><h1>dok</h1>
<form method=post action="/auth/logout"><button>Log out</button></form></div>
<table><thead><tr><th>app</th><th>domain</th><th>actions</th></tr></thead>
<tbody>{rows}</tbody></table>
<p class="sub" style="margin-top:1.5rem">Deploy with <code>dok push</code>. Rollback keeps the last 3 builds.</p>
</body></html>"#
    )
}

async fn logs_page(st: &AppState, app: &str) -> String {
    let build = st
        .db
        .last_deploy_log(app)
        .ok()
        .flatten()
        .unwrap_or_default();
    let runtime = crate::deploy::container_logs(app, 300).await;
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>dok · {app}</title>
<style>
:root{{color-scheme:light dark}}
body{{font:14px/1.45 system-ui,sans-serif;max-width:72rem;margin:2rem auto;padding:0 1rem}}
pre{{background:#8881;padding:1rem;border-radius:8px;overflow:auto;font-size:.82rem;line-height:1.4}}
h2{{font-size:1rem;margin:1.6rem 0 .4rem;opacity:.8}}
a{{color:inherit}}
</style></head><body>
<p><a href="/dashboard">← dashboard</a></p>
<h1 style="font-size:1.3rem">{name}</h1>
<h2>build</h2><pre>{build}</pre>
<h2>runtime</h2><pre>{runtime}</pre>
</body></html>"#,
        app = esc(app),
        name = esc(app),
        build = esc(&build),
        runtime = esc(&runtime)
    )
}

fn login_page(setup: bool) -> String {
    let action = if setup { "register" } else { "login" };
    let label = if setup {
        "Create a passkey"
    } else {
        "Sign in with a passkey"
    };
    let hint = if setup {
        "First run: this passkey becomes the admin login. Proton Pass, iCloud, Google or a hardware key all work."
    } else {
        "Use the passkey stored in your password manager (Proton Pass, iCloud, Google) or a hardware key."
    };
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>dok</title>
<style>
:root{{color-scheme:light dark}}
body{{font:15px/1.6 system-ui,sans-serif;max-width:26rem;margin:12vh auto;padding:0 1.2rem}}
h1{{font-size:1.6rem;margin:0 0 .4rem}}
p{{opacity:.75;margin:.2rem 0 1.6rem}}
button{{font:inherit;font-size:1rem;padding:.7rem 1.1rem;border-radius:8px;border:1px solid #8886;cursor:pointer;width:100%;background:transparent;color:inherit}}
#msg{{margin-top:1rem;font-size:.9rem;color:#c33;white-space:pre-wrap}}
code{{background:#8881;padding:.1rem .3rem;border-radius:4px}}
</style></head><body>
<h1>dok</h1><p>{hint}</p>
<button id="go" onclick="{action}()">{label}</button>
<div id="msg"></div>
<script>
const b64u={{
  toBuf(s){{s=s.replace(/-/g,'+').replace(/_/g,'/');const p='='.repeat((4-s.length%4)%4);const b=atob(s+p);const u=new Uint8Array(b.length);for(let i=0;i<b.length;i++)u[i]=b.charCodeAt(i);return u.buffer}},
  fromBuf(b){{const u=new Uint8Array(b);let s='';for(const x of u)s+=String.fromCharCode(x);return btoa(s).replace(/\+/g,'-').replace(/\//g,'_').replace(/=+$/,'')}}
}};
async function jpost(url,body){{
  const r=await fetch(url,{{method:'POST',headers:{{'content-type':'application/json'}},body:JSON.stringify(body||{{}})}});
  if(!r.ok) throw new Error(await r.text());
  return r.json();
}}
function fail(e){{document.getElementById('msg').textContent=e.message||String(e)}}
async function register(){{
  try{{
    const o=await jpost('/auth/register/start');
    const pk=o.publicKey;
    pk.challenge=b64u.toBuf(pk.challenge);
    pk.user.id=b64u.toBuf(pk.user.id);
    if(pk.excludeCredentials) pk.excludeCredentials=pk.excludeCredentials.map(c=>({{...c,id:b64u.toBuf(c.id)}}));
    const c=await navigator.credentials.create({{publicKey:pk}});
    await jpost('/auth/register/finish',{{id:o.id,credential:{{id:c.id,rawId:b64u.fromBuf(c.rawId),type:c.type,response:{{clientDataJSON:b64u.fromBuf(c.response.clientDataJSON),attestationObject:b64u.fromBuf(c.response.attestationObject),transports:c.response.getTransports?c.response.getTransports():[]}}}}}});
    location.href='/dashboard';
  }}catch(e){{fail(e)}}
}}
async function login(){{
  try{{
    const o=await jpost('/auth/login/start');
    if(o.setup){{return register()}}
    const pk=o.publicKey;
    pk.challenge=b64u.toBuf(pk.challenge);
    pk.allowCredentials=(pk.allowCredentials||[]).map(c=>({{...c,id:b64u.toBuf(c.id)}}));
    const c=await navigator.credentials.get({{publicKey:pk}});
    await jpost('/auth/login/finish',{{id:o.id,credential:{{id:c.id,rawId:b64u.fromBuf(c.rawId),type:c.type,response:{{clientDataJSON:b64u.fromBuf(c.response.clientDataJSON),authenticatorData:b64u.fromBuf(c.response.authenticatorData),signature:b64u.fromBuf(c.response.signature),userHandle:c.response.userHandle?b64u.fromBuf(c.response.userHandle):null}}}}}});
    location.href='/dashboard';
  }}catch(e){{fail(e)}}
}}
</script></body></html>"#
    )
}

use axum::response::IntoResponse;
