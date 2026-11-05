use crate::db::Db;
use crate::state::AppState;
use axum::body::Body;
use axum::http::{header, HeaderValue, Request, Response, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
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
    sessions: Mutex<HashMap<String, i64>>,
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
            sessions: Mutex::new(HashMap::new()),
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

    fn new_session(&self) -> String {
        let id = format!(
            "{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        let now = crate::db::now();
        let mut s = self.sessions.lock().unwrap();
        s.retain(|_, exp| *exp > now);
        s.insert(id.clone(), now + SESSION_TTL);
        id
    }

    fn valid_session(&self, req: &Request<Body>) -> bool {
        let Some(id) = cookie(req, COOKIE) else {
            return false;
        };
        let now = crate::db::now();
        let s = self.sessions.lock().unwrap();
        s.get(&id).map(|exp| *exp > now).unwrap_or(false)
    }

    fn cookie_header(&self, value: &str, max_age: i64) -> HeaderValue {
        let mut c = format!("{COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}");
        if self.secure {
            c.push_str("; Secure");
        }
        HeaderValue::from_str(&c).unwrap()
    }
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

    match (method.as_str(), path.as_str()) {
        ("GET", "/") => {
            if auth.valid_session(&req) {
                Some(redirect("/dashboard"))
            } else {
                Some(html(&login_page(auth.passkeys(&st.db).is_empty())))
            }
        }
        ("GET", "/dashboard") => {
            if !auth.valid_session(&req) {
                return Some(redirect("/"));
            }
            Some(html(&dashboard_page(st)))
        }
        ("POST", "/auth/register/start") => {
            let authed = auth.valid_session(&req);
            Some(register_start(st, auth, authed))
        }
        ("POST", "/auth/register/finish") => Some(register_finish(st, auth, req).await),
        ("POST", "/auth/login/start") => Some(login_start(st, auth)),
        ("POST", "/auth/login/finish") => Some(login_finish(st, auth, req).await),
        ("POST", "/auth/logout") => Some(logout(auth, &req)),
        ("GET", "/auth/status") => Some(Json(
            json!({
                "authenticated": auth.valid_session(&req),
                "has_passkeys": !auth.passkeys(&st.db).is_empty(),
            }),
        )
        .into_response()),
        _ => None,
    }
}

fn register_start(st: &AppState, auth: &AuthState, authed: bool) -> Response<Body> {
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
            let id = auth.new_session();
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
            let id = auth.new_session();
            with_cookie(auth, id, json!({"ok": true}))
        }
        Err(e) => bad(&format!("login failed: {e}")),
    }
}

fn logout(auth: &AuthState, req: &Request<Body>) -> Response<Body> {
    if let Some(id) = cookie(req, COOKIE) {
        auth.sessions.lock().unwrap().remove(&id);
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

fn dashboard_page(st: &AppState) -> String {
    let apps = st.db.list_apps().unwrap_or_default();
    let rows = apps
        .iter()
        .map(|a| {
            let target = a
                .domain
                .clone()
                .map(|d| format!("https://{d}"))
                .unwrap_or_else(|| "—".to_string());
            format!(
                "<tr><td><b>{}</b></td><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td></tr>",
                a.name, a.kind, a.status, target, a.updated_at
            )
        })
        .collect::<String>();
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>dok</title>
<style>
:root{{color-scheme:light dark}}
body{{font:15px/1.5 system-ui,sans-serif;max-width:60rem;margin:3rem auto;padding:0 1rem}}
h1{{font-size:1.4rem;margin:0 0 1.5rem}}
table{{width:100%;border-collapse:collapse}}
th,td{{text-align:left;padding:.5rem .6rem;border-bottom:1px solid #8883}}
th{{font-weight:600;opacity:.7}}
code{{background:#8881;padding:.1rem .3rem;border-radius:4px}}
.bar{{display:flex;justify-content:space-between;align-items:center;margin-bottom:1rem}}
button{{font:inherit;padding:.4rem .8rem;border-radius:6px;border:1px solid #8886;background:transparent;cursor:pointer}}
</style></head><body>
<div class="bar"><h1>dok</h1><button onclick="fetch('/auth/logout',{{method:'POST'}}).then(()=>location.href='/')">Log out</button></div>
<table><thead><tr><th>app</th><th>kind</th><th>status</th><th>domain</th><th>updated</th></tr></thead>
<tbody>{rows}</tbody></table>
<p style="opacity:.6;margin-top:1.5rem">Deploy with <code>dok push</code>. No apps yet? Push your first one.</p>
</body></html>"#
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
button{{font:inherit;font-size:1rem;padding:.7rem 1.1rem;border-radius:8px;border:1px solid #8886;cursor:pointer;width:100%}}
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
