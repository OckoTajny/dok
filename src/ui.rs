//! Server-rendered dashboard. No JS framework; only the passkey ceremony uses
//! a small inline script.

use crate::auth::{self, AuthState, Session};
use crate::db::{App, Db, Team, User};
use crate::state::AppState;
use axum::body::Body;
use axum::http::{header, Method, Request, Response, StatusCode};
use axum::response::IntoResponse;
use std::collections::HashMap;

const CSS: &str = r#"
:root{color-scheme:dark}
*{box-sizing:border-box}
html,body{margin:0;padding:0}
body{background:#000;color:#ededed;font:14px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI",system-ui,sans-serif;-webkit-font-smoothing:antialiased}
a{color:#ededed;text-decoration:none}
a:hover{color:#fff;text-decoration:underline}
code,pre{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
.wrap{max-width:1120px;margin:0 auto;padding:0 24px}
.top{display:flex;align-items:center;justify-content:space-between;padding:16px 0;border-bottom:1px solid #222}
.brand{font-weight:600;letter-spacing:-.02em;font-size:15px}
.muted{color:#888}
.small{font-size:12px}
.layout{display:grid;grid-template-columns:230px minmax(0,1fr);gap:36px;padding:28px 0 80px}
aside{font-size:13px}
.side-title{color:#777;text-transform:uppercase;letter-spacing:.08em;font-size:10px;margin:22px 0 8px}
.side a{display:block;padding:5px 8px;margin-left:-8px;border-radius:6px;color:#bbb;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.side a:hover{background:#111;color:#fff;text-decoration:none}
.side a.on{background:#171717;color:#fff}
main{min-width:0}
h1{font-size:24px;letter-spacing:-.03em;margin:0 0 6px;font-weight:600}
h2{font-size:15px;margin:32px 0 12px;font-weight:600}
h3{font-size:13px;margin:20px 0 8px;color:#bbb;font-weight:500}
.row{display:flex;align-items:center;gap:10px;flex-wrap:wrap}
.between{justify-content:space-between}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:12px}
.card{display:block;border:1px solid #222;border-radius:10px;padding:16px;background:#050505}
.card:hover{border-color:#444;text-decoration:none}
.card .name{font-weight:600;font-size:14px}
.badge{display:inline-flex;align-items:center;gap:5px;border:1px solid #333;border-radius:999px;padding:1px 8px;font-size:11px;color:#bbb;background:#0a0a0a;white-space:nowrap}
.badge.ok{color:#7ee787;border-color:#1f4426;background:#07130a}
.badge.failed{color:#ff7b72;border-color:#4b1f1f;background:#170808}
.badge.run{color:#e3b341;border-color:#4a3a16;background:#151004}
.badge.env{color:#79c0ff;border-color:#1f3a5f;background:#07111d}
.btn{display:inline-flex;align-items:center;gap:6px;border:1px solid #333;background:#111;color:#eee;border-radius:7px;padding:6px 11px;font:inherit;font-size:12px;cursor:pointer}
.btn:hover{background:#1a1a1a;border-color:#555}
.btn.primary{background:#ededed;color:#000;border-color:#ededed;font-weight:500}
.btn.primary:hover{background:#fff}
.btn.danger{color:#ff7b72;border-color:#4b1f1f}
.btn.link{background:none;border:0;padding:0;color:#bbb;font-size:12px}
.btn.link:hover{color:#fff;text-decoration:underline}
input,select,textarea{background:#0a0a0a;border:1px solid #333;color:#eee;border-radius:7px;padding:8px 10px;font:inherit;font-size:13px;width:100%}
input:focus,select:focus,textarea:focus{outline:none;border-color:#666}
textarea{min-height:150px;resize:vertical}
label{display:block;color:#999;font-size:12px;margin:12px 0 4px}
form.inline{display:inline}
.form{max-width:420px}
.form .btn{margin-top:16px}
table{width:100%;border-collapse:collapse;font-size:13px}
th{text-align:left;color:#777;font-weight:500;font-size:11px;text-transform:uppercase;letter-spacing:.05em;padding:8px 10px;border-bottom:1px solid #222}
td{padding:10px;border-bottom:1px solid #161616;vertical-align:middle}
tr:last-child td{border-bottom:0}
pre.log{background:#070707;border:1px solid #222;border-radius:8px;padding:12px;overflow:auto;max-height:420px;font-size:12px;color:#ccc;white-space:pre-wrap}
.tabs{display:flex;gap:2px;border-bottom:1px solid #222;margin:20px 0 0;flex-wrap:wrap}
.tabs a{padding:8px 12px;color:#888;font-size:13px;border-bottom:2px solid transparent;margin-bottom:-1px}
.tabs a:hover{color:#fff;text-decoration:none}
.tabs a.on{color:#fff;border-bottom-color:#ededed}
.head{padding:26px 0 4px}
.empty{border:1px dashed #333;border-radius:10px;padding:36px 20px;text-align:center;color:#888}
.notice{border:1px solid #1f4426;background:#07130a;color:#7ee787;border-radius:8px;padding:10px 12px;margin:12px 0;font-size:12px;word-break:break-all}
.error{border:1px solid #4b1f1f;background:#170808;color:#ff7b72;border-radius:8px;padding:10px 12px;margin:12px 0;font-size:12px}
.auth{max-width:400px;margin:11vh auto;padding:0 24px}
.auth h1{font-size:26px;margin-bottom:8px}
.auth p{color:#999;margin:0 0 24px}
.auth .card{padding:22px}
.toggle{display:flex;gap:10px;align-items:center;margin-top:14px;font-size:12px;color:#888}
.invite{font-family:ui-monospace,monospace;font-size:12px;word-break:break-all}
.foot{margin-top:40px;color:#555;font-size:11px}
@media(max-width:760px){.layout{grid-template-columns:1fr;gap:12px}.grid{grid-template-columns:1fr}.wrap{padding:0 16px}.head{padding-top:18px}}
"#;

pub fn esc(s: &str) -> String {
    auth::esc(s)
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\">\
<title>{t} · dok</title><style>{css}</style><body>{body}</body></html>",
        t = esc(title),
        css = CSS,
        body = body
    )
}

fn auth_page(title: &str, subtitle: &str, inner: &str, script: &str) -> String {
    let body = format!(
        "<div class=auth><div class=brand>dok</div><h1>{t}</h1><p>{s}</p><div class=card>{inner}</div>\
<div class=foot>Passkeys only — nothing to remember, nothing to leak.</div></div><script>{script}</script>",
        t = esc(title),
        s = esc(subtitle),
        inner = inner,
        script = script
    );
    page(title, &body)
}

const B64_JS: &str = r#"
const b64={
  toBuf(s){s=String(s).replace(/-/g,'+').replace(/_/g,'/');while(s.length%4)s+='=';const bin=atob(s);const u=new Uint8Array(bin.length);for(let i=0;i<bin.length;i++)u[i]=bin.charCodeAt(i);return u},
  fromBuf(buf){const u=new Uint8Array(buf);let s='';for(const b of u)s+=String.fromCharCode(b);return btoa(s).replace(/\+/g,'-').replace(/\//g,'_').replace(/=+$/,'')}
};
function encCred(c){return{id:c.id,rawId:b64.fromBuf(c.rawId),type:c.type,response:{clientDataJSON:b64.fromBuf(c.response.clientDataJSON),attestationObject:b64.fromBuf(c.response.attestationObject),transports:c.response.getTransports?c.response.getTransports():undefined},clientExtensionResults:c.getClientExtensionResults()}}
function encAssertion(c){return{id:c.id,rawId:b64.fromBuf(c.rawId),type:c.type,response:{clientDataJSON:b64.fromBuf(c.response.clientDataJSON),authenticatorData:b64.fromBuf(c.response.authenticatorData),signature:b64.fromBuf(c.response.signature),userHandle:c.response.userHandle?b64.fromBuf(c.response.userHandle):null},clientExtensionResults:c.getClientExtensionResults()}}
function fail(e){const el=document.getElementById('err');if(el){el.textContent=e&&e.message?e.message:String(e);el.hidden=false}document.getElementById('go').disabled=false}
async function post(url,body){const r=await fetch(url,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body||{})});if(!r.ok)throw new Error(await r.text()||('HTTP '+r.status));return r.json()}
"#;

const SETUP_JS: &str = r#"
async function setup(){
  const go=document.getElementById('go');go.disabled=true;
  try{
    const email=document.getElementById('email').value.trim();
    const name=document.getElementById('name').value.trim();
    const team=document.getElementById('team').value.trim()||'Personal';
    const opts=await post('/auth/setup/start',{email,name,team});
    const id=opts.id;const pk=opts.publicKey||opts;
    pk.challenge=b64.toBuf(pk.challenge);
    pk.user.id=b64.toBuf(pk.user.id);
    (pk.excludeCredentials||[]).forEach(c=>c.id=b64.toBuf(c.id));
    const cred=await navigator.credentials.create({publicKey:pk});
    await post('/auth/setup/finish',{id,credential:encCred(cred)});
    location.href='/dashboard';
  }catch(e){fail(e)}
}
"#;

const LOGIN_JS: &str = r#"
async function login(){
  const go=document.getElementById('go');go.disabled=true;
  try{
    const opts=await post('/auth/login/start',{});
    if(opts.setup){location.href='/';return}
    const id=opts.id;const pk=opts.publicKey||opts;
    pk.challenge=b64.toBuf(pk.challenge);
    (pk.allowCredentials||[]).forEach(c=>c.id=b64.toBuf(c.id));
    const cred=await navigator.credentials.get({publicKey:pk});
    await post('/auth/login/finish',{id,credential:encAssertion(cred)});
    location.href='/dashboard';
  }catch(e){fail(e)}
}
"#;

const JOIN_JS: &str = r#"
async function join(){
  const go=document.getElementById('go');go.disabled=true;
  try{
    const token=document.getElementById('token').value;
    const email=document.getElementById('email').value.trim();
    const name=document.getElementById('name').value.trim();
    const opts=await post('/auth/join/start',{token,email,name});
    const id=opts.id;const pk=opts.publicKey||opts;
    pk.challenge=b64.toBuf(pk.challenge);
    pk.user.id=b64.toBuf(pk.user.id);
    (pk.excludeCredentials||[]).forEach(c=>c.id=b64.toBuf(c.id));
    const cred=await navigator.credentials.create({publicKey:pk});
    await post('/auth/setup/finish',{id,credential:encCred(cred)});
    location.href='/dashboard';
  }catch(e){fail(e)}
}
"#;

fn err_box() -> &'static str {
    "<div id=err class=error hidden></div>"
}

pub fn page_setup() -> Response<Body> {
    let inner = format!(
        r#"{}<label for=email>Email</label><input id=email type=email autocomplete=username placeholder="you@example.com" required>
<label for=name>Name</label><input id=name type=text autocomplete=name placeholder="Your name" required>
<label for=team>Team</label><input id=team type=text placeholder="Personal">
<button id=go class="btn primary" onclick=setup() style="width:100%">Create a passkey</button>"#,
        err_box()
    );
    html(auth_page(
        "Create your account",
        "First run: register a passkey to own this instance.",
        &inner,
        &format!("{B64_JS}{SETUP_JS}"),
    ))
}

pub fn page_login() -> Response<Body> {
    let inner = format!(
        r#"{}<p class=small style="margin:0 0 14px">Use the passkey you registered for this instance.</p>
<button id=go class="btn primary" onclick=login() style="width:100%">Sign in with passkey</button>"#,
        err_box()
    );
    html(auth_page(
        "Welcome back",
        "Sign in with a passkey — no password needed.",
        &inner,
        &format!("{B64_JS}{LOGIN_JS}"),
    ))
}

pub fn page_join(token: &str) -> Response<Body> {
    let inner = format!(
        r#"{}<input id=token type=hidden value="{}"><label for=email>Email</label><input id=email type=email autocomplete=username placeholder="you@example.com" required>
<label for=name>Name</label><input id=name type=text autocomplete=name placeholder="Your name" required>
<button id=go class="btn primary" onclick=join() style="width:100%">Join with a passkey</button>"#,
        err_box(),
        esc(token)
    );
    html(auth_page(
        "Join the team",
        "Accept the invite by creating a passkey.",
        &inner,
        &format!("{B64_JS}{JOIN_JS}"),
    ))
}

fn html(body: String) -> Response<Body> {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

fn page_html(status: StatusCode, body: String) -> Response<Body> {
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

fn forbidden(msg: &str) -> Response<Body> {
    page_html(StatusCode::FORBIDDEN, page("Forbidden", &format!("<div class=wrap><h1>Forbidden</h1><p class=muted>{}</p><p><a href=/dashboard>Back to dashboard</a></p></div>", esc(msg))))
}

fn not_found(msg: &str) -> Response<Body> {
    page_html(
        StatusCode::NOT_FOUND,
        page(
            "Not found",
            &format!("<div class=wrap><h1>Not found</h1><p class=muted>{}</p><p><a href=/dashboard>Back to dashboard</a></p></div>", esc(msg)),
        ),
    )
}

fn age(ts: i64) -> String {
    let d = crate::db::now() - ts;
    if d < 45 {
        "just now".into()
    } else if d < 90 {
        "1 min ago".into()
    } else if d < 3600 {
        format!("{} min ago", d / 60)
    } else if d < 7200 {
        "1 hour ago".into()
    } else if d < 86400 {
        format!("{} hours ago", d / 3600)
    } else if d < 172800 {
        "1 day ago".into()
    } else {
        format!("{} days ago", d / 86400)
    }
}

fn status_badge(status: &str) -> String {
    let class = match status {
        "ok" | "ready" => "ok",
        "failed" | "error" => "failed",
        "running" | "building" => "run",
        _ => "",
    };
    format!("<span class=\"badge {class}\">{}</span>", esc(status))
}

fn env_badge(env: &str) -> String {
    let cls = if env == "production" { "env" } else { "" };
    format!("<span class=\"badge {cls}\">{}</span>", esc(env))
}

fn domain_of(st: &AppState, app: &App) -> String {
    app.domain
        .clone()
        .or_else(|| {
            st.base_domain
                .as_ref()
                .map(|b| format!("{}.{}", app.name, b))
        })
        .unwrap_or_else(|| st.public_url.clone())
}

fn csrf_input(session: &Session) -> String {
    format!(
        "<input type=hidden name=csrf value=\"{}\">",
        esc(&session.csrf)
    )
}

fn sidebar(
    st: &AppState,
    db: &Db,
    user: &User,
    teams: &[Team],
    selected: Option<&Team>,
    csrf: &str,
) -> String {
    let mut s = String::from("<aside class=side>");
    s.push_str("<div class=side-title>Teams</div>");
    for t in teams {
        let on = selected.map(|x| x.id == t.id).unwrap_or(false);
        s.push_str(&format!(
            "<a class=\"{}\" href=\"/dashboard?team={}\">{}</a>",
            if on { "on" } else { "" },
            esc(&t.slug),
            esc(&t.name)
        ));
    }
    if let Some(team) = selected {
        s.push_str(&format!(
            "<a href=\"/dashboard/team?team={}\">Members &amp; invites</a>",
            esc(&team.slug)
        ));
        s.push_str("<div class=side-title>Projects</div>");
        let mut apps = db.apps_for_team(&team.id).unwrap_or_default();
        apps.sort_by(|a, b| a.name.cmp(&b.name));
        if apps.is_empty() {
            s.push_str("<span class=muted style=\"padding-left:8px\">No projects yet</span>");
        }
        for a in &apps {
            s.push_str(&format!(
                "<a href=\"/dashboard/project/{}\">{}</a>",
                esc(&a.name),
                esc(&a.name)
            ));
        }
    }
    s.push_str(&format!(
        "<div class=side-title>Account</div><div class=muted style=\"padding:0 8px;margin-left:-8px;font-size:12px\">{}</div>\
<a href=\"#\" onclick=\"fetch('/auth/logout',{{method:'POST'}}).then(()=>location.href='/');return false\">Sign out</a>",
        esc(&user.email)
    ));
    let _ = (st, csrf);
    s.push_str("</aside>");
    s
}

fn layout_with_sidebar(
    st: &AppState,
    db: &Db,
    user: &User,
    teams: &[Team],
    selected: Option<&Team>,
    csrf: &str,
    main: &str,
) -> String {
    let body = format!(
        "<div class=wrap><div class=top><a class=brand href=/dashboard>dok</a><span class=\"muted small\">{} · {}</span></div>\
<div class=layout>{}<main>{}</main></div></div>",
        esc(&user.name),
        esc(selected.map(|t| t.name.as_str()).unwrap_or("")),
        sidebar(st, db, user, teams, selected, csrf),
        main
    );
    page("Dashboard", &body)
}

fn team_and_apps(
    db: &Db,
    user: &User,
    query: &HashMap<String, String>,
) -> (Vec<Team>, Option<Team>, Vec<App>) {
    let teams = db.teams_for(&user.id).unwrap_or_default();
    let selected = query
        .get("team")
        .and_then(|slug| teams.iter().find(|t| &t.slug == slug).cloned())
        .or_else(|| teams.first().cloned());
    let mut apps = match &selected {
        Some(t) => db.apps_for_team(&t.id).unwrap_or_default(),
        None => Vec::new(),
    };
    apps.sort_by(|a, b| a.name.cmp(&b.name));
    (teams, selected, apps)
}

fn project_for(db: &Db, user: &User, name: &str) -> Option<App> {
    let app = db.get_app(name).ok().flatten()?;
    let tid = app.team_id.as_deref()?;
    let role = db.role(tid, &user.id).ok().flatten();
    if crate::db::can_read(role.as_deref()) {
        Some(app)
    } else {
        None
    }
}

fn can_write(db: &Db, user: &User, app: &App) -> bool {
    let Some(tid) = app.team_id.as_deref() else {
        return false;
    };
    let role = db.role(tid, &user.id).ok().flatten();
    crate::db::can_write(role.as_deref())
}

fn render_dashboard(st: &AppState, user: &User, session: &Session, q: &HashMap<String, String>) -> Response<Body> {
    let (teams, selected, apps) = team_and_apps(&st.db, user, q);
    let csrf = csrf_input(session);
    let team_slug = selected.as_ref().map(|t| t.slug.clone()).unwrap_or_default();

    let mut cards = String::new();
    for a in &apps {
        let deploys = st.db.deploys(&a.name, 1).unwrap_or_default();
        let age_txt = deploys
            .first()
            .map(|d| format!("deployed {}", age(d.created_at)))
            .unwrap_or_else(|| "no deploys yet".into());
        cards.push_str(&format!(
            "<a class=card href=\"/dashboard/project/{n}\"><div class=\"row between\"><span class=name>{n}</span>{badge}</div>\
<div class=\"muted small\" style=\"margin:6px 0 10px\">{domain}</div>\
<div class=\"muted small\">{age_txt} · Deployments · logs · rollback</div></a>",
            n = esc(&a.name),
            badge = status_badge(&a.status),
            domain = esc(&domain_of(st, a)),
            age_txt = esc(&age_txt)
        ));
    }
    if cards.is_empty() {
        cards.push_str("<div class=empty><strong>No projects yet</strong><div class=small style=\"margin-top:6px\">Push from the CLI or create one above. Your first deploy appears here.</div></div>");
    }

    let team_name = selected
        .as_ref()
        .map(|t| t.name.clone())
        .unwrap_or_else(|| "Workspace".into());
    let main = format!(
        "<div class=head><div class=\"row between\"><div><h1>{team}</h1><div class=muted>Projects deployed from this team.</div></div>\
<a class=btn href=\"/dashboard/team?team={slug}\">Team</a></div></div>\
<form class=\"row\" method=post action=/dashboard/projects style=\"margin:22px 0 18px;max-width:520px\">{csrf}\
<input name=team type=hidden value=\"{slug}\"><input name=name placeholder=\"new-project\" pattern=\"[a-z0-9][a-z0-9-]*\" maxlength=63 required>\
<button class=\"btn primary\">New project</button></form>\
<div class=grid>{cards}</div>",
        team = esc(&team_name),
        slug = esc(&team_slug),
        csrf = csrf,
        cards = cards
    );
    html(layout_with_sidebar(
        st,
        &st.db,
        user,
        &teams,
        selected.as_ref(),
        &session.csrf,
        &main,
    ))
}

fn tabs(app: &str, active: &str) -> String {
    let items = [
        ("deployments", "Deployments"),
        ("logs", "Logs"),
        ("env", "Environment"),
        ("domains", "Domains"),
        ("proxy", "Proxy"),
        ("settings", "Settings"),
    ];
    let mut s = String::from("<nav class=tabs>");
    for (id, label) in items {
        s.push_str(&format!(
            "<a class=\"{}\" href=\"/dashboard/project/{}/{}\">{}</a>",
            if id == active { "on" } else { "" },
            esc(app),
            id,
            label
        ));
    }
    s.push_str("</nav>");
    s
}

fn cert_table(st: &AppState, app: &App, csrf: &str) -> String {
    use std::collections::HashSet;
    let mut set = HashSet::new();
    if let Some(d) = &app.domain {
        set.insert(d.clone());
    }
    if let Some(b) = &st.base_domain {
        set.insert(format!("{}.{}", app.name, b));
    }
    for d in st.db.domains_for(&app.name).unwrap_or_default() {
        set.insert(d);
    }
    let mut hosts: Vec<_> = set.into_iter().collect();
    hosts.sort();
    let mut rows = String::new();
    for host in hosts {
        let info = crate::api::cert_info(st, &host);
        let status = info["status"].as_str().unwrap_or("pending");
        let cls = match status {
            "issued" => "ok",
            "error" => "failed",
            "expiring" => "run",
            _ => "",
        };
        rows.push_str(&format!(
            "<tr><td><code>{host}</code></td><td><span class=\"badge {cls}\">{status}</span></td><td style=\"text-align:right\">\
<form class=inline method=post action=/dashboard/certificates/renew>{csrf}<input type=hidden name=app value=\"{app}\"><input type=hidden name=host value=\"{host}\">\
<button class=\"btn link\">Renew</button></form></td></tr>",
            host = esc(&host),
            cls = cls,
            status = esc(status),
            csrf = csrf,
            app = esc(&app.name)
        ));
    }
    format!("<table><thead><tr><th>Host</th><th>Status</th><th></th></tr></thead><tbody>{rows}</tbody></table><p class=\"muted small\">Certificates are issued by the built-in edge (<code>dok serve --edge</code>) or your external proxy.</p>")
}

async fn render_project(
    st: &AppState,
    user: &User,
    session: &Session,
    q: &HashMap<String, String>,
    app: &App,
    tab: &str,
) -> Response<Body> {
    let (teams, selected, _) = team_and_apps(&st.db, user, q);
    let selected = app
        .team_id
        .as_ref()
        .and_then(|tid| teams.iter().find(|t| &t.id == tid).cloned())
        .or(selected);
    let csrf = csrf_input(session);
    let can = can_write(&st.db, user, app);
    let mut content = String::new();

    match tab {
        "logs" => {
            let deploys = st.db.deploys(&app.name, 20).unwrap_or_default();
            let selected = q
                .get("deploy")
                .and_then(|id| deploys.iter().find(|d| &d.id == id));
            let env = selected
                .map(|d| d.env.clone())
                .or_else(|| q.get("env").cloned())
                .unwrap_or_else(|| "production".into());
            let build = match selected {
                Some(d) => st
                    .db
                    .deploy_log(&app.name, &d.id)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                None => st
                    .db
                    .last_deploy_log(&app.name)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            };
            let kind = selected
                .map(|d| if d.image.is_some() { "docker" } else { "static" })
                .unwrap_or(app.kind.as_str());
            let container = selected.and_then(|d| d.container.clone());
            let runtime = if kind == "docker" {
                crate::deploy::container_logs(
                    &app.name,
                    &env,
                    container.as_deref(),
                    200,
                )
                .await
            } else {
                "no container (static app)".to_string()
            };
            let stats = crate::deploy::container_stats(&app.name, &env, container.as_deref()).await;
            let cpu = stats["cpu"].as_str().unwrap_or("n/a");
            let mem = stats["memory"].as_str().unwrap_or("n/a");
            let stats_note = stats["note"].as_str().unwrap_or("");
            let mut selector = String::from("<option value=\"\">latest deploy</option>");
            for d in &deploys {
                let short = &d.id[..d.id.len().min(8)];
                selector.push_str(&format!(
                    "<option value=\"{id}\"{sel}>{} · {} · {}</option>",
                    short,
                    d.env,
                    d.status,
                    id = esc(&d.id),
                    sel = if selected.map(|x| x.id == d.id).unwrap_or(false) { " selected" } else { "" }
                ));
            }
            let mut access = String::new();
            for r in st.db.access_logs(&app.name, 50).unwrap_or_default() {
                access.push_str(&format!(
                    "<tr><td class=muted>{}</td><td><code>{}</code> {}</td><td>{}</td><td class=muted>{} ms</td><td class=muted>{}</td></tr>",
                    esc(&age(r.ts)),
                    esc(&r.method),
                    esc(&r.path),
                    esc(&r.status.to_string()),
                    esc(&r.latency_ms.to_string()),
                    esc(r.ip.as_deref().unwrap_or("-"))
                ));
            }
            if access.is_empty() {
                access.push_str("<tr><td colspan=5 class=muted>No proxy requests recorded yet.</td></tr>");
            }
            let deploy_q = selected
                .map(|d| format!("&deploy={}", d.id))
                .unwrap_or_default();
            content.push_str(&format!(
                "<div class=\"row between\" style=\"margin-top:18px\"><h2 style=\"margin:0\">Logs</h2>\
<a class=\"btn\" href=\"/dashboard/stream/{app}?env={env}{deploy_q}\">Live stream</a></div>\
<form class=\"row\" method=get action=\"/dashboard/project/{app}/logs\" style=\"margin:10px 0 16px\">\
<select name=deploy style=\"max-width:360px\">{selector}</select><button class=\"btn\">Show build</button></form>\
<div class=\"muted small\">Runtime: CPU {cpu} · memory {mem}{note}</div>\
<h3>Build log</h3><pre class=log>{build}</pre><h3>Runtime log</h3><pre class=log>{runtime}</pre>\
<h3>Recent requests</h3><table><thead><tr><th>When</th><th>Request</th><th>Status</th><th>Latency</th><th>IP</th></tr></thead><tbody>{access}</tbody></table>",
                app = esc(&app.name),
                env = esc(&env),
                deploy_q = deploy_q,
                selector = selector,
                cpu = esc(cpu),
                mem = esc(mem),
                note = if stats_note.is_empty() { String::new() } else { format!(" ({})", esc(stats_note)) },
                build = esc(&build),
                runtime = esc(&runtime),
                access = access
            ));
        }
        "env" => {
            let env = q.get("env").cloned().unwrap_or_else(|| "production".into());
            let vars = st.db.env(&app.name, &env).unwrap_or_default();
            let mut rows = String::new();
            let mut text = String::new();
            for (k, v, secret) in &vars {
                text.push_str(&format!("{k}={v}\n"));
                rows.push_str(&format!(
                    "<tr><td><code>{}</code></td><td class=muted>{}</td><td>{}</td><td style=\"text-align:right\">\
<form class=inline method=post action=\"/dashboard/env/{}/delete\">{}<input type=hidden name=env value=\"{}\"><input type=hidden name=key value=\"{}\">\
<button class=\"btn link danger\">Remove</button></form></td></tr>",
                    esc(k),
                    if *secret { "••••••••".to_string() } else { esc(v) },
                    if *secret { "<span class=\"badge\">secret</span>" } else { "" },
                    esc(&app.name),
                    csrf,
                    esc(&env),
                    esc(k)
                ));
            }
            if rows.is_empty() {
                rows.push_str("<tr><td colspan=4 class=muted>No variables in this environment.</td></tr>");
            }
            let mut switches = String::new();
            for e in ["production", "preview", "development"] {
                switches.push_str(&format!(
                    "<a class=\"btn\" href=\"/dashboard/project/{}/env?env={}\">{}</a> ",
                    esc(&app.name),
                    e,
                    e
                ));
            }
            content.push_str(&format!(
                "<div class=\"row between\" style=\"margin-top:18px\"><h2 style=\"margin:0\">Environment · {env}</h2><div>{switches}</div></div>\
<div class=\"muted small\" style=\"margin-bottom:12px\">Production values are inherited by preview and development; the more specific environment overrides them.</div>\
<table><thead><tr><th>Key</th><th>Value</th><th></th><th></th></tr></thead><tbody>{rows}</tbody></table>\
<h2>Set variables</h2><form method=post action=\"/dashboard/env/{app}\" class=form>{csrf}\
<input type=hidden name=env value=\"{env}\"><label>KEY=VALUE, one per line</label><textarea name=vars placeholder=\"DATABASE_URL=postgres://...\">{text}</textarea>\
<button class=\"btn primary\">Save {env}</button></form>",
                env = esc(&env),
                switches = switches,
                rows = rows,
                app = esc(&app.name),
                csrf = csrf,
                text = esc(&text)
            ));
        }
        "domains" => {
            let domains = st.db.domains_for(&app.name).unwrap_or_default();
            let mut rows = String::new();
            for d in &domains {
                rows.push_str(&format!(
                    "<tr><td><code>{}</code></td><td style=\"text-align:right\">\
<form class=inline method=post action=\"/dashboard/domains/{}/remove\">{}<input type=hidden name=host value=\"{}\">\
<button class=\"btn link danger\">Remove</button></form></td></tr>",
                    esc(d),
                    esc(&app.name),
                    csrf,
                    esc(d)
                ));
            }
            if rows.is_empty() {
                rows.push_str("<tr><td colspan=2 class=muted>No custom domains. The default hostname below already works.</td></tr>");
            }
            content.push_str(&format!(
                "<h2 style=\"margin-top:18px\">Domains</h2>\
<div class=muted small style=\"margin-bottom:12px\">Default: <code>{default}</code>. Point DNS at this host before adding a custom domain.</div>\
<table><tbody>{rows}</tbody></table>\
<form class=\"row\" method=post action=\"/dashboard/domains/{app}\" style=\"margin-top:18px;max-width:520px\">{csrf}\
<input name=host placeholder=\"app.example.com\" required><button class=\"btn primary\">Add domain</button></form>\
<h2 style=\"margin-top:36px\">Certificates</h2>{certs}",
                default = esc(&domain_of(st, app)),
                rows = rows,
                app = esc(&app.name),
                csrf = csrf,
                certs = cert_table(st, app, &csrf)
            ));
        }
        "proxy" => {
            let s = st.db.proxy_settings(&app.name);
            let headers = s
                .headers
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n");
            let redirect = s.redirect.as_deref().unwrap_or("off");
            let mut opts = String::new();
            for (v, label) in [
                ("off", "No redirect"),
                ("www-to-apex", "www → apex"),
                ("apex-to-www", "apex → www"),
            ] {
                opts.push_str(&format!(
                    "<option value=\"{v}\"{sel}>{label}</option>",
                    v = v,
                    sel = if redirect == v { " selected" } else { "" },
                    label = label
                ));
            }
            content.push_str(&format!(
                "<h2 style=\"margin-top:18px\">Proxy</h2>\
<div class=\"muted small\" style=\"margin-bottom:12px\">Applied by the built-in edge/proxy on every request to this project.</div>\
<form method=post action=\"/dashboard/proxy/{app}\" class=form>{csrf}\
<label>Custom response headers (Name: value, one per line)</label>\
<textarea name=headers placeholder=\"X-Frame-Options: DENY\nContent-Security-Policy: default-src 'self'\">{headers}</textarea>\
<label>Basic auth user</label><input name=basic_auth_user value=\"{user}\" placeholder=\"leave blank to disable\">\
<label>Basic auth password</label><input name=basic_auth_password type=password placeholder=\"{pw}\">\
<label>www / apex redirect</label><select name=redirect>{opts}</select>\
<div class=\"row\" style=\"margin-top:12px\"><label style=\"margin:0\"><input type=checkbox name=compression value=1{comp} style=\"width:auto\"> Compress static text</label>\
<label style=\"margin:0\"><input type=checkbox name=force_https value=1{https} style=\"width:auto\"> Force HTTPS</label></div>\
<label>Max request body (MB, 0 = unlimited)</label><input name=max_body_mb type=number min=0 max=512 value=\"{max}\">\
<button class=\"btn primary\">Save proxy settings</button></form>",
                app = esc(&app.name),
                csrf = csrf,
                headers = esc(&headers),
                user = esc(s.basic_auth_user.as_deref().unwrap_or("")),
                pw = if s.basic_auth_hash.is_some() { "configured — leave blank to keep" } else { "not set" },
                opts = opts,
                comp = if s.compression { " checked" } else { "" },
                https = if s.force_https { " checked" } else { "" },
                max = s.max_body_mb.unwrap_or(0)
            ));
        }
        "settings" => {
            content.push_str(&format!(
                "<h2 style=\"margin-top:18px\">Settings</h2>\
<form method=post action=\"/dashboard/settings/{app}\" class=form>{csrf}\
<label>Git repository</label><input name=repo value=\"{repo}\" placeholder=\"https://github.com/you/app.git\">\
<label>Framework</label><input name=framework value=\"{framework}\" placeholder=\"next, vite, static…\">\
<button class=\"btn primary\">Save settings</button></form>\
<h2 style=\"margin-top:36px\">Danger zone</h2>\
<form method=post action=\"/dashboard/delete/{app}\" onsubmit=\"return confirm('Delete {app}? This cannot be undone.')\">{csrf}\
<button class=\"btn danger\">Delete project</button></form>",
                app = esc(&app.name),
                csrf = csrf,
                repo = esc(app.repo.as_deref().unwrap_or("")),
                framework = esc(app.framework.as_deref().unwrap_or(""))
            ));
        }
        _ => {
            let deploys = st.db.deploys(&app.name, 20).unwrap_or_default();
            let mut rows = String::new();
            for d in &deploys {
                let short = if d.id.len() > 10 { &d.id[..10] } else { &d.id };
                let mut actions = String::new();
                if can && d.status == "ok" {
                    actions.push_str(&format!(
                        "<form class=inline method=post action=\"/dashboard/promote/{}/{}\">{}<button class=\"btn link\">Promote</button></form> ",
                        esc(&app.name),
                        esc(&d.id),
                        csrf
                    ));
                }
                if can && d.env == "production" && d.status == "ok" && app.production_deploy.as_deref() != Some(d.id.as_str()) {
                    actions.push_str(&format!(
                        "<form class=inline method=post action=\"/dashboard/rollback/{}/{}\">{}<button class=\"btn link\">Rollback</button></form>",
                        esc(&app.name),
                        esc(&d.id),
                        csrf
                    ));
                }
                let mut meta = esc(d.author.as_deref().unwrap_or("token"));
                if let Some(sha) = &d.commit_sha {
                    let s8 = &sha[..sha.len().min(7)];
                    let msg = d.commit_message.as_deref().unwrap_or("");
                    meta.push_str(&format!(
                        " · <code>{}</code> {}",
                        esc(s8),
                        esc(&msg.chars().take(60).collect::<String>())
                    ));
                }
                if let Some(url) = &d.url {
                    meta.push_str(&format!(
                        "<br><a class=small href=\"{}\" target=_blank rel=noreferrer>{}</a>",
                        esc(url),
                        esc(url.trim_start_matches("https://").trim_start_matches("http://"))
                    ));
                }
                rows.push_str(&format!(
                    "<tr><td><code>{short}</code><div class=\"muted small\">{meta}</div></td><td>{env}</td><td>{status}</td><td class=muted>{age}</td><td style=\"text-align:right\">{actions}</td></tr>",
                    short = esc(short),
                    meta = meta,
                    env = env_badge(&d.env),
                    status = status_badge(&d.status),
                    age = esc(&age(d.created_at)),
                    actions = actions
                ));
            }
            if rows.is_empty() {
                rows.push_str("<tr><td colspan=5 class=muted>No deployments yet. Run <code>dok push</code> to create one.</td></tr>");
            }
            let redeploy = if can {
                format!(
                    "<form class=inline method=post action=\"/dashboard/redeploy/{}\">{}<button class=\"btn\">Redeploy from git</button></form>",
                    esc(&app.name),
                    csrf
                )
            } else {
                String::new()
            };
            content.push_str(&format!(
                "<div class=\"row between\" style=\"margin-top:18px\"><h2 style=\"margin:0\">Deployments</h2><div>{redeploy}</div></div>\
<table><thead><tr><th>Deploy</th><th>Env</th><th>Status</th><th>Age</th><th></th></tr></thead><tbody>{rows}</tbody></table>",
                redeploy = redeploy,
                rows = rows
            ));
        }
    }

    let main = format!(
        "<div class=head><div class=\"row between\"><div><h1>{n}</h1><div class=muted>{domain}</div></div><div class=row>{badge}{prod}</div></div></div>\
{tabs}{content}",
        n = esc(&app.name),
        domain = esc(&domain_of(st, app)),
        badge = status_badge(&app.status),
        prod = app
            .production_deploy
            .as_ref()
            .map(|d| format!("<span class=muted small>prod {}</span>", esc(&d[..d.len().min(10)])))
            .unwrap_or_default(),
        tabs = tabs(&app.name, tab),
        content = content
    );
    html(layout_with_sidebar(
        st,
        &st.db,
        user,
        &teams,
        selected.as_ref(),
        &session.csrf,
        &main,
    ))
}

fn render_team(
    st: &AppState,
    user: &User,
    session: &Session,
    q: &HashMap<String, String>,
    invite: Option<String>,
) -> Response<Body> {
    let (teams, selected, _) = team_and_apps(&st.db, user, q);
    let Some(team) = selected else {
        return not_found("You are not a member of any team yet.");
    };
    let members = st.db.members(&team.id).unwrap_or_default();
    let mut rows = String::new();
    for (u, role) in &members {
        rows.push_str(&format!(
            "<tr><td><strong>{}</strong><div class=\"muted small\">{}</div></td><td><span class=badge>{}</span></td><td class=muted>{}</td></tr>",
            esc(&u.name),
            esc(&u.email),
            esc(role),
            if u.id == user.id { "you" } else { "" }
        ));
    }
    let notice = invite
        .map(|t| {
            format!(
                "<div class=notice>Invite link (valid 7 days):<br><span class=invite>{}/join/{}</span></div>",
                esc(st.public_url.trim_end_matches('/')),
                esc(&t)
            )
        })
        .unwrap_or_default();
    let actor_role = st.db.role(&team.id, &user.id).ok().flatten();
    let can_invite = crate::db::can_write(actor_role.as_deref());
    let mut role_options = String::new();
    for r in ["member", "viewer"] {
        role_options.push_str(&format!("<option>{r}</option>"));
    }
    if actor_role.as_deref() == Some("owner") {
        role_options.push_str("<option>admin</option><option>owner</option>");
    }
    let invite_form = if can_invite {
        format!(
            "<h2>Invite a teammate</h2><form class=\"row\" method=post action=/dashboard/team/invite style=\"max-width:520px\">{csrf}<input type=hidden name=team value=\"{slug}\">\
<select name=role style=\"max-width:160px\">{options}</select>\
<button class=\"btn primary\">Create invite link</button></form>",
            csrf = csrf_input(session),
            slug = esc(&team.slug),
            options = role_options
        )
    } else {
        "<p class=muted>Only owners and admins can invite members.</p>".into()
    };
    let main = format!(
        "<div class=head><h1>{}</h1><div class=muted>Members and invite links.</div></div>{notice}\
<h2>Members</h2><table><thead><tr><th>User</th><th>Role</th><th></th></tr></thead><tbody>{}</tbody></table>{}",
        esc(&team.name),
        rows,
        invite_form
    );
    html(layout_with_sidebar(
        st,
        &st.db,
        user,
        &teams,
        Some(&team),
        &session.csrf,
        &main,
    ))
}

async fn read_form(req: Request<Body>) -> Result<HashMap<String, String>, Response<Body>> {
    let bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
        .await
        .map_err(|_| forbidden("body too large"))?;
    serde_urlencoded::from_bytes::<HashMap<String, String>>(&bytes)
        .map_err(|_| forbidden("bad form body"))
}

fn check_csrf(session: &Session, form: &HashMap<String, String>) -> Result<(), Response<Body>> {
    match form.get("csrf") {
        Some(v) if v == &session.csrf => Ok(()),
        _ => Err(forbidden("invalid CSRF token")),
    }
}

fn parse_vars(text: &str) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            if !k.is_empty() {
                out.push((k.to_string(), v.trim().to_string(), false));
            }
        }
    }
    out
}

fn create_project(
    st: &AppState,
    user: &User,
    form: &HashMap<String, String>,
) -> Result<(), Response<Body>> {
    let name = form.get("name").map(|s| s.trim().to_string()).unwrap_or_default();
    if !crate::api::valid_name(&name) {
        return Err(not_found(
            "Project names must be lowercase letters, digits and dashes (max 63).",
        ));
    }
    if st.db.get_app(&name).ok().flatten().is_some() {
        return Err(forbidden("a project with that name already exists"));
    }
    let team = if let Some(slug) = form.get("team").filter(|s| !s.is_empty()) {
        let t = st
            .db
            .team_by_slug(slug)
            .ok()
            .flatten()
            .ok_or_else(|| not_found("no such team"))?;
        let role = st.db.role(&t.id, &user.id).ok().flatten();
        if !crate::db::can_write(role.as_deref()) {
            return Err(forbidden("you cannot create projects in this team"));
        }
        Some(t)
    } else {
        st.db
            .teams_for(&user.id)
            .unwrap_or_default()
            .into_iter()
            .find(|t| {
                let role = st.db.role(&t.id, &user.id).ok().flatten();
                crate::db::can_write(role.as_deref())
            })
    };
    let Some(team) = team else {
        return Err(forbidden("you are not an owner or admin of any team"));
    };
    let app = App {
        name: name.clone(),
        team_id: Some(team.id),
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
    st.db
        .upsert_app(&app)
        .map_err(|e| crate::ui::server_error(&e.to_string()))?;
    st.sync_traefik();
    Ok(())
}

fn server_error(msg: &str) -> Response<Body> {
    page_html(
        StatusCode::INTERNAL_SERVER_ERROR,
        page(
            "Error",
            &format!("<div class=wrap><h1>Something went wrong</h1><pre class=log>{}</pre><p><a href=/dashboard>Back</a></p></div>", esc(msg)),
        ),
    )
}

pub async fn handle(
    st: &AppState,
    _auth: &AuthState,
    req: Request<Body>,
    session: Option<Session>,
) -> Option<Response<Body>> {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let query: HashMap<String, String> = req
        .uri()
        .query()
        .and_then(|q| serde_urlencoded::from_str(q).ok())
        .unwrap_or_default();

    let is_ui = path == "/dashboard" || path.starts_with("/dashboard/");
    if !is_ui {
        return None;
    }
    let Some(session) = session else {
        return Some(auth::redirect("/"));
    };
    let user = session.user.clone();

    if method == Method::GET {
        if let Some(rest) = path.strip_prefix("/dashboard/stream/") {
            let app_name = rest.split('/').next().unwrap_or("");
            let Some(app) = project_for(&st.db, &user, app_name) else {
                return Some(not_found("project not found in your teams"));
            };
            let deploy = query
                .get("deploy")
                .and_then(|id| st.db.deploy(&app.name, id).ok().flatten());
            let env = deploy
                .as_ref()
                .map(|d| d.env.clone())
                .or_else(|| query.get("env").cloned())
                .unwrap_or_else(|| "production".into());
            let kind = deploy
                .as_ref()
                .map(|d| if d.image.is_some() { "docker" } else { "static" })
                .unwrap_or(app.kind.as_str());
            let container = deploy.as_ref().and_then(|d| d.container.clone());
            return Some(
                crate::api::log_stream_response(
                    st,
                    &app.name,
                    &env,
                    kind,
                    container.as_deref(),
                )
                .await,
            );
        }
        if path == "/dashboard" || path == "/dashboard/" {
            return Some(render_dashboard(st, &user, &session, &query));
        }
        if path == "/dashboard/team" {
            return Some(render_team(st, &user, &session, &query, None));
        }
        let rest = path.trim_start_matches("/dashboard/");
        let mut parts = rest.splitn(2, '/');
        let first = parts.next().unwrap_or("");
        let second = parts.next().unwrap_or("");
        let (app_name, tab) = match first {
            "project" => {
                let mut it = second.splitn(2, '/');
                (
                    it.next().unwrap_or("").to_string(),
                    it.next().unwrap_or("deployments").to_string(),
                )
            }
            "deployments" | "logs" | "env" | "domains" | "proxy" | "settings" => {
                (second.to_string(), first.to_string())
            }
            "app" => (second.to_string(), "deployments".to_string()),
            _ => return Some(not_found("unknown dashboard page")),
        };
        if app_name.is_empty() {
            return Some(not_found("unknown dashboard page"));
        }
        let Some(app) = project_for(&st.db, &user, &app_name) else {
            return Some(not_found("project not found in your teams"));
        };
        let tab = match tab.as_str() {
            "deployments" | "logs" | "env" | "domains" | "proxy" | "settings" => tab.as_str(),
            _ => "deployments",
        };
        return Some(render_project(st, &user, &session, &query, &app, tab).await);
    }

    if method != Method::POST {
        return Some(page_html(
            StatusCode::METHOD_NOT_ALLOWED,
            page("Method not allowed", "<div class=wrap><h1>Method not allowed</h1></div>"),
        ));
    }

    // All dashboard writes are CSRF-protected.
    let rest = path.trim_start_matches("/dashboard/").to_string();
    let form = match read_form(req).await {
        Ok(f) => f,
        Err(r) => return Some(r),
    };
    if let Err(r) = check_csrf(&session, &form) {
        return Some(r);
    }

    if rest == "projects" {
        return match create_project(st, &user, &form) {
            Ok(_) => Some(auth::redirect(&format!(
                "/dashboard?team={}",
                form.get("team").cloned().unwrap_or_default()
            ))),
            Err(r) => Some(r),
        };
    }

    if rest == "team/invite" {
        let Some(team) = form
            .get("team")
            .and_then(|slug| st.db.team_by_slug(slug).ok().flatten())
        else {
            return Some(not_found("no such team"));
        };
        let role = form.get("role").cloned().unwrap_or_else(|| "member".into());
        let role = match role.as_str() {
            "admin" | "viewer" | "member" => role,
            _ => "member".into(),
        };
        let actor_role = st.db.role(&team.id, &user.id).ok().flatten();
        if !crate::db::can_write(actor_role.as_deref()) {
            return Some(forbidden("only owners and admins can invite"));
        }
        if !crate::db::can_grant(actor_role.as_deref(), &role) {
            return Some(forbidden("you cannot grant that role"));
        }
        return match st.db.create_invite(&team.id, &role, 7 * 86400) {
            Ok(token) => {
                let mut ctx = query.clone();
                ctx.insert("team".into(), team.slug.clone());
                Some(render_team(st, &user, &session, &ctx, Some(token)))
            }
            Err(e) => Some(server_error(&e.to_string())),
        };
    }

    if rest == "certificates/renew" {
        let host = form.get("host").cloned().unwrap_or_default();
        let app_name = form.get("app").cloned().unwrap_or_default();
        let Some(app) = project_for(&st.db, &user, &app_name) else {
            return Some(not_found("project not found in your teams"));
        };
        if !can_write(&st.db, &user, &app) {
            return Some(forbidden("you need owner or admin rights for this project"));
        }
        if !crate::api::valid_host(&host) {
            return Some(not_found("invalid host"));
        }
        let value = serde_json::json!({
            "status": "pending",
            "requested_at": crate::db::now(),
            "expires": null
        });
        if let Err(e) = st.db.set_setting(&format!("cert:{host}"), &value.to_string()) {
            return Some(server_error(&e.to_string()));
        }
        return Some(auth::redirect(&format!(
            "/dashboard/project/{}/domains",
            app.name
        )));
    }

    let mut parts = rest.splitn(3, '/');
    let action = parts.next().unwrap_or("");
    let app_name = parts.next().unwrap_or("").to_string();
    let tail = parts.next().unwrap_or("").to_string();

    let Some(app) = project_for(&st.db, &user, &app_name) else {
        return Some(not_found("project not found in your teams"));
    };
    if !can_write(&st.db, &user, &app) {
        return Some(forbidden("you need owner or admin rights for this project"));
    }

    match action {
        "redeploy" => match crate::api::deploy_from_repo(st, &app.name).await {
            Ok(_) => Some(auth::redirect(&format!(
                "/dashboard/project/{}/deployments",
                app.name
            ))),
            Err(e) => Some(server_error(&e.to_string())),
        },
        "promote" | "rollback" => {
            if tail.is_empty() {
                return Some(not_found("missing deploy id"));
            }
            match crate::deploy::promote(st, &app.name, &tail, Some(&user.email)).await {
                Ok(_) => Some(auth::redirect(&format!(
                    "/dashboard/project/{}/deployments",
                    app.name
                ))),
                Err(e) => Some(server_error(&e.to_string())),
            }
        }
        "env" => {
            let env = form
                .get("env")
                .cloned()
                .unwrap_or_else(|| "production".into());
            if !matches!(env.as_str(), "production" | "preview" | "development") {
                return Some(not_found("unknown environment"));
            }
            if tail == "delete" {
                let key = form.get("key").cloned().unwrap_or_default();
                let _ = st.db.delete_env(&app.name, &env, &key);
            } else {
                let vars = parse_vars(form.get("vars").map(String::as_str).unwrap_or(""));
                for (k, _, _) in st.db.env(&app.name, &env).unwrap_or_default() {
                    if !vars.iter().any(|(nk, _, _)| nk == &k) {
                        let _ = st.db.delete_env(&app.name, &env, &k);
                    }
                }
                let _ = st.db.put_env(&app.name, &env, &vars);
            }
            Some(auth::redirect(&format!(
                "/dashboard/project/{}/env?env={}",
                app.name, env
            )))
        }
        "domains" => {
            if tail == "remove" {
                let host = form.get("host").cloned().unwrap_or_default();
                match st.db.remove_domain(&app.name, &host) {
                    Ok(0) => return Some(not_found("domain not found on this project")),
                    Ok(_) => st.sync_traefik(),
                    Err(e) => return Some(server_error(&e.to_string())),
                }
            } else {
                let host = form
                    .get("host")
                    .map(|s| s.trim().trim_end_matches('.').to_ascii_lowercase())
                    .unwrap_or_default();
                if !crate::api::valid_host(&host) {
                    return Some(not_found("invalid domain (needs a dot)"));
                }
                if st.reserved_host(&host) || st.is_base_subdomain(&host) {
                    return Some(forbidden("host is reserved for the dashboard"));
                }
                if let Ok(Some(other)) = st.db.domain_owner(&host) {
                    if other != app.name {
                        return Some(forbidden("domain belongs to another project"));
                    }
                }
                let _ = st.db.add_domain(&app.name, &host);
                st.sync_traefik();
            }
            Some(auth::redirect(&format!(
                "/dashboard/project/{}/domains",
                app.name
            )))
        }
        "proxy" => {
            let mut ps = st.db.proxy_settings(&app.name);
            ps.headers.clear();
            if let Some(text) = form.get("headers") {
                for line in text.lines().take(20) {
                    if let Some((k, v)) = line.split_once(':') {
                        let k = k.trim();
                        let v = v.trim();
                        if crate::api::safe_header(k) && !v.is_empty() {
                            ps.headers.push((k.to_string(), v.to_string()));
                        }
                    }
                }
            }
            ps.basic_auth_user = form
                .get("basic_auth_user")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if let Some(pw) = form.get("basic_auth_password").filter(|p| !p.is_empty()) {
                ps.basic_auth_hash = Some(crate::db::hash_password(pw));
            }
            if ps.basic_auth_user.is_none() {
                ps.basic_auth_hash = None;
            }
            ps.redirect = Some(
                form.get("redirect")
                    .map(String::as_str)
                    .filter(|r| matches!(*r, "www-to-apex" | "apex-to-www"))
                    .unwrap_or("off")
                    .to_string(),
            );
            ps.compression = form.contains_key("compression");
            ps.force_https = form.contains_key("force_https");
            ps.max_body_mb = form
                .get("max_body_mb")
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| *v > 0)
                .map(|v| v.min(512));
            if let Err(e) = st.db.set_proxy_settings(&app.name, &ps) {
                return Some(server_error(&e.to_string()));
            }
            Some(auth::redirect(&format!(
                "/dashboard/project/{}/proxy",
                app.name
            )))
        }
        "settings" => {
            let repo = form.get("repo").cloned().unwrap_or_default();
            let framework = form.get("framework").cloned().unwrap_or_default();
            let mut row = app.clone();
            row.repo = if repo.is_empty() { None } else { Some(repo) };
            row.framework = if framework.is_empty() {
                None
            } else {
                Some(framework)
            };
            row.updated_at = crate::db::now();
            let _ = st.db.upsert_app(&row);
            Some(auth::redirect(&format!(
                "/dashboard/project/{}/settings",
                app.name
            )))
        }
        "delete" => {
            crate::deploy::stop_container(&app.name, "production").await;
            crate::deploy::stop_container(&app.name, "preview").await;
            crate::deploy::stop_all_previews(&app.name).await;
            let _ = tokio::fs::remove_dir_all(st.builds_dir().join(&app.name)).await;
            if let Err(e) = st.db.delete_app(&app.name) {
                return Some(server_error(&e.to_string()));
            }
            st.sync_traefik();
            Some(auth::redirect("/dashboard"))
        }
        _ => Some(not_found("unknown action")),
    }
}

#[cfg(test)]
mod tests {
    use super::{age, parse_vars};

    #[test]
    fn parses_env_lines() {
        let v = parse_vars("# comment\nA=1\n B = two words \nbadline\n\nC=\n");
        assert_eq!(
            v,
            vec![
                ("A".into(), "1".into(), false),
                ("B".into(), "two words".into(), false),
                ("C".into(), "".into(), false),
            ]
        );
    }

    #[tokio::test]
    async fn dashboard_posts_require_csrf() {
        let path = std::env::temp_dir().join(format!("dok-ui-{}.db", uuid::Uuid::new_v4()));
        let db = crate::db::Db::open(&path).unwrap();
        let u = db.create_user("ui@example.com", "UI").unwrap();
        let t = db.create_team("UI", "ui", &u.id).unwrap();
        db.upsert_app(&crate::db::App {
            name: "app".into(),
            team_id: Some(t.id),
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
        let data = std::env::temp_dir().join(format!("dok-ui-data-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data).unwrap();
        let st = crate::state::AppState::new(db, data, None, "http://localhost".into());
        let auth = crate::auth::AuthState::new(url::Url::parse("http://localhost").unwrap()).unwrap();
        let make = |csrf: &str| {
            axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri("/dashboard/settings/app")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(axum::body::Body::from(format!(
                    "csrf={csrf}&repo=https://example.com/r.git"
                )))
                .unwrap()
        };
        let bad = super::handle(
            &st,
            &auth,
            make("nope"),
            Some(crate::auth::Session {
                user: u.clone(),
                csrf: "good".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(bad.status(), axum::http::StatusCode::FORBIDDEN);
        let good = super::handle(
            &st,
            &auth,
            make("good"),
            Some(crate::auth::Session {
                user: u.clone(),
                csrf: "good".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(good.status(), axum::http::StatusCode::SEE_OTHER);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn humanises_age() {
        let now = crate::db::now();
        assert_eq!(age(now), "just now");
        assert_eq!(age(now - 120), "2 min ago");
        assert_eq!(age(now - 7200), "2 hours ago");
        assert_eq!(age(now - 3 * 86400), "3 days ago");
    }
}
