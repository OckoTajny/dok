//! Server-rendered dashboard. No JS framework; only the passkey ceremony uses
//! a small inline script.

use crate::auth::{self, AuthState, Session};
use crate::db::{App, Db, Team, User};
use crate::state::AppState;
use axum::body::Body;
use axum::http::{header, Method, Request, Response, StatusCode};
use axum::response::IntoResponse;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set by `server::run` when the public origin is HTTPS so one-time and flash
/// cookies carry `Secure` too.
pub static SECURE_COOKIES: AtomicBool = AtomicBool::new(false);

fn cookie_secure() -> &'static str {
    if SECURE_COOKIES.load(Ordering::Relaxed) {
        "; Secure"
    } else {
        ""
    }
}

const CSS: &str = r#"
/* Obsidian precision: graphite surfaces, amber signal, hairline structure. */
:root{
 color-scheme:dark;
 --bg:#08080a;
 --surface:#0d0d11;
 --surface-2:#101015;
 --raised:#15151b;
 --raised-2:#1d1d25;
 --line:#23232b;
 --line-soft:#191920;
 --line-strong:#34343f;
 --ink:#f4f3ef;
 --ink-2:#d8d7d1;
 --muted:#95959f;
 --faint:#7f7f8a;
 --accent:#ffb454;
 --accent-hi:#ffc678;
 --accent-ink:#1a1000;
 --accent-soft:rgba(255,180,84,.12);
 --accent-line:rgba(255,180,84,.38);
 --danger:#ff7264;
 --danger-soft:rgba(255,114,100,.1);
 --success:#7fe0a0;
 --success-soft:rgba(127,224,160,.09);
 --sans:-apple-system,BlinkMacSystemFont,"Segoe UI",Inter,"Helvetica Neue",system-ui,sans-serif;
 --mono:ui-monospace,SFMono-Regular,"SF Mono",Menlo,Consolas,"Liberation Mono",monospace;
 --r:9px;
 --r-lg:14px;
 --shadow:0 24px 60px -34px rgba(0,0,0,.95);
}
*,*::before,*::after{box-sizing:border-box}
html,body{margin:0;padding:0}
html{-webkit-text-size-adjust:100%}
body{
 min-height:100vh;
 background:radial-gradient(1100px 520px at 22% -8%,rgba(255,180,84,.07),transparent 62%),radial-gradient(900px 600px at 100% 0,rgba(255,255,255,.025),transparent 55%),var(--bg);
 color:var(--ink);
 font:16px/1.6 var(--sans);
 -webkit-font-smoothing:antialiased;
 text-rendering:optimizeLegibility;
}
body::before{content:"";position:fixed;inset:0;z-index:0;pointer-events:none;opacity:.028;background-image:url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='120' height='120'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='.85' numOctaves='2' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='120' height='120' filter='url(%23n)'/%3E%3C/svg%3E")}
a{color:var(--ink-2);text-decoration:none;transition:color .15s ease}
a:hover{color:var(--ink)}
code,pre{font-family:var(--mono)}
code{font-size:.92em}
.skip{position:absolute;left:-9999px;top:0;background:var(--accent);color:var(--accent-ink);padding:10px 16px;border-radius:0 0 8px 0;z-index:100;font-weight:600}
.skip:focus{left:0}
:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
::placeholder{color:var(--faint);opacity:1}
::selection{background:rgba(255,180,84,.32);color:#fff}
.wrap{position:relative;z-index:1;max-width:1280px;margin:0 auto;padding:0 32px}

/* ---------- topbar ---------- */
.top{position:sticky;top:0;z-index:40;display:flex;align-items:center;justify-content:space-between;gap:18px;height:60px;margin:0 -32px;padding:0 32px;border-bottom:1px solid var(--line-soft);background:rgba(8,8,10,.78);backdrop-filter:blur(16px) saturate(140%)}
.brand{display:inline-flex;align-items:center;gap:9px;font-weight:650;font-size:16px;letter-spacing:-.025em;color:var(--ink)}
.brand:hover{color:var(--ink);text-decoration:none}
.mark{position:relative;width:18px;height:18px;border-radius:5px;background:var(--accent);box-shadow:0 0 18px -5px rgba(255,180,84,.7);flex:0 0 auto}
.mark::after{content:"";position:absolute;inset:5px;border-radius:1.5px;background:var(--accent-ink)}
.brand.small{font-size:14px}
.brand.small .mark{width:16px;height:16px;border-radius:4px}
.brand.small .mark::after{inset:4.5px}
.top-meta{display:flex;align-items:center;gap:16px;min-width:0}
.top-link{font:10.5px var(--mono);letter-spacing:.1em;text-transform:uppercase;color:var(--muted)}
.top-link:hover{color:var(--ink);text-decoration:none}
.who{display:flex;align-items:center;gap:9px;padding-left:16px;border-left:1px solid var(--line);min-width:0}
.avatar{width:28px;height:28px;border-radius:50%;display:grid;place-items:center;background:var(--raised);border:1px solid var(--line-strong);font:600 11px/1 var(--mono);color:var(--accent);flex:0 0 auto}
.who-text{line-height:1.3;min-width:0}
.who-name{font-size:12.5px;font-weight:600;color:var(--ink);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;max-width:160px}
.who-team{font:9.5px/1.5 var(--mono);letter-spacing:.1em;text-transform:uppercase;color:var(--muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;max-width:160px}

/* ---------- shell ---------- */
.layout{display:grid;grid-template-columns:248px minmax(0,1fr);gap:44px;padding:36px 0 96px;align-items:start}
main{grid-column:2;grid-row:1;min-width:0;animation:rise .45s cubic-bezier(.22,.61,.36,1) both}
aside{grid-column:1;grid-row:1;position:sticky;top:84px;font-size:13.5px;min-width:0}
.side-head{display:none}
.side-title{display:flex;align-items:center;justify-content:space-between;gap:8px;margin:26px 0 8px;padding:0 10px;color:var(--faint);font:10px/1.4 var(--mono);letter-spacing:.16em;text-transform:uppercase}
.side-title:first-of-type{margin-top:0}
.side-count{color:var(--muted);letter-spacing:0;border:1px solid var(--line);border-radius:999px;padding:0 6px;font-size:9.5px;line-height:15px}
.side a{position:relative;display:flex;align-items:center;gap:9px;padding:8px 10px;border-radius:7px;color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;transition:background .15s,color .15s}
.side a:hover{background:rgba(255,255,255,.04);color:var(--ink);text-decoration:none}
.side a.on{background:linear-gradient(90deg,rgba(255,180,84,.14),rgba(255,180,84,.03));color:var(--ink)}
.side a.on::before{content:"";position:absolute;left:0;top:7px;bottom:7px;width:2px;border-radius:2px;background:var(--accent)}
.sdot{width:6px;height:6px;border-radius:50%;background:var(--faint);flex:0 0 auto}
.sdot.ok{background:var(--success);box-shadow:0 0 0 3px rgba(127,224,160,.12)}
.sdot.failed{background:var(--danger);box-shadow:0 0 0 3px rgba(255,114,100,.12)}
.sdot.run{background:var(--accent);box-shadow:0 0 0 3px rgba(255,180,84,.14)}
.side-foot{margin-top:28px;padding-top:18px;border-top:1px solid var(--line-soft)}
.side-user{display:flex;align-items:center;gap:9px;padding:0 10px;margin-bottom:10px;min-width:0}
.side-user .avatar{width:26px;height:26px;font-size:10px}
.side-user div{min-width:0}
.side-user b{display:block;font-size:12.5px;font-weight:600;color:var(--ink-2);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.side-user span{display:block;font:10px/1.5 var(--mono);color:var(--faint);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.side-ver{margin-top:16px;padding:0 10px;font:10px/1.4 var(--mono);color:var(--faint);letter-spacing:.08em}
.side .close{display:none}
.scrim{display:none}
.menu{display:none;border:1px solid var(--line-strong);background:var(--raised);color:var(--ink);border-radius:8px;padding:0 12px;min-height:36px;font:500 12.5px var(--sans);cursor:pointer;align-items:center}
.menu:hover{background:var(--raised-2)}

/* ---------- type ---------- */
h1{font-size:clamp(30px,3.4vw,44px);line-height:1.06;letter-spacing:-.035em;font-weight:600;margin:0 0 10px}
h2{display:flex;align-items:center;gap:10px;font-size:19px;line-height:1.3;letter-spacing:-.02em;font-weight:600;margin:36px 0 14px}
h3{font-size:13.5px;font-weight:600;letter-spacing:-.005em;color:var(--ink-2);margin:18px 0 8px}
.eyebrow{display:inline-flex;align-items:center;gap:9px;font:10.5px/1 var(--mono);letter-spacing:.16em;text-transform:uppercase;color:var(--muted);margin:0 0 14px}
.eyebrow .tick{width:14px;height:1px;background:var(--accent);box-shadow:0 0 8px rgba(255,180,84,.55);flex:0 0 auto}
.lede{color:var(--muted);font-size:15px;line-height:1.6;margin:0;max-width:64ch}
.muted{color:var(--muted)}
.small{font-size:12.5px}
.mono{font-family:var(--mono)}
.head{padding:4px 0 0}
.row{display:flex;align-items:center;gap:10px;flex-wrap:wrap}
.between{justify-content:space-between}
.inline{display:inline}
.page-head{display:flex;align-items:flex-end;justify-content:space-between;gap:28px;flex-wrap:wrap;padding:2px 0 0;margin-bottom:30px}
.page-head h1{margin-bottom:8px}
.head-actions{display:flex;align-items:center;gap:10px;flex-wrap:wrap}

/* ---------- stat strip ---------- */
.stats{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:1px;background:var(--line-soft);border:1px solid var(--line);border-radius:var(--r-lg);overflow:hidden;margin:0 0 36px;box-shadow:var(--shadow)}
.stat{padding:18px 20px 16px;background:linear-gradient(180deg,rgba(255,255,255,.022),transparent 72%),var(--surface);min-width:0;animation:rise .45s cubic-bezier(.22,.61,.36,1) both}
.stat-label{display:flex;align-items:center;gap:6px;font:10px/1.4 var(--mono);letter-spacing:.14em;text-transform:uppercase;color:var(--faint)}
.stat-num{margin-top:9px;font-size:34px;line-height:1;letter-spacing:-.04em;font-weight:600;font-variant-numeric:tabular-nums;color:var(--ink)}
.stat-num em{font-style:normal;font-size:15px;color:var(--muted);letter-spacing:-.01em;margin-left:3px}
.stat-sub{margin-top:7px;font-size:11.5px;line-height:1.5;color:var(--muted)}
.stat.hot .stat-num{color:var(--accent)}
.stat.warn .stat-num{color:var(--danger)}

/* ---------- cards ---------- */
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(286px,1fr));gap:14px}
.grid>.start,.grid>.empty{grid-column:1/-1}
.card{display:block;border:1px solid var(--line);border-radius:var(--r-lg);padding:18px;background:linear-gradient(180deg,rgba(255,255,255,.022),transparent 60%),var(--surface);box-shadow:0 1px 0 rgba(255,255,255,.02) inset}
a.card:hover{border-color:var(--line-strong);text-decoration:none}
.card .name{font-weight:600;font-size:17px;letter-spacing:-.02em;margin-top:6px;font-variant-numeric:tabular-nums}
.pcard{display:flex;flex-direction:column;gap:11px;padding:18px 18px 0;border:1px solid var(--line);border-radius:var(--r-lg);background:linear-gradient(180deg,rgba(255,255,255,.025),transparent 58%),var(--surface);transition:transform .2s ease,border-color .2s ease,box-shadow .2s ease;animation:rise .45s cubic-bezier(.22,.61,.36,1) both}
.pcard:hover{transform:translateY(-2px);border-color:#3b3b47;box-shadow:0 22px 44px -30px #000,0 0 0 1px rgba(255,180,84,.1)}
.pcard-head{display:flex;align-items:center;justify-content:space-between;gap:10px;min-width:0}
.pcard-id{display:flex;align-items:center;gap:9px;min-width:0}
.pcard-name{font-size:16px;font-weight:600;letter-spacing:-.02em;margin:0;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.pcard-name a{color:var(--ink)}
.pcard-host{font:12px/1.5 var(--mono);color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.pcard-host a{color:var(--muted)}
.pcard-host a:hover{color:var(--accent);text-decoration:none}
.pcard-meta{display:flex;align-items:center;gap:8px;flex-wrap:wrap;font-size:11.5px;color:var(--muted)}
.pcard-meta code{font-size:11px;color:var(--ink-2)}
.pcard-foot{display:flex;align-items:center;justify-content:space-between;gap:10px;margin:3px -18px 0;padding:10px 18px;border-top:1px solid var(--line-soft);font-size:11.5px;min-width:0}
.pcard-foot a{color:var(--muted)}
.pcard-foot a:hover{color:var(--ink);text-decoration:none}
.pcard-links{display:flex;gap:14px;flex:0 0 auto}
.pcard-author{font:10px/1.5 var(--mono);color:var(--faint);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.dot{position:relative;width:8px;height:8px;border-radius:50%;background:var(--faint);flex:0 0 auto}
.dot.ok{background:var(--success);box-shadow:0 0 0 3px rgba(127,224,160,.12)}
.dot.failed{background:var(--danger);box-shadow:0 0 0 3px rgba(255,114,100,.12)}
.dot.run{background:var(--accent);box-shadow:0 0 0 3px rgba(255,180,84,.14);animation:pulse 1.8s ease-in-out infinite}
.dot.new{background:var(--faint)}

/* ---------- dashboard layout ---------- */
.dash-grid{display:grid;grid-template-columns:minmax(0,1fr) 300px;gap:24px;align-items:start;margin-top:6px}
.section-head{display:flex;align-items:center;justify-content:space-between;gap:12px;margin:0 0 14px}
.section-head h2{margin:0}
.count{font:10px var(--mono);letter-spacing:.1em;color:var(--faint);border:1px solid var(--line);border-radius:999px;padding:2px 8px}
.rail{position:sticky;top:84px;display:flex;flex-direction:column;gap:16px;min-width:0}
.panel{border:1px solid var(--line);border-radius:var(--r-lg);background:var(--surface);overflow:hidden}
.panel-head{display:flex;align-items:center;justify-content:space-between;gap:8px;padding:13px 16px;border-bottom:1px solid var(--line-soft)}
.panel-head h2{margin:0;font:500 11.5px var(--mono);letter-spacing:.1em;text-transform:uppercase;color:var(--ink-2)}
.panel-head .count{font-size:9.5px}
.timeline{list-style:none;margin:0;padding:6px 0}
.tl{position:relative;display:flex;gap:10px;padding:9px 16px}
.tl+.tl::before{content:"";position:absolute;left:19.5px;top:-10px;height:13px;width:1px;background:var(--line)}
.tl .dot{margin-top:5px}
.tl-body{flex:1;min-width:0}
.tl-title{display:flex;align-items:center;gap:6px;flex-wrap:wrap;font-size:12.5px;color:var(--ink-2)}
.tl-title a{font-weight:600;color:var(--ink)}
.tl-meta{font:10.5px/1.5 var(--mono);color:var(--faint);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.tl-time{font:10px var(--mono);color:var(--faint);white-space:nowrap;margin-top:3px}

/* ---------- empty + start ---------- */
.empty{border:1px dashed var(--line-strong);border-radius:var(--r-lg);padding:38px 22px;text-align:center;color:var(--muted);background:radial-gradient(400px 160px at 50% 0,rgba(255,180,84,.05),transparent 70%)}
.empty strong{display:block;color:var(--ink);font-size:16px;letter-spacing:-.01em}
.empty code{display:inline-block;margin-top:10px;font:12px var(--mono);color:var(--ink-2);background:rgba(255,255,255,.04);border:1px solid var(--line);border-radius:5px;padding:3px 7px}
.start{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1.05fr);gap:30px;align-items:center;border:1px solid var(--line);border-radius:var(--r-lg);padding:28px;background:radial-gradient(420px 200px at 0 0,rgba(255,180,84,.06),transparent 70%),linear-gradient(160deg,rgba(255,255,255,.025),transparent 60%),var(--surface)}
.start h2{display:block;margin:0 0 8px;font-size:22px}
.steps{list-style:none;margin:0;padding:0;counter-reset:s}
.steps li{position:relative;padding:0 0 16px 36px;color:var(--muted);font-size:13px;line-height:1.55}
.steps li::before{counter-increment:s;content:counter(s,decimal-leading-zero);position:absolute;left:0;top:0;font:10px var(--mono);color:var(--accent);border:1px solid var(--accent-line);border-radius:5px;padding:2px 5px;background:var(--accent-soft)}
.steps li:last-child{padding-bottom:0}
.steps strong{color:var(--ink-2);font-weight:500}
.steps code{display:block;width:fit-content;max-width:100%;margin-top:6px;font:11.5px/1.7 var(--mono);color:var(--ink-2);background:rgba(255,255,255,.04);border:1px solid var(--line);border-radius:5px;padding:3px 7px;overflow-wrap:anywhere;white-space:normal}

/* ---------- badges ---------- */
.badge{display:inline-flex;align-items:center;gap:5px;border:1px solid var(--line-strong);border-radius:6px;padding:2px 7px;font:10px/1.6 var(--mono);letter-spacing:.07em;text-transform:uppercase;color:var(--ink-2);background:rgba(255,255,255,.03);white-space:nowrap}
.badge::before{content:"";width:4px;height:4px;border-radius:50%;background:currentColor;opacity:.85}
.badge.ok{color:var(--success);border-color:rgba(127,224,160,.28);background:var(--success-soft)}
.badge.failed{color:var(--danger);border-color:rgba(255,114,100,.3);background:var(--danger-soft)}
.badge.run{color:var(--accent);border-color:var(--accent-line);background:var(--accent-soft)}
.badge.env{color:#a8c8e8;border-color:rgba(130,175,225,.26);background:rgba(130,175,225,.08)}
.badge.new{color:#c9b3f5;border-color:rgba(190,150,255,.26);background:rgba(190,150,255,.08)}

/* ---------- buttons ---------- */
.btn{display:inline-flex;align-items:center;justify-content:center;gap:7px;min-height:36px;padding:0 14px;border:1px solid var(--line-strong);border-radius:8px;background:linear-gradient(180deg,var(--raised-2),var(--raised));color:var(--ink);font:500 13px/1 var(--sans);letter-spacing:-.005em;cursor:pointer;white-space:nowrap;transition:background .15s,border-color .15s,transform .1s,box-shadow .15s}
.btn:hover{background:var(--raised-2);border-color:#43434f;color:var(--ink);text-decoration:none}
.btn:active{transform:translateY(1px)}
.btn.primary{background:var(--accent);border-color:var(--accent);color:var(--accent-ink);font-weight:600;box-shadow:0 10px 26px -16px rgba(255,180,84,.85)}
.btn.primary:hover{background:var(--accent-hi);border-color:var(--accent-hi);color:var(--accent-ink)}
.btn.danger{color:var(--danger);border-color:rgba(255,114,100,.34);background:transparent;box-shadow:none}
.btn.danger:hover{background:var(--danger-soft);border-color:rgba(255,114,100,.55);color:#ff9d92}
.btn:disabled{opacity:.5;cursor:not-allowed;pointer-events:none}
.btn.link{background:none;border:0;padding:0 2px;min-height:0;color:var(--muted);font:12.5px var(--sans);box-shadow:none}
.btn.link:hover{color:var(--ink);text-decoration:underline;background:none}
.btn.link.danger{color:var(--danger)}
.btn.link.danger:hover{color:#ff9d92}

/* ---------- forms ---------- */
input,select,textarea{width:100%;background:var(--surface-2);border:1px solid var(--line-strong);color:var(--ink);border-radius:8px;padding:9px 11px;font:14px/1.5 var(--sans);min-height:40px;transition:border-color .15s,box-shadow .15s,background .15s}
input[type=checkbox]{width:16px;height:16px;min-height:0;accent-color:var(--accent)}
input[type=checkbox]:focus-visible{outline:2px solid var(--accent);outline-offset:2px;box-shadow:none}
select{appearance:none;background-image:linear-gradient(45deg,transparent 50%,var(--muted) 50%),linear-gradient(135deg,var(--muted) 50%,transparent 50%);background-position:calc(100% - 15px) calc(50% + 1px),calc(100% - 10px) calc(50% + 1px);background-size:5px 5px,5px 5px;background-repeat:no-repeat;padding-right:34px}
textarea{min-height:130px;resize:vertical;font:12.5px/1.65 var(--mono)}
input:focus,select:focus,textarea:focus{outline:none;border-color:var(--accent);box-shadow:0 0 0 3px rgba(255,180,84,.17);background:var(--surface)}
label{display:block;color:var(--muted);font:10.5px/1.45 var(--mono);letter-spacing:.1em;text-transform:uppercase;margin:16px 0 6px}
label.toggle{display:flex;align-items:center;gap:7px;margin:0;font:12px var(--sans);letter-spacing:0;text-transform:none;color:var(--muted)}
label.toggle input{width:16px;flex:0 0 16px}
.form{max-width:520px}
.form .btn{margin-top:18px}
.row>input:not([type=checkbox]):not([type=hidden]){width:auto;flex:1 1 220px}
.create-form{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin:0}
.create-form input{width:200px;min-height:36px;padding:7px 11px;font-size:13px}
details{margin:0}
summary{cursor:pointer;color:var(--muted);font-size:12.5px}
summary:hover{color:var(--ink)}
summary::marker{color:var(--faint)}

/* ---------- tables ---------- */
table{width:100%;min-width:680px;border-collapse:collapse;font-size:13px}
th{position:relative;text-align:left;padding:9px 12px;border-bottom:1px solid var(--line);color:var(--faint);font:10px var(--mono);letter-spacing:.12em;text-transform:uppercase;font-weight:500;background:var(--surface);white-space:nowrap}
td{padding:11px 12px;border-bottom:1px solid var(--line-soft);vertical-align:middle;overflow-wrap:anywhere}
tbody tr:last-child td{border-bottom:0}
tbody tr{transition:background .12s}
tbody tr:hover{background:rgba(255,255,255,.022)}
td code{font-size:12px;color:var(--ink-2);background:rgba(255,255,255,.03);border:1px solid var(--line-soft);border-radius:4px;padding:1px 5px}

/* ---------- tabs ---------- */
.tabs{display:flex;gap:4px;border-bottom:1px solid var(--line);margin:28px 0 0;padding:0;flex-wrap:wrap}
.tabs a{position:relative;display:inline-flex;align-items:center;gap:6px;padding:11px 14px;color:var(--muted);font-size:13px;font-weight:500;border-radius:7px 7px 0 0;margin-bottom:-1px;border-bottom:2px solid transparent}
.tabs a:hover{color:var(--ink);background:rgba(255,255,255,.03);text-decoration:none}
.tabs a.on{color:var(--ink);border-bottom-color:var(--accent);background:linear-gradient(180deg,transparent,rgba(255,180,84,.05))}
.tab-body{padding-top:2px}

/* ---------- project hero + KPIs ---------- */
.phero{position:relative;display:flex;align-items:flex-start;justify-content:space-between;gap:24px;flex-wrap:wrap;padding:26px 28px;border:1px solid var(--line);border-radius:18px;background:radial-gradient(620px 220px at 0 0,rgba(255,180,84,.07),transparent 65%),linear-gradient(180deg,rgba(255,255,255,.025),transparent 72%),var(--surface);overflow:hidden}
.phero::after{content:"";position:absolute;left:0;right:0;top:0;height:1px;background:linear-gradient(90deg,transparent,rgba(255,180,84,.5),transparent)}
.phero h1{margin:0 0 6px;font-size:clamp(28px,3vw,38px)}
.phero .eyebrow{margin-bottom:12px}
.phero-domain{font:12.5px/1.6 var(--mono);color:var(--muted);overflow-wrap:anywhere}
.phero-domain a{color:var(--muted)}
.phero-domain a:hover{color:var(--accent);text-decoration:none}
.phero-meta{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-top:14px}
.phero-actions{display:flex;align-items:center;gap:8px;flex-wrap:wrap;padding-top:6px}
.kpis{display:grid;grid-template-columns:repeat(auto-fit,minmax(172px,1fr));gap:12px;margin:16px 0 26px}
.kpi{padding:16px 18px;border:1px solid var(--line);border-radius:var(--r);background:linear-gradient(180deg,rgba(255,255,255,.022),transparent 72%),var(--surface);min-width:0;animation:rise .45s cubic-bezier(.22,.61,.36,1) both}
.kpi-label{font:10px/1.4 var(--mono);letter-spacing:.12em;text-transform:uppercase;color:var(--faint)}
.kpi-val{margin-top:8px;font-size:30px;line-height:1.05;letter-spacing:-.035em;font-weight:600;font-variant-numeric:tabular-nums;overflow-wrap:anywhere}
.kpi-val.sm{font-size:17px;line-height:1.35;letter-spacing:-.01em;color:var(--ink-2);font-weight:500}
.kpi-sub{margin-top:5px;font-size:11.5px;line-height:1.5;color:var(--muted)}

/* ---------- env / logs / details ---------- */
.envitem{border:1px solid var(--line);border-radius:var(--r);padding:14px 16px;margin:0 0 10px;background:var(--surface);transition:border-color .15s}
.envitem:hover{border-color:var(--line-strong)}
.envitem .k{font:600 12.5px var(--mono);color:var(--ink)}
.env-row{flex-wrap:wrap}
pre.log{margin:0;padding:16px 18px;background:#07070a;border:1px solid var(--line);border-radius:var(--r);overflow:auto;max-height:440px;font:12.5px/1.65 var(--mono);color:#c9c9d2;white-space:pre-wrap;tab-size:2;box-shadow:0 1px 0 rgba(255,255,255,.02) inset}
.log-head{display:flex;align-items:center;justify-content:space-between;gap:12px;margin:22px 0 8px;padding-bottom:8px;border-bottom:1px solid var(--line-soft)}
.log-head h3{margin:0}
.danger-zone{border:1px solid rgba(255,114,100,.22);border-radius:var(--r-lg);padding:18px 20px;background:linear-gradient(180deg,rgba(255,114,100,.05),transparent 72%),var(--surface);margin-top:14px}
.danger-zone strong{font-size:14px;color:#ffb3ab}

/* ---------- banners ---------- */
.notice,.error{border:1px solid var(--line);border-left:2px solid var(--success);background:var(--success-soft);color:#c8f3d7;border-radius:8px;padding:11px 14px;margin:0 0 18px;font-size:12.5px;line-height:1.6;overflow-wrap:anywhere}
.error{border-left-color:var(--danger);background:var(--danger-soft);color:#ffcac4}
.notice a,.error a{color:inherit;text-decoration:underline;text-underline-offset:2px}
.notice code,.error code{background:rgba(0,0,0,.28);border:1px solid rgba(255,255,255,.08);border-radius:4px;padding:1px 5px;color:inherit}

/* ---------- auth ---------- */
.auth{position:relative;z-index:1;display:flex;flex-direction:column;justify-content:center;max-width:470px;min-height:100vh;margin:0 auto;padding:48px 24px}
.auth-top{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:30px}
.auth-kicker{font:10px var(--mono);letter-spacing:.18em;text-transform:uppercase;color:var(--faint)}
.auth h1{font-size:clamp(28px,5vw,36px);margin:0 0 8px}
.auth p.lede{margin:0 0 8px;font-size:14px}
.auth .card{margin-top:22px;padding:24px;background:linear-gradient(180deg,rgba(255,255,255,.03),transparent 72%),var(--surface);border-color:var(--line);box-shadow:var(--shadow)}
.auth .card .small{color:var(--muted);line-height:1.65}
.auth .card .small a{color:var(--ink-2);text-decoration:underline;text-underline-offset:2px}
.auth .card .small a:hover{color:var(--accent)}
.foot{margin-top:30px;color:var(--faint);font:10.5px/1.7 var(--mono);letter-spacing:.04em}
.invite{font:11.5px/1.6 var(--mono);word-break:break-all;color:var(--ink-2);background:rgba(255,255,255,.03);border:1px solid var(--line);border-radius:5px;padding:2px 6px}

/* ---------- error pages ---------- */
.errpage{position:relative;z-index:1;max-width:680px;margin:0 auto;padding:12vh 24px 10vh}
.err-code{font:11px var(--mono);letter-spacing:.2em;text-transform:uppercase;color:var(--accent);margin:30px 0 10px}
.errpage h1{font-size:clamp(30px,5vw,46px);margin-bottom:12px}
.errpage .lede{font-size:14.5px}
.errpage .meta{border-top:1px solid var(--line);margin-top:30px;padding-top:16px;font:11.5px/1.8 var(--mono);color:var(--muted)}
.errpage .meta div{margin:5px 0;overflow-wrap:anywhere}
.errpage .meta code{color:var(--ink-2)}

/* ---------- motion ---------- */
@keyframes rise{from{opacity:0;transform:translateY(8px)}to{opacity:1;transform:none}}
@keyframes pulse{0%,100%{opacity:1}50%{opacity:.5}}
.pcard:nth-child(2){animation-delay:.04s}
.pcard:nth-child(3){animation-delay:.08s}
.pcard:nth-child(4){animation-delay:.12s}
.pcard:nth-child(5){animation-delay:.16s}
.pcard:nth-child(6){animation-delay:.2s}
.stat:nth-child(2){animation-delay:.03s}
.stat:nth-child(3){animation-delay:.06s}
.stat:nth-child(4){animation-delay:.09s}
@media(prefers-reduced-motion:reduce){*{animation:none!important;transition:none!important}}
.sr{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0 0 0 0);white-space:nowrap;border:0}

/* ---------- responsive ---------- */
@media(max-width:1080px){
 .layout{grid-template-columns:224px minmax(0,1fr);gap:30px}
 .dash-grid{grid-template-columns:1fr}
 .rail{position:static}
}
@media(max-width:760px){
 .wrap{padding:0 18px}
 .top{height:56px;margin:0 -18px;padding:0 18px}
 .menu{display:inline-flex}
 .top-link{display:none}
 .who{padding-left:12px}
 .who-text{display:none}
 .layout{grid-template-columns:1fr;gap:16px;padding:20px 0 72px}
 main{grid-column:1;grid-row:1}
 aside{position:fixed;top:0;bottom:0;left:0;width:288px;max-width:86vw;background:#0a0a0d;border-right:1px solid var(--line);padding:16px 14px;z-index:60;transform:translateX(-102%);visibility:hidden;transition:transform .2s ease,visibility .2s;overflow:auto;box-shadow:40px 0 90px -40px #000}
 body.nav-open aside{transform:none;visibility:visible}
 .side-head{display:flex;align-items:center;justify-content:space-between;margin-bottom:14px}
 .side .close{display:inline-flex}
 .side-title{margin-top:20px}
 .scrim{display:none;position:fixed;inset:0;background:rgba(0,0,0,.68);z-index:50;border:0;backdrop-filter:blur(2px)}
 body.nav-open .scrim{display:block}
 body.nav-open{overflow:hidden}
 .btn,.menu,.tabs a,.side a,.side .close{min-height:44px}
 .btn.link{min-height:44px;padding:0 4px}
 .tabs{flex-wrap:nowrap;overflow-x:auto;scrollbar-width:none;position:sticky;top:56px;z-index:30;background:rgba(8,8,10,.93);backdrop-filter:blur(12px);margin-top:20px}
 .tabs::-webkit-scrollbar{display:none}
 .tabs a{display:flex;align-items:center;justify-content:center;flex:0 0 auto;padding:12px 14px}
 .page-head{gap:18px;margin-bottom:22px;align-items:flex-start}
 .head-actions{width:100%}
 .create-form{display:grid;grid-template-columns:minmax(0,1fr) auto;width:100%}
 .create-form input{width:100%;min-width:0}
 .stats{grid-template-columns:repeat(2,minmax(0,1fr));margin-bottom:26px}
 .stat{padding:15px 16px 14px}
 .stat-num{font-size:28px}
 .dash-grid{gap:18px}
 .grid{grid-template-columns:1fr}
 .phero{padding:20px 18px;border-radius:14px}
 .phero-actions{width:100%;padding-top:2px}
 .kpis{grid-template-columns:repeat(2,minmax(0,1fr));gap:10px}
 .kpi{padding:14px 15px}
 .kpi-val{font-size:26px}
 .kpi:last-child:nth-child(odd){grid-column:1/-1}
 .start{grid-template-columns:1fr;padding:22px 18px;gap:20px}
 .start h2{font-size:20px}
 table{min-width:0;font-size:12.5px}
 th,td{padding:9px 8px}
 .tabs a{font-size:12.5px}
 .auth{padding:30px 18px}
 .errpage{padding:8vh 18px}
 pre.log{font-size:11.5px;padding:12px 13px;max-height:360px}
 .pcard-foot{margin:3px -16px 0;padding:10px 16px}
 h1{font-size:30px}
 h2{font-size:17px;margin-top:30px}
 .head{padding-top:0}
 input:not([type=checkbox]),select,textarea{font-size:16px;min-height:44px}
}
"#;

pub fn esc(s: &str) -> String {
    auth::esc(s)
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\">\
<meta name=theme-color content=\"#08080a\"><meta name=color-scheme content=dark>\
<link rel=icon href=\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'%3E%3Crect width='32' height='32' rx='7' fill='%23ffb454'/%3E%3Crect x='9' y='9' width='14' height='14' rx='2' fill='%231a1000'/%3E%3C/svg%3E\">\
<title>{t} · dok</title><style>{css}</style></head><body>{body}</body></html>",
        t = esc(title),
        css = CSS,
        body = body
    )
}

fn auth_page(title: &str, subtitle: &str, inner: &str, script: &str) -> String {
    let body = format!(
        "<main class=auth><div class=auth-top><span class=\"brand small\"><span class=mark aria-hidden=true></span>dok</span>\
<span class=auth-kicker>Passkey access</span></div>\
<h1>{t}</h1><p class=lede>{s}</p><div class=card>{inner}</div>\
<div class=foot>Passkeys only — nothing to remember, nothing to leak.</div></main><script>{script}</script>",
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
function fail(e){const el=document.getElementById('err');let msg=e&&e.message?e.message:String(e);if(e&&(e.name==='NotAllowedError'||e.name==='AbortError'))msg='The passkey prompt was cancelled or timed out. Try again.';if(msg.indexOf('too many attempts')>=0)msg='Too many attempts — wait a minute and try again.';if(msg.indexOf('expired')>=0)msg='That sign-in attempt expired. Reload and try again.';if(el){el.textContent=msg;el.hidden=false}const go=document.getElementById('go');if(go){go.disabled=false;if(go.textContent==='Waiting for passkey…')go.textContent='Try again'}}
async function post(url,body){const r=await fetch(url,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body||{})});if(!r.ok)throw new Error(await r.text()||('HTTP '+r.status));return r.json()}
"#;

const SETUP_JS: &str = r#"
async function setup(){
  const go=document.getElementById('go');go.disabled=true;go.textContent='Waiting for passkey…';
  try{
    if(!window.isSecureContext||!window.PublicKeyCredential){fail(new Error('Passkeys need HTTPS or localhost. Open the dashboard on its https:// address (or localhost).'));return}
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
  const go=document.getElementById('go');go.disabled=true;go.textContent='Waiting for passkey…';
  try{
    if(!window.isSecureContext||!window.PublicKeyCredential){fail(new Error('Passkeys need HTTPS or localhost. Open the dashboard on its https:// address (or localhost).'));return}
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

const PASSKEY_JS: &str = r#"
async function addPasskey(){
  const btn=document.getElementById('add-passkey');const err=document.getElementById('passkey-err');
  if(err)err.hidden=true;
  if(!window.isSecureContext||!window.PublicKeyCredential){if(err){err.textContent='Passkeys need HTTPS or localhost.';err.hidden=false}return}
  btn.disabled=true;btn.textContent='Waiting for passkey…';
  try{
    const opts=await post('/auth/passkey/start',{});
    const id=opts.id;const pk=opts.publicKey||opts;
    pk.challenge=b64.toBuf(pk.challenge);
    pk.user.id=b64.toBuf(pk.user.id);
    (pk.excludeCredentials||[]).forEach(c=>c.id=b64.toBuf(c.id));
    const cred=await navigator.credentials.create({publicKey:pk});
    await post('/auth/passkey/finish',{id,credential:encCred(cred)});
    btn.textContent='Passkey added';setTimeout(()=>location.reload(),600);
  }catch(e){btn.disabled=false;btn.textContent='Add passkey';if(err){err.textContent=e.message||String(e);err.hidden=false}}
}
"#;

const JOIN_JS: &str = r#"
async function join(){
  const go=document.getElementById('go');go.disabled=true;go.textContent='Waiting for passkey…';
  try{
    if(!window.isSecureContext||!window.PublicKeyCredential){fail(new Error('Passkeys need HTTPS or localhost. Open the dashboard on its https:// address (or localhost).'));return}
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
        r#"{}<form onsubmit="setup();return false">
<label for=email>Email</label><input id=email type=email autocomplete=username placeholder="you@example.com" required>
<label for=name>Name</label><input id=name type=text autocomplete=name placeholder="Your name" required>
<label for=team>Team</label><input id=team type=text placeholder="Personal">
<button id=go class="btn primary" type=submit style="width:100%;margin-top:16px">Create a passkey</button></form>"#,
        err_box()
    );
    html(auth_page(
        "Create your account",
        "First run: register a passkey to own this instance.",
        &inner,
        &format!("{B64_JS}{SETUP_JS}"),
    ))
}

pub fn page_login_for(req: &Request<Body>) -> Response<Body> {
    let flash = take_flash(req);
    with_flash_cleared(login_page(flash.as_ref()))
}

fn login_page(flash: Option<&Flash>) -> Response<Body> {
    let inner = format!(
        r#"{}{}<p class=small style="margin:0 0 14px">Use the passkey you registered for this instance.</p>
<form onsubmit="login();return false"><button id=go class="btn primary" type=submit style="width:100%">Sign in with passkey</button></form>
<p class="small muted" style="margin:18px 0 0">New here? You need an invite link from a teammate — ask an owner for a <code>/join/…</code> link. Lost your passkey or need another account? An owner can send you a fresh invite. <a href="https://github.com/OckoTajny/dok#quick-start" target=_blank rel=noreferrer>Docs</a></p>
<noscript><p class=error role=alert>Signing in needs JavaScript for the passkey prompt. Enable JavaScript and reload this page.</p></noscript>"#,
        flash_banner(flash),
        err_box()
    );
    html(auth_page(
        "Welcome back",
        "Sign in with a passkey — no password needed.",
        &inner,
        &format!("{B64_JS}{LOGIN_JS}"),
    ))
}

pub fn page_join(token: &str, team: &str, role: &str, inviter: Option<&str>) -> Response<Body> {
    let who = inviter
        .map(|i| format!("{i} invited you"))
        .unwrap_or_else(|| "You were invited".into());
    let inner = format!(
        r#"{}<form onsubmit="join();return false"><input id=token type=hidden value="{}">
<label for=email>Email</label><input id=email type=email autocomplete=username placeholder="you@example.com" required>
<label for=name>Name</label><input id=name type=text autocomplete=name placeholder="Your name" required>
<button id=go class="btn primary" type=submit style="width:100%;margin-top:16px">Join with a passkey</button></form>
<p class="small muted" style="margin:14px 0 0">Already have an account? <a href="/">Sign in</a>, then open this link again.</p>"#,
        err_box(),
        esc(token)
    );
    html(auth_page(
        "Join the team",
        &format!("{who} to join {team} as {role}."),
        &inner,
        &format!("{B64_JS}{JOIN_JS}"),
    ))
}

pub fn page_join_signed_in(
    token: &str,
    team: &str,
    role: &str,
    inviter: Option<&str>,
    csrf: &str,
) -> Response<Body> {
    let who = inviter
        .map(|i| format!("{i} invited you"))
        .unwrap_or_else(|| "You were invited".into());
    let inner = format!(
        r#"<form method=post action=/auth/join/accept>
<input type=hidden name=token value="{token}"><input type=hidden name=csrf value="{csrf}">
<button class="btn primary" type=submit style="width:100%">Join {team}</button></form>
<p class="small muted" style="margin:14px 0 0">You are signed in. This adds {team} to your account.</p>"#,
        token = esc(token),
        csrf = esc(csrf),
        team = esc(team)
    );
    html(auth_page(
        "Join the team",
        &format!("{who} to join {team} as {role}."),
        &inner,
        "",
    ))
}

pub fn page_join_invalid() -> Response<Body> {
    page_html(
        StatusCode::GONE,
        page(
            "Invite not valid",
            "<main class=errpage><span class=\"brand small\"><span class=mark aria-hidden=true></span>dok</span><div class=err-code>Invite link</div>\
<h1>This invite link is not valid</h1><p class=lede>It may have expired, already been used, or been revoked. Ask a team owner or admin for a new link.</p>\
<p><a class=btn href=/>Back to sign in</a></p></main>",
        ),
    )
}

pub fn page_not_configured(st: &AppState) -> Response<Body> {
    let origin = url::Url::parse(&st.public_url)
        .ok()
        .map(|u| {
            let host = st
                .dashboard_origin_host
                .clone()
                .unwrap_or_else(|| u.host_str().unwrap_or("localhost").to_string());
            match u.port() {
                Some(p) => format!("{}://{host}:{p}", u.scheme()),
                None => format!("{}://{host}", u.scheme()),
            }
        })
        .unwrap_or_else(|| "http://localhost:8080".into());
    let msg = format!(
        "The dashboard is not configured. Start dok with a hostname WebAuthn can use, for example:<br>\
<code>dok serve --dashboard-origin {origin} --base-domain apps.example.com</code><br>\
The token API keeps working at <code>/api/v1/health</code>."
    );
    page_html(
        StatusCode::SERVICE_UNAVAILABLE,
        page(
            "Dashboard not configured",
            &format!(
                "<main class=errpage><span class=\"brand small\"><span class=mark aria-hidden=true></span>dok</span><div class=err-code>503 · Not configured</div>\
<h1>Dashboard not configured</h1><p class=lede>{msg}</p>\
<div class=meta><div>Listening on <code>{}</code></div></div></main>",
                esc(&st.public_url)
            ),
        ),
    )
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

fn error_body(
    code: &str,
    title: &str,
    message: &str,
    app: Option<&str>,
    last_good: Option<i64>,
    request_id: &str,
    back: Option<&str>,
) -> String {
    let mut meta = String::new();
    if let Some(app) = app {
        meta.push_str(&format!("<div>Project <strong>{}</strong></div>", esc(app)));
    }
    if let Some(ts) = last_good {
        meta.push_str(&format!(
            "<div>Last good deploy {} <span class=muted>({})</span></div>",
            esc(&fmt_time(ts)),
            esc(&age(ts))
        ));
    } else if app.is_some() {
        meta.push_str("<div>No successful production deploy yet.</div>");
    }
    if request_id != "-" && !request_id.is_empty() {
        meta.push_str(&format!(
            "<div>Request id <code>{}</code></div>",
            esc(request_id)
        ));
    }
    let back = back
        .map(|b| format!("<p><a class=btn href=\"{}\">Back</a></p>", esc(b)))
        .unwrap_or_default();
    format!(
        "<main class=errpage><span class=\"brand small\"><span class=mark aria-hidden=true></span>dok</span>\
<div class=err-code>{code} · dok</div><h1>{title}</h1><p class=lede>{msg}</p>{back}<div class=meta>{meta}</div></main>",
        code = esc(code),
        title = esc(title),
        msg = message,
        back = back,
        meta = meta
    )
}

/// Branded 404/502/504 pages for project hosts. Never lists other projects.
pub fn branded_error(
    status: StatusCode,
    title: &str,
    app: Option<&str>,
    message: &str,
    last_good: Option<i64>,
    request_id: &str,
) -> Response<Body> {
    branded_error_with_back(status, title, app, message, last_good, request_id, None)
}

pub fn branded_error_with_back(
    status: StatusCode,
    title: &str,
    app: Option<&str>,
    message: &str,
    last_good: Option<i64>,
    request_id: &str,
    back: Option<&str>,
) -> Response<Body> {
    let retry = if matches!(
        status,
        StatusCode::BAD_GATEWAY | StatusCode::GATEWAY_TIMEOUT
    ) {
        "<p><button class=btn type=button onclick=\"location.reload()\">Try again</button></p>"
    } else {
        ""
    };
    let body = error_body(
        status.as_str(),
        title,
        &esc(message),
        app,
        last_good,
        request_id,
        back,
    );
    let resp = page_html(
        status,
        page(
            title,
            &format!(
                "{}{}",
                body.replace("</main>", &format!("{retry}</main>")),
                ""
            ),
        ),
    );
    let mut resp = resp;
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    resp
}

fn forbidden(msg: &str) -> Response<Body> {
    page_html(
        StatusCode::FORBIDDEN,
        page(
            "Forbidden",
            &error_body(
                "403",
                "You do not have access",
                &esc(msg),
                None,
                None,
                "-",
                Some("/dashboard"),
            ),
        ),
    )
}

fn not_found(msg: &str) -> Response<Body> {
    page_html(
        StatusCode::NOT_FOUND,
        page(
            "Not found",
            &error_body(
                "404",
                "Not found",
                &esc(msg),
                None,
                None,
                "-",
                Some("/dashboard"),
            ),
        ),
    )
}

/// Validation failures redirect back to the originating dashboard view with a
/// flash error, keeping the user inside the normal layout instead of replacing
/// the whole shell with a standalone error page.
fn unprocessable(msg: &str, back: &str) -> Response<Body> {
    redirect_flash(back, false, msg)
}

// ---------- flash messages ----------

#[derive(Clone)]
struct Flash {
    ok: bool,
    msg: String,
}

fn enc_cookie(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn dec_cookie(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn cookie_value(req: &Request<Body>, name: &str) -> Option<String> {
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

fn take_flash(req: &Request<Body>) -> Option<Flash> {
    let raw = dec_cookie(&cookie_value(req, "dok_flash")?)?;
    let (kind, msg) = raw.split_once('|')?;
    Some(Flash {
        ok: kind == "ok",
        msg: msg.to_string(),
    })
}

fn flash_banner(flash: Option<&Flash>) -> String {
    match flash {
        Some(f) if f.ok => format!("<div class=\"notice\" role=status>{}</div>", esc(&f.msg)),
        Some(f) => format!("<div class=\"error\" role=alert>{}</div>", esc(&f.msg)),
        None => String::new(),
    }
}

pub(crate) fn redirect_flash(to: &str, ok: bool, msg: &str) -> Response<Body> {
    let truncated = msg.chars().count() > 299;
    let mut msg: String = msg.chars().take(299).collect();
    if truncated {
        msg.push('…');
    }
    let value = enc_cookie(&format!("{}|{msg}", if ok { "ok" } else { "err" }));
    // The target can contain a user-supplied team slug: never let it inject a
    // header or panic the response builder.
    let clean: String = to.chars().filter(|c| !matches!(c, '\r' | '\n')).collect();
    let location = header::HeaderValue::from_str(&clean)
        .unwrap_or_else(|_| header::HeaderValue::from_static("/dashboard"));
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(
            header::SET_COOKIE,
            format!(
                "dok_flash={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age=60{}",
                cookie_secure()
            ),
        )
        .body(Body::empty())
        .unwrap()
}

fn take_invite_once(req: &Request<Body>) -> Option<String> {
    dec_cookie(&cookie_value(req, "dok_invite_once")?)
}

fn redirect_invite(to: &str, token: &str) -> Response<Body> {
    let value = enc_cookie(token);
    let location = header::HeaderValue::from_str(to)
        .unwrap_or_else(|_| header::HeaderValue::from_static("/dashboard/team"));
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(
            header::SET_COOKIE,
            format!(
                "dok_invite_once={value}; Path=/dashboard/team; HttpOnly; SameSite=Lax; Max-Age=60{}",
                cookie_secure()
            ),
        )
        .body(Body::empty())
        .unwrap()
}

fn with_invite_cleared(mut resp: Response<Body>) -> Response<Body> {
    if let Ok(v) = header::HeaderValue::from_str(&format!(
        "dok_invite_once=; Path=/dashboard/team; HttpOnly; SameSite=Lax; Max-Age=0{}",
        cookie_secure()
    )) {
        resp.headers_mut().append(header::SET_COOKIE, v);
    }
    resp
}

fn take_token_once(req: &Request<Body>) -> Option<String> {
    dec_cookie(&cookie_value(req, "dok_token_once")?)
}

fn redirect_new_token(to: &str, token: &str) -> Response<Body> {
    let value = enc_cookie(token);
    let location = header::HeaderValue::from_str(to)
        .unwrap_or_else(|_| header::HeaderValue::from_static("/dashboard/account"));
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(
            header::SET_COOKIE,
            format!(
                "dok_token_once={value}; Path=/dashboard/account; HttpOnly; SameSite=Lax; Max-Age=60{}",
                cookie_secure()
            ),
        )
        .body(Body::empty())
        .unwrap()
}

fn with_token_cleared(mut resp: Response<Body>) -> Response<Body> {
    if let Ok(v) = header::HeaderValue::from_str(&format!(
        "dok_token_once=; Path=/dashboard/account; HttpOnly; SameSite=Lax; Max-Age=0{}",
        cookie_secure()
    )) {
        resp.headers_mut().append(header::SET_COOKIE, v);
    }
    resp
}

fn with_flash_cleared(mut resp: Response<Body>) -> Response<Body> {
    if let Ok(v) = header::HeaderValue::from_str(&format!(
        "dok_flash=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
        cookie_secure()
    )) {
        resp.headers_mut().append(header::SET_COOKIE, v);
    }
    resp
}

fn status_html(status: StatusCode, body: String) -> Response<Body> {
    with_flash_cleared(page_html(status, body))
}

fn js_string(msg: &str) -> String {
    let msg: String = msg.chars().filter(|c| *c != '\n' && *c != '\r').collect();
    let js = msg.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", esc(&js))
}

/// Confirm on form submit. The attribute is single-quoted and the JS string
/// double-quoted so HTML entities cannot break out of the JavaScript context.
fn confirm(msg: &str) -> String {
    format!("onsubmit='return confirm({})'", js_string(msg))
}

fn confirm_click(msg: &str) -> String {
    format!("onclick='return confirm({})'", js_string(msg))
}

fn age(ts: i64) -> String {
    let d = crate::db::now() - ts;
    if d < 0 {
        let ahead = -d;
        return if ahead < 90 {
            "in a minute".into()
        } else if ahead < 3600 {
            format!("in {} min", ahead / 60)
        } else if ahead < 7200 {
            "in 1 hour".into()
        } else if ahead < 86400 {
            format!("in {} hours", ahead / 3600)
        } else if ahead < 172800 {
            "in 1 day".into()
        } else {
            format!("in {} days", ahead / 86400)
        };
    }
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

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Absolute UTC timestamp so tables are readable without hovering.
fn fmt_time(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        secs / 3600,
        (secs % 3600) / 60
    )
}

fn when(ts: i64) -> String {
    format!(
        "{}<div class=\"muted small\">{}</div>",
        esc(&age(ts)),
        esc(&fmt_time(ts))
    )
}

fn dot_class(status: &str) -> &'static str {
    match status {
        "ok" | "ready" => "ok",
        "failed" | "error" => "failed",
        "running" | "building" => "run",
        _ => "new",
    }
}

fn initial_of(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "D".into())
}

fn status_badge(status: &str) -> String {
    let class = match status {
        "ok" | "ready" => "ok",
        "failed" | "error" => "failed",
        "running" | "building" => "run",
        "new" => "new",
        _ => "",
    };
    format!("<span class=\"badge {class}\">{}</span>", esc(status))
}

fn http_badge(status: u16) -> String {
    let class = match status {
        200..=299 => "ok",
        300..=399 => "env",
        400..=499 => "run",
        _ => "failed",
    };
    let label = match status {
        200 => "200 OK",
        301 | 302 | 307 | 308 => "redirect",
        401 => "401 unauthorized",
        403 => "403 forbidden",
        404 => "404 not found",
        429 => "429 rate limited",
        500 => "500 server error",
        502 => "502 bad gateway",
        503 => "503 unavailable",
        504 => "504 timeout",
        _ => "request",
    };
    format!(
        "<span class=\"badge {class}\" title=\"{}\">{status}</span>",
        esc(label)
    )
}

fn env_badge(env: &str) -> String {
    let cls = if env == "production" { "env" } else { "" };
    format!("<span class=\"badge {cls}\">{}</span>", esc(env))
}

fn public_url_for(st: &AppState, app: &App) -> Option<String> {
    app.domain
        .as_ref()
        .map(|d| format!("{}://{}", st.scheme, d))
        .or_else(|| {
            st.base_domain
                .as_ref()
                .map(|b| format!("{}://{}.{}", st.scheme, app.name, b))
        })
        .or_else(|| {
            st.db
                .domains_for(&app.name)
                .ok()
                .and_then(|d| d.into_iter().next())
                .map(|d| format!("{}://{}", st.scheme, d))
        })
}

fn domain_of(st: &AppState, app: &App) -> String {
    public_url_for(st, app)
        .map(|u| {
            u.trim_start_matches("https://")
                .trim_start_matches("http://")
                .to_string()
        })
        .unwrap_or_else(|| "no public host configured".into())
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
    active_app: Option<&str>,
    active_section: Option<&str>,
) -> String {
    let mut s = String::from(
        "<aside class=side id=sidebar aria-label=Sidebar>\
<div class=side-head><span class=\"brand small\"><span class=mark aria-hidden=true></span>dok</span>\
<button class=\"btn link close\" id=nav-close type=button onclick=\"toggleNav()\">Close</button></div>",
    );
    if !teams.is_empty() {
        s.push_str(&format!(
            "<div class=side-title>Workspace<span class=side-count>{}</span></div>",
            teams.len()
        ));
    }
    for t in teams {
        let on = active_section != Some("team") && selected.map(|x| x.id == t.id).unwrap_or(false);
        s.push_str(&format!(
            "<a class=\"{}\" href=\"/dashboard?team={}\"{}>{}</a>",
            if on { "on" } else { "" },
            esc(&t.slug),
            if on { " aria-current=page" } else { "" },
            esc(&t.name)
        ));
    }
    if let Some(team) = selected {
        let team_on = active_section == Some("team");
        s.push_str(&format!(
            "<a class=\"{}\" href=\"/dashboard/team?team={}\"{}>Members &amp; invites</a>",
            if team_on { "on" } else { "" },
            esc(&team.slug),
            if team_on { " aria-current=page" } else { "" }
        ));
        let mut apps = db.apps_for_team(&team.id).unwrap_or_default();
        apps.sort_by(|a, b| a.name.cmp(&b.name));
        s.push_str(&format!(
            "<div class=side-title>Projects<span class=side-count>{}</span></div>",
            apps.len()
        ));
        if apps.is_empty() {
            s.push_str("<span class=\"muted small\" style=\"display:block;padding:2px 10px\">No projects yet</span>");
        }
        for a in &apps {
            let on = active_app == Some(a.name.as_str());
            s.push_str(&format!(
                "<a class=\"{}\" href=\"/dashboard/project/{}\"{}><span class=\"sdot {dot}\" aria-hidden=true></span>{}</a>",
                if on { "on" } else { "" },
                esc(&a.name),
                if on { " aria-current=page" } else { "" },
                esc(&a.name),
                dot = dot_class(&a.status)
            ));
        }
    }
    let account_on = active_section == Some("account");
    let team_q = selected
        .map(|t| format!("?team={}", t.slug))
        .unwrap_or_default();
    s.push_str("<div class=side-title>Account</div>");
    s.push_str(&format!(
        "<div class=side-foot style=\"margin-top:0;padding-top:0;border-top:0\">\
<div class=side-user><span class=avatar aria-hidden=true>{initial}</span><div><b>{name}</b><span>{email}</span></div></div>\
<a class=\"{}\" href=\"/dashboard/account{team_q}\"{}>Account &amp; API tokens</a>\
<form method=post action=/dashboard/logout class=inline><input type=hidden name=csrf value=\"{csrf}\"><button class=\"btn link\">Sign out</button></form>\
<div class=side-ver>dok {version}</div></div>",
        if account_on { "on" } else { "" },
        if account_on { " aria-current=page" } else { "" },
        initial = esc(&initial_of(&user.name)),
        name = esc(&user.name),
        email = esc(&user.email),
        csrf = esc(csrf),
        version = env!("CARGO_PKG_VERSION")
    ));
    let _ = st;
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
    active_app: Option<&str>,
    active_section: Option<&str>,
    title: &str,
    flash: Option<&Flash>,
    main: &str,
) -> String {
    let team_q = selected
        .map(|t| format!("?team={}", t.slug))
        .unwrap_or_default();
    let team_name = selected.map(|t| t.name.as_str()).unwrap_or("Workspace");
    let body = format!(
        "<a class=skip href=\"#main\">Skip to content</a>\
<div class=wrap><header class=top><a class=brand href=\"/dashboard{team_q}\"><span class=mark aria-hidden=true></span>dok</a>\
<div class=top-meta><a class=top-link href=\"https://github.com/OckoTajny/dok#readme\" target=_blank rel=noreferrer>Docs</a>\
<div class=who><span class=avatar aria-hidden=true>{initial}</span><div class=who-text><div class=who-name>{user}</div><div class=who-team>{team}</div></div></div>\
<button class=menu type=button aria-controls=sidebar aria-expanded=false onclick=\"toggleNav(this)\">Menu</button></div></header>\
<div class=layout><main id=main tabindex=-1>{flash}{main}</main>\
<button class=scrim type=button aria-label=\"Close menu\" onclick=\"toggleNav()\"></button>{side}</div></div>\
<script>\
function setNav(open){{document.body.classList.toggle('nav-open',open);\
var m=document.querySelector('button.menu');if(m)m.setAttribute('aria-expanded',open);\
if(open){{var c=document.getElementById('nav-close');if(c)c.focus();}}\
else{{var b=document.querySelector('button.menu');if(b)b.focus();}}}}\
function toggleNav(){{setNav(!document.body.classList.contains('nav-open'));}}\
document.addEventListener('keydown',function(e){{if(e.key==='Escape'&&document.body.classList.contains('nav-open'))setNav(false);}});\
function copyText(t,b){{function done(ok){{if(!b)return;var o=b.dataset.label||b.textContent;b.dataset.label=o;b.textContent=ok?'Copied':'Copy failed';setTimeout(function(){{b.textContent=o}},1600);}}function fallback(){{var ta=document.createElement('textarea');ta.value=t;ta.style.position='fixed';ta.style.opacity='0';document.body.appendChild(ta);ta.select();var ok=false;try{{ok=document.execCommand('copy')}}catch(e){{}}ta.remove();done(ok);}}if(navigator.clipboard&&navigator.clipboard.writeText){{navigator.clipboard.writeText(t).then(function(){{done(true)}}).catch(fallback);}}else{{fallback();}}}}\
if(document.querySelector('[data-status-app],[data-deploy-status]')){{setTimeout(pollStatus,5000);}}\
function pollStatus(){{var nodes=document.querySelectorAll('[data-status-app],[data-deploy-status]');if(!nodes.length)return;\
fetch('/dashboard/status',{{headers:{{'accept':'application/json'}},credentials:'same-origin'}}).then(function(r){{if(!r.ok)throw 0;return r.json()}}).then(function(j){{\
nodes.forEach(function(el){{var html=el.dataset.statusApp?(j.apps[el.dataset.statusApp]||{{}}).badge:(j.deploys[el.dataset.deployStatus]||{{}}).badge;if(html&&el.innerHTML!==html)el.innerHTML=html;}});\
if(document.querySelector('.badge.run'))setTimeout(pollStatus,5000);else if(document.querySelector('[data-status-app],[data-deploy-status]'))setTimeout(pollStatus,15000);}}).catch(function(){{setTimeout(pollStatus,10000);}});}}\
</script>",
        user = esc(&user.name),
        team = esc(team_name),
        initial = esc(&initial_of(&user.name)),
        flash = flash_banner(flash),
        side = sidebar(
            st,
            db,
            user,
            teams,
            selected,
            csrf,
            active_app,
            active_section
        ),
        main = main
    );
    page(title, &body)
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

fn can_deploy(db: &Db, user: &User, app: &App) -> bool {
    let Some(tid) = app.team_id.as_deref() else {
        return false;
    };
    let role = db.role(tid, &user.id).ok().flatten();
    crate::db::can_deploy(role.as_deref())
}

fn sparkline(samples: &[crate::state::MetricSample]) -> String {
    if samples.is_empty() {
        return "<div class=\"muted small\">History appears after the next sample (30s).</div>"
            .into();
    }
    let max = samples
        .iter()
        .filter_map(|s| s.cpu)
        .fold(0.0f64, f64::max)
        .max(5.0);
    let mut bars = String::new();
    for s in samples {
        let cpu = s.cpu.unwrap_or(0.0).max(0.0);
        let height = (4.0 + (cpu / max) * 18.0).round() as i64;
        let label = s
            .cpu
            .map(|v| format!("{v:.1}% CPU"))
            .unwrap_or_else(|| "n/a".into());
        bars.push_str(&format!(
            "<span title=\"{}\" style=\"display:inline-block;width:2px;margin-right:1px;height:{}px;background:#ffb454;opacity:.7;vertical-align:bottom\"></span>",
            esc(&label),
            height
        ));
    }
    format!("<div style=\"height:24px;margin:6px 0 4px;line-height:24px\">{bars}</div>")
}

fn no_team_page(
    st: &AppState,
    user: &User,
    session: &Session,
    flash: Option<&Flash>,
) -> Response<Body> {
    let main = format!(
        "<div class=page-head><div><div class=eyebrow><span class=tick></span>Getting started</div><h1>No team yet</h1>\
<p class=lede>You are not a member of any team. Create one, or ask a teammate for an invite link. Teams group projects, environments and members.</p></div></div>\
<div class=start><div><h2>Create a team</h2><p class=lede>One team is enough for a solo setup; add members later with an invite link.</p></div>\
<form class=form method=post action=/dashboard/team/create>{csrf}\
<label for=team-name>Team name</label><input id=team-name name=name placeholder=\"Personal\" maxlength=60 required>\
<button class=\"btn primary\">Create team</button></form></div>",
        csrf = csrf_input(session)
    );
    status_html(
        StatusCode::OK,
        layout_with_sidebar(
            st,
            &st.db,
            user,
            &[],
            None,
            &session.csrf,
            None,
            None,
            "No team",
            flash,
            &main,
        ),
    )
}

fn render_dashboard(
    st: &AppState,
    user: &User,
    session: &Session,
    q: &HashMap<String, String>,
    flash: Option<&Flash>,
) -> Response<Body> {
    let (teams, selected, apps) = team_and_apps(&st.db, user, q);
    let Some(selected_team) = selected.as_ref() else {
        return no_team_page(st, user, session, flash);
    };
    let team_slug = selected_team.slug.clone();
    let can_create_here = selected
        .as_ref()
        .map(|t| {
            let role = st.db.role(&t.id, &user.id).ok().flatten();
            crate::db::can_write(role.as_deref())
        })
        .unwrap_or(false);

    let mut cards = String::new();
    let mut activity: Vec<(String, crate::db::Deploy)> = Vec::new();
    let mut total_deploys = 0usize;
    let mut live = 0usize;
    let mut failing = 0usize;
    let mut requests_hour = 0i64;
    for a in &apps {
        let deploys = st.db.deploys(&a.name, 200).unwrap_or_default();
        total_deploys += deploys.len();
        if a.production_deploy.is_some() {
            live += 1;
        }
        if a.status == "failed" {
            failing += 1;
        }
        requests_hour += st
            .db
            .access_stats(&a.name, crate::db::now() - 3600)
            .map(|(r, _, _)| r)
            .unwrap_or(0);
        for d in deploys.iter().take(12) {
            activity.push((a.name.clone(), d.clone()));
        }
        let latest = deploys.first();
        let url = a
            .domain
            .as_ref()
            .map(|d| format!("{}://{d}", st.scheme))
            .or_else(|| {
                st.base_domain
                    .as_ref()
                    .map(|b| format!("{}://{}.{}", st.scheme, a.name, b))
            });
        let host = url
            .as_deref()
            .map(|u| {
                u.trim_start_matches("https://")
                    .trim_start_matches("http://")
            })
            .unwrap_or("not routed yet");
        let host_html = if latest.is_some() {
            match url.as_deref() {
                Some(u) => format!(
                    "<a href=\"{}\" target=_blank rel=noreferrer>{}</a>",
                    esc(u),
                    esc(host)
                ),
                None => esc(host),
            }
        } else {
            format!(
                "{} <span class=muted>· awaiting first deploy</span>",
                esc(host)
            )
        };
        let (badge, meta) = match latest {
            Some(d) => {
                let short: String = d.id.chars().take(8).collect();
                let commit = d
                    .commit_message
                    .as_deref()
                    .map(|m| {
                        format!(
                            "<div class=\"mono small\" style=\"color:var(--faint);margin-top:2px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap\">{} <code>{}</code></div>",
                            esc(&m.chars().take(52).collect::<String>()),
                            esc(d.commit_sha.as_deref().unwrap_or("").chars().take(7).collect::<String>().as_str())
                        )
                    })
                    .unwrap_or_default();
                (
                    status_badge(&d.status),
                    format!(
                        "{env} <span class=muted>·</span> <span title=\"{abs}\">{ago}</span> <span class=muted>·</span> <code>{short}</code>{commit}",
                        env = env_badge(&d.env),
                        abs = esc(&fmt_time(d.created_at)),
                        ago = esc(&age(d.created_at)),
                        short = esc(&short)
                    ),
                )
            }
            None => (
                status_badge(&a.status),
                "<span class=muted>No deploys yet. Run <code>dok push</code> from the project.</span>"
                    .to_string(),
            ),
        };
        let alarm =
            if a.status == "failed" && !latest.map(|d| d.status == "failed").unwrap_or(false) {
                " <span class=\"badge failed\">build failed</span>"
            } else {
                ""
            };
        let prod = a
            .production_deploy
            .as_ref()
            .map(|d| {
                format!(
                    " <span class=\"badge env\">prod {}</span>",
                    esc(&d.chars().take(8).collect::<String>())
                )
            })
            .unwrap_or_default();
        let author = latest
            .and_then(|d| {
                d.commit_author
                    .as_deref()
                    .or(d.author.as_deref())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "no deploy".into());
        cards.push_str(&format!(
            "<article class=pcard>\
<div class=pcard-head><div class=pcard-id><span class=\"dot {dot}\" aria-hidden=true></span><h3 class=pcard-name><a href=\"/dashboard/project/{n}\">{n}</a></h3></div>\
<div class=row><span data-status-app=\"{n}\" aria-live=polite>{badge}</span>{prod}{alarm}</div></div>\
<div class=pcard-host>{host}</div>\
<div class=pcard-meta>{meta}</div>\
<div class=pcard-foot><span class=pcard-links><a href=\"/dashboard/project/{n}\">Overview</a><a href=\"/dashboard/project/{n}/logs\">Logs</a><a href=\"/dashboard/project/{n}/deployments\">Deploys</a></span><span class=pcard-author>{author}</span></div></article>",
            n = esc(&a.name),
            dot = dot_class(&a.status),
            badge = badge,
            prod = prod,
            alarm = alarm,
            host = host_html,
            meta = meta,
            author = esc(&author)
        ));
    }
    if cards.is_empty() {
        if can_create_here {
            cards.push_str(&format!(
                "<div class=start><div><div class=eyebrow><span class=tick></span>Get started</div>\
<h2>No projects yet</h2>\
<p class=lede>Projects appear here after the first push. A project is one deployable site with its own domains, environments and deployment history.</p></div>\
<ol class=steps>\
<li><strong>Create a project</strong> with the form above or from a push.</li>\
<li><strong>Sign the CLI in</strong><code>dok login {url} &lt;token&gt;</code></li>\
<li><strong>Ship it</strong><code>dok push ./my-app --app my-app</code></li>\
</ol></div>",
                url = esc(&st.public_url)
            ));
        } else {
            cards.push_str("<div class=empty><strong>No projects yet</strong><div class=\"small\" style=\"margin-top:6px\">You have read-only access to this team. Ask an owner or admin to create a project or grant you a deploy role.</div></div>");
        }
    }

    activity.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at));
    let mut timeline = String::new();
    for (app_name, d) in activity.iter().take(8) {
        let short: String = d.id.chars().take(8).collect();
        let detail = d
            .commit_message
            .as_deref()
            .map(|m| m.chars().take(34).collect::<String>())
            .or_else(|| d.branch.clone())
            .unwrap_or_else(|| short.clone());
        timeline.push_str(&format!(
            "<li class=tl><span class=\"dot {dot}\" aria-hidden=true></span>\
<div class=tl-body><div class=tl-title><a href=\"/dashboard/project/{app}/deployments\">{app}</a>{env}</div>\
<div class=tl-meta title=\"{detail_full}\">{detail}</div></div>\
<span class=tl-time title=\"{abs}\">{ago}</span></li>",
            dot = dot_class(&d.status),
            app = esc(app_name),
            env = env_badge(&d.env),
            detail = esc(&detail),
            detail_full = esc(&format!("{} {}", short, detail)),
            abs = esc(&fmt_time(d.created_at)),
            ago = esc(&age(d.created_at))
        ));
    }
    if timeline.is_empty() {
        timeline.push_str("<li class=tl><span class=\"dot new\" aria-hidden=true></span><div class=tl-body><div class=tl-meta>Deploys show up here as they happen.</div></div></li>");
    }

    let team_name = selected
        .as_ref()
        .map(|t| t.name.clone())
        .unwrap_or_else(|| "Workspace".into());
    let members_n = selected
        .as_ref()
        .map(|t| st.db.members(&t.id).map(|m| m.len()).unwrap_or(0))
        .unwrap_or(0);
    let member_word = if members_n == 1 { "member" } else { "members" };
    let can_create = can_create_here;
    let csrf = csrf_input(session);
    let has_tokens = !st
        .db
        .tokens_for_user(&user.id)
        .unwrap_or_default()
        .is_empty();
    let onboarding = if has_tokens {
        String::new()
    } else {
        format!(
            "<div class=notice>New here? <a href=\"/dashboard/account\">Create an API token</a> in Account → API tokens, then run <code>dok login {url} &lt;token&gt;</code> before <code>dok push</code>. Need the CLI? <a href=\"https://github.com/OckoTajny/dok#quick-start\" target=_blank rel=noreferrer>Install dok</a>.</div>",
            url = esc(&st.public_url)
        )
    };
    let create = if can_create {
        format!(
            "<form id=project-create class=create-form method=post action=/dashboard/projects>{csrf}\
<input name=team type=hidden value=\"{slug}\"><label class=sr for=new-project>Project name</label>\
<input id=new-project name=name placeholder=\"new-project\" pattern=\"[a-z0-9][a-z0-9-]*\" maxlength=63 required>\
<button class=\"btn primary\">New project</button></form>",
            csrf = csrf,
            slug = esc(&team_slug)
        )
    } else {
        "<span class=badge>read-only</span>".to_string()
    };
    let failed_sub = if failing > 0 {
        format!(" · {failing} failing")
    } else {
        String::new()
    };
    let main = format!(
        "<div class=page-head><div><div class=eyebrow><span class=tick></span>Workspace</div><h1>{team}</h1>\
<p class=lede>Projects, deployments and previews for this team.</p></div>\
<div class=head-actions><a class=btn href=\"/dashboard/team?team={slug}\">Team &amp; invites</a>{create}</div></div>\
{onboarding}\
<div class=stats>\
<div class=stat><div class=stat-label>Projects</div><div class=stat-num>{projects}</div><div class=stat-sub>in {team}</div></div>\
<div class=\"stat hot\"><div class=stat-label><span class=\"dot ok\" aria-hidden=true></span>Live</div><div class=stat-num>{live}</div><div class=stat-sub>production deploys{failed_sub}</div></div>\
<div class=stat><div class=stat-label>Deployments</div><div class=stat-num>{total_deploys}</div><div class=stat-sub>retained in this team</div></div>\
<div class=stat><div class=stat-label>Requests · 1h</div><div class=stat-num>{requests_hour}</div><div class=stat-sub>from the access log</div></div>\
</div>\
<div class=dash-grid><section><div class=section-head><h2>All projects</h2><span class=count>{projects} total</span></div><div class=grid>{cards}</div></section>\
<div class=rail>\
<div class=panel><div class=panel-head><h2>Recent activity</h2><span class=count>{events}</span></div><ul class=timeline>{timeline}</ul></div>\
<div class=panel><div class=panel-head><h2>Team</h2><span class=count>{members_n}</span></div>\
<div style=\"padding:14px 16px\"><div class=\"muted small\" style=\"margin-bottom:12px\">{members_n} {member_word} · owner, admin, member and viewer roles.</div>\
<a class=\"btn\" style=\"width:100%\" href=\"/dashboard/team?team={slug}\">Manage members &amp; invites</a></div></div>\
<div class=panel><div class=panel-head><h2>Deploy from CLI</h2></div>\
<div style=\"padding:14px 16px\"><code style=\"display:block;font:11.5px/1.7 var(--mono);color:var(--ink-2);overflow-wrap:anywhere\">dok push ./my-app --app my-app</code>\
<div class=\"muted small\" style=\"margin-top:8px\">Production by default; add <code>--env preview</code> for a preview URL.</div></div></div>\
</div></div>",
        team = esc(&team_name),
        slug = esc(&team_slug),
        onboarding = onboarding,
        create = create,
        projects = apps.len(),
        live = live,
        failed_sub = failed_sub,
        total_deploys = total_deploys,
        requests_hour = requests_hour,
        cards = cards,
        timeline = timeline,
        events = activity.len().min(8),
        members_n = members_n,
        member_word = member_word
    );
    status_html(
        StatusCode::OK,
        layout_with_sidebar(
            st,
            &st.db,
            user,
            &teams,
            selected.as_ref(),
            &session.csrf,
            None,
            None,
            &format!("{team_name} · Projects"),
            flash,
            &main,
        ),
    )
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
            "<a class=\"{}\" href=\"/dashboard/project/{}/{}\"{}>{}</a>",
            if id == active { "on" } else { "" },
            esc(app),
            id,
            if id == active {
                " aria-current=page"
            } else {
                ""
            },
            label
        ));
    }
    s.push_str("</nav>");
    s
}

fn cert_table(st: &AppState, app: &App, csrf: &str, can: bool) -> String {
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
    for h in st.db.preview_hosts_for(Some(&app.name)) {
        set.insert(h);
    }
    let mut hosts: Vec<_> = set.into_iter().collect();
    hosts.sort();
    let mut rows = String::new();
    for host in hosts {
        let info = crate::api::cert_info(st, &host);
        let status = if st.edge {
            info["status"].as_str().unwrap_or("pending")
        } else {
            "external proxy"
        };
        let cls = match status {
            "issued" => "ok",
            "error" => "failed",
            "expiring" => "run",
            "external proxy" => "env",
            _ => "",
        };
        let expires = if st.edge {
            info["expires"]
                .as_i64()
                .map(fmt_time)
                .unwrap_or_else(|| "-".into())
        } else {
            "managed by proxy".into()
        };
        let detail = info["error"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| info["note"].as_str().filter(|s| !s.is_empty()))
            .map(|s| {
                format!(
                    "<div class=\"muted small\">{}</div>",
                    esc(&s.chars().take(180).collect::<String>())
                )
            })
            .unwrap_or_default();
        let renew = if can && st.edge {
            format!(
                "<form class=inline method=post action=/dashboard/certificates/renew>{csrf}<input type=hidden name=app value=\"{app}\"><input type=hidden name=host value=\"{host}\">\
<button class=\"btn link\">Renew</button></form>",
                csrf = csrf,
                app = esc(&app.name),
            )
        } else {
            String::new()
        };
        rows.push_str(&format!(
            "<tr><td><code>{host}</code>{detail}</td><td><span class=\"badge {cls}\">{status}</span></td><td class=muted>{expires}</td><td style=\"text-align:right\">{renew}</td></tr>",
            host = esc(&host),
            detail = detail,
            cls = cls,
            status = esc(status),
            expires = esc(&expires),
            renew = renew
        ));
    }
    format!("<div style=\"overflow:auto\"><table><thead><tr><th>Host</th><th>Status</th><th>Expires</th><th><span class=sr>Actions</span></th></tr></thead><tbody>{rows}</tbody></table></div><p class=\"muted small\">Certificates are issued by the built-in edge (<code>dok serve --edge</code>) or your external proxy.</p>")
}

async fn render_project(
    st: &AppState,
    user: &User,
    session: &Session,
    q: &HashMap<String, String>,
    app: &App,
    tab: &str,
    flash: Option<&Flash>,
) -> Response<Body> {
    let (teams, selected, _) = team_and_apps(&st.db, user, q);
    let selected = app
        .team_id
        .as_ref()
        .and_then(|tid| teams.iter().find(|t| &t.id == tid).cloned())
        .or(selected);
    let csrf = csrf_input(session);
    let can = can_write(&st.db, user, app);
    let deployer = can_deploy(&st.db, user, app);
    let mut content = String::new();

    match tab {
        "logs" => {
            let deploys = st.db.deploys(&app.name, 100_000).unwrap_or_default();
            let selected = q
                .get("deploy")
                .and_then(|id| deploys.iter().find(|d| &d.id == id));
            let effective = if q.get("env").is_some() {
                selected
            } else {
                selected.or_else(|| deploys.first())
            };
            let env = effective
                .map(|d| d.env.clone())
                .or_else(|| q.get("env").cloned())
                .unwrap_or_else(|| "production".into());
            let build = match effective {
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
            let kind = effective
                .map(|d| {
                    if d.image.is_some() {
                        "docker"
                    } else {
                        "static"
                    }
                })
                .unwrap_or(app.kind.as_str());
            let container = effective.and_then(|d| d.container.clone());
            let runtime = if kind == "docker" {
                crate::deploy::container_logs(&app.name, &env, container.as_deref(), 200).await
            } else {
                String::new()
            };
            let stats = if kind == "docker" {
                crate::deploy::container_stats(&app.name, &env, container.as_deref()).await
            } else {
                serde_json::json!({})
            };
            let cpu = stats["cpu"].as_str().unwrap_or("n/a");
            let mem = stats["memory"].as_str().unwrap_or("n/a");
            let stats_note = stats["note"].as_str().unwrap_or("");
            let mut selector = String::from("<option value=\"\">Latest deploy</option>");
            let deploys_len = deploys.len();
            for d in &deploys {
                let short = &d.id[..d.id.len().min(8)];
                selector.push_str(&format!(
                    "<option value=\"{id}\"{sel}>{} · {} · {} · {} · {}</option>",
                    short,
                    d.env,
                    esc(d.branch.as_deref().unwrap_or("-")),
                    d.status,
                    fmt_time(d.created_at),
                    id = esc(&d.id),
                    sel = if selected.map(|x| x.id == d.id).unwrap_or(false) {
                        " selected"
                    } else {
                        ""
                    }
                ));
            }
            let status_filter = q.get("status").and_then(|s| s.parse::<u16>().ok());
            let page = q
                .get("page")
                .and_then(|p| p.parse::<usize>().ok())
                .unwrap_or(0);
            let per_page = 100usize;
            let all_access = st
                .db
                .access_logs_filtered(&app.name, 5000, status_filter, None)
                .unwrap_or_default();
            let access_total = st
                .db
                .access_log_count(&app.name, status_filter, None)
                .unwrap_or(all_access.len() as i64);
            let access_rows: Vec<_> = all_access
                .into_iter()
                .skip(page * per_page)
                .take(per_page)
                .collect();
            let access_shown = access_rows.len();
            let pages = ((access_total as usize).saturating_add(per_page - 1)) / per_page;
            let mut pager = String::new();
            if pages > 1 {
                let base = format!(
                    "/dashboard/project/{}/logs?deploy={}&status={}&page=",
                    app.name,
                    effective.map(|d| d.id.as_str()).unwrap_or(""),
                    status_filter.map(|s| s.to_string()).unwrap_or_default()
                );
                if page > 0 {
                    pager.push_str(&format!(
                        "<a class=btn href=\"{base}{}\">Newer</a> ",
                        page - 1
                    ));
                }
                pager.push_str(&format!(
                    "<span class=\"muted small\">Page {} of {}</span>",
                    page + 1,
                    pages
                ));
                if page + 1 < pages {
                    pager.push_str(&format!(
                        " <a class=btn href=\"{base}{}\">Older</a>",
                        page + 1
                    ));
                }
            }
            let mut access = String::new();
            for r in access_rows {
                access.push_str(&format!(
                    "<tr><td>{}</td><td><code>{}</code> {}</td><td>{}</td><td class=muted>{} ms</td><td class=muted>{}</td></tr>",
                    when(r.ts),
                    esc(&r.method),
                    esc(&r.path),
                    http_badge(r.status),
                    esc(&r.latency_ms.to_string()),
                    esc(r.ip.as_deref().unwrap_or("-"))
                ));
            }
            if access.is_empty() {
                access.push_str("<tr><td colspan=5><div class=empty><strong>No requests yet</strong><div class=small style=\"margin-top:6px\">Requests to this project appear here as soon as it is routed.</div></div></td></tr>");
            }
            let build_body = if build.trim().is_empty() {
                "<div class=empty><strong>No build log yet</strong><div class=small style=\"margin-top:6px\">Run <code>dok push</code> or promote a deploy to record one.</div></div>".to_string()
            } else {
                format!("<pre class=log id=build-log>{}</pre>", esc(&build))
            };
            let runtime_body = if kind != "docker" {
                "<div class=empty><strong>Static app</strong><div class=small style=\"margin-top:6px\">This project is served as files, so there is no container runtime log. Build output is above.</div></div>".to_string()
            } else if runtime.trim().is_empty() {
                "<div class=empty><strong>No runtime output yet</strong><div class=small style=\"margin-top:6px\">The container has not written to stdout/stderr.</div></div>".to_string()
            } else {
                format!("<pre class=log id=runtime-log>{}</pre>", esc(&runtime))
            };
            let deploy_q = effective
                .map(|d| format!("&deploy={}", d.id))
                .unwrap_or_default();
            let deploy_id_for_filter = effective.map(|d| d.id.clone()).unwrap_or_default();
            let live_btn = if kind == "docker" {
                format!("<button class=\"btn\" type=button id=live-btn onclick=\"toggleLive('{}')\">Live stream</button>", esc(&format!("/dashboard/stream/{}?env={}{}", app.name, env, deploy_q)))
            } else {
                "<span class=\"badge\">static</span>".to_string()
            };
            let copy_btn = if build.trim().is_empty() {
                String::new()
            } else {
                "<button class=\"btn link\" type=button onclick=\"copyLog('build-log',this)\">Copy</button>\
<button class=\"btn link\" type=button onclick=\"downloadLog('build-log','build.log')\">Download</button>".to_string()
            };
            let runtime_btns = if kind == "docker" && !runtime.trim().is_empty() {
                "<button class=\"btn link\" type=button onclick=\"copyLog('runtime-log',this)\">Copy</button>\
<button class=\"btn link\" type=button onclick=\"downloadLog('runtime-log','runtime.log')\">Download</button>".to_string()
            } else {
                String::new()
            };
            let stats_line = if kind == "docker" {
                format!(
                    "<div class=\"muted small\" style=\"margin-top:10px\">Runtime: CPU {cpu} · memory {mem}{note}</div>",
                    cpu = esc(cpu),
                    mem = esc(mem),
                    note = if stats_note.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", esc(stats_note))
                    }
                )
            } else {
                String::new()
            };
            content.push_str(&format!(
                "<div class=\"row between\" style=\"margin-top:18px\"><h2 style=\"margin:0\">Logs</h2><div class=row>{live_btn}</div></div>\
<div class=\"muted small\" style=\"margin:0 0 6px\">Newest {deploys_len} deploys are selectable here; older builds remain available through the API.</div>\
<form class=\"row\" method=get action=\"/dashboard/project/{app}/logs\" style=\"margin:10px 0 16px\">\
<label class=sr for=deploy-select>Deploy</label><select id=deploy-select name=deploy onchange=\"this.form.submit()\" style=\"max-width:420px\">{selector}</select><button class=\"btn\">Show deploy</button></form>\
<div class=log-head><h3>Build log</h3><div class=row>{copy_btn}</div></div>{build_body}\
<div class=log-head><h3>Runtime log</h3><div class=row>{runtime_btns}</div></div>{runtime_body}\
{stats_line}\
<h3 style=\"margin-top:24px\">Recent requests</h3>\
<div class=\"row between\" style=\"margin-bottom:8px\"><div class=\"muted small\">Showing the newest {access_shown} of {access_total} retained requests.</div>\
<form class=row method=get action=\"/dashboard/project/{app}/logs\"><input type=hidden name=deploy value=\"{deploy_id}\"><label class=sr for=status-filter>Status filter</label>\
<select id=status-filter name=status onchange=\"this.form.submit()\" style=\"width:auto\"><option value=\"\">All statuses</option><option value=\"400\"{f400}>4xx and 5xx</option><option value=\"500\"{f500}>5xx only</option></select></form></div>\
<div class=row style=\"margin-bottom:8px\">{pager}</div>\
<div style=\"overflow:auto\"><table><thead><tr><th>When</th><th>Request</th><th>Status</th><th>Latency</th><th>IP</th></tr></thead><tbody>{access}</tbody></table></div>\
<script>\
var liveCtl=null;\
function copyLog(id,b){{var el=document.getElementById(id);if(el)copyText(el.textContent,b);}}\
function downloadLog(id,name){{var el=document.getElementById(id);if(!el)return;var a=document.createElement('a');a.href=URL.createObjectURL(new Blob([el.textContent],{{type:'text/plain'}}));a.download=name;a.click();setTimeout(function(){{URL.revokeObjectURL(a.href)}},2000);}}\
async function toggleLive(url){{var b=document.getElementById('live-btn');var el=document.getElementById('runtime-log');\
if(liveCtl){{liveCtl.abort();liveCtl=null;b.textContent='Resume stream';return;}}\
if(!el){{el=document.createElement('pre');el.className='log';el.id='runtime-log';document.querySelectorAll('.empty').forEach(function(e){{if(e.textContent.indexOf('No runtime output')>=0)e.replaceWith(el)}});}}\
liveCtl=new AbortController();b.textContent='Pause';el.textContent='';\
try{{var r=await fetch(url,{{signal:liveCtl.signal}});var rd=r.body.getReader();var dec=new TextDecoder();\
while(true){{var n=await rd.read();if(n.done)break;el.textContent+=dec.decode(n.value,{{stream:true}});el.scrollTop=el.scrollHeight;}}\
}}catch(e){{if(e.name!=='AbortError')el.textContent+='\\n[stream stopped: '+e.message+']\\n';}}\
liveCtl=null;b.textContent='Live stream';}}\
</script>",
                app = esc(&app.name),
                selector = selector,
                deploys_len = deploys_len,
                build_body = build_body,
                runtime_body = runtime_body,
                live_btn = live_btn,
                copy_btn = copy_btn,
                runtime_btns = runtime_btns,
                stats_line = stats_line,
                access = access,
                access_shown = access_shown,
                pager = pager,
                access_total = access_total,
                deploy_id = esc(&deploy_id_for_filter),
                f400 = if status_filter == Some(400) { " selected" } else { "" },
                f500 = if status_filter == Some(500) { " selected" } else { "" }
            ));
        }
        "env" => {
            let env = q.get("env").cloned().unwrap_or_else(|| "production".into());
            let vars = st.db.env(&app.name, &env).unwrap_or_default();
            let mut rows = String::new();
            let mut text = String::new();
            for (k, v, secret) in &vars {
                // Secrets are write-only: the textarea only ever sees `KEY=`.
                // Members and viewers get the same masked values as the API.
                if can {
                    text.push_str(&format!("{k}={}\n", if *secret { "" } else { v.as_str() }));
                }
                let current = if !can {
                    "<span class=muted>••••••••</span> <span class=\"badge\">read-only</span>"
                        .to_string()
                } else if *secret {
                    "<span class=muted>••••••••</span> <span class=\"badge\">secret</span>"
                        .to_string()
                } else {
                    format!("<code>{}</code>", esc(v))
                };
                let check = if *secret { " checked" } else { "" };
                let update = if can {
                    format!(
                        "<form class=\"row env-row\" method=post action=\"/dashboard/env/{app}\" style=\"margin-top:8px\">{csrf}\
<input type=hidden name=env value=\"{env}\"><input type=hidden name=key value=\"{key}\"><input type=hidden name=op value=set>\
<label class=sr for=v-{key}>New value for {key}</label>\
<input id=v-{key} name=value placeholder=\"New value (blank = keep)\" autocomplete=off>\
<label class=toggle style=\"margin:0\"><input type=checkbox name=secret value=1{check}> Secret</label>\
<button class=btn>Save</button>\
<button class=\"btn link danger\" name=op value=delete formnovalidate {confirm_remove}>Remove</button>\
</form>",
                        key = esc(k),
                        app = esc(&app.name),
                        csrf = csrf,
                        env = esc(&env),
                        check = check,
                        confirm_remove = confirm_click(&format!(
                            "Remove {k} from {env}? The app keeps running with its current environment until the next deploy."
                        ))
                    )
                } else {
                    String::new()
                };
                rows.push_str(&format!(
                    "<div class=envitem><div class=\"row between\"><code class=k>{key}</code><div class=small>{current}</div></div>{update}</div>",
                    key = esc(k),
                    current = current,
                    update = update
                ));
            }
            if env != "production" {
                let specified: Vec<&str> = vars.iter().map(|(k, _, _)| k.as_str()).collect();
                let production_secrets: Vec<String> = st
                    .db
                    .env(&app.name, "production")
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(_, _, s)| *s)
                    .map(|(k, _, _)| k)
                    .collect();
                for (k, v) in st.db.env_resolved(&app.name, &env).unwrap_or_default() {
                    if specified.contains(&k.as_str()) {
                        continue;
                    }
                    let hidden = !can || production_secrets.contains(&k);
                    let current = if hidden {
                        "<span class=muted>••••••••</span> <span class=\"badge\">inherited</span>"
                            .to_string()
                    } else {
                        format!(
                            "<code>{}</code> <span class=\"badge\">inherited</span>",
                            esc(&v)
                        )
                    };
                    rows.push_str(&format!(
                        "<div class=envitem><div class=\"row between\"><code class=k>{key}</code><div class=small>{current}</div></div></div>",
                        key = esc(&k),
                        current = current
                    ));
                }
            }
            if rows.is_empty() {
                if can {
                    rows.push_str("<div class=empty><strong>No variables in this environment</strong><div class=\"small\" style=\"margin-top:6px\">Add the first one below. Values are passed to the next deploy.</div></div>");
                } else {
                    rows.push_str("<div class=empty><strong>No variables in this environment</strong><div class=\"small\" style=\"margin-top:6px\">You have read-only access; ask an owner or admin to add them.</div></div>");
                }
            }
            let mut switches = String::new();
            for e in ["production", "preview", "development"] {
                let on = if *e == env { " primary" } else { "" };
                switches.push_str(&format!(
                    "<a class=\"btn{on}\" href=\"/dashboard/project/{}/env?env={}\"{}>{}</a> ",
                    esc(&app.name),
                    e,
                    if *e == env { " aria-current=page" } else { "" },
                    e
                ));
            }
            let editor = if can {
                format!(
                    "<h2>Add a variable</h2><form id=env-add method=post action=\"/dashboard/env/{app}\" class=form>{csrf}\
<input type=hidden name=env value=\"{env}\"><input type=hidden name=op value=set>\
<label for=new-key>Key</label><input id=new-key name=key placeholder=\"DATABASE_URL\" pattern=\"[A-Za-z_][A-Za-z0-9_]*\" required>\
<label for=new-value>Value</label><input id=new-value name=value placeholder=\"postgres://...\" autocomplete=off>\
<label class=toggle style=\"margin-top:12px\"><input type=checkbox name=secret value=1> Store as a secret (masked everywhere)</label>\
<button class=\"btn primary\">Add variable</button></form>\
<details style=\"margin-top:28px\"><summary class=\"muted small\" style=\"cursor:pointer\">Bulk edit values</summary>\
<form id=env-bulk method=post action=\"/dashboard/env/{app}\" class=form style=\"margin-top:10px\">{csrf}\
<input type=hidden name=env value=\"{env}\"><input type=hidden name=op value=bulk>\
<label for=bulk>KEY=VALUE, one per line. <code>KEY=</code> keeps the current value; secret keys are shown blank and stay secret. Removing a key is done per row above.</label>\
<textarea id=bulk name=vars>{text}</textarea>\
<button class=\"btn primary\">Save values</button></form></details>",
                    app = esc(&app.name),
                    csrf = csrf,
                    env = esc(&env),
                    text = esc(&text)
                )
            } else {
                "<div class=card><span class=muted>You have read-only access to this project. Values are hidden; ask an owner or admin to change them.</span></div>".to_string()
            };
            content.push_str(&format!(
                "<div class=\"row between\" style=\"margin-top:18px\"><h2 style=\"margin:0\">Environment · {env}</h2><div class=row>{switches}</div></div>\
<div class=\"muted small\" style=\"margin-bottom:12px\">Non-secret production values are inherited by preview and development; the more specific environment overrides them. Secrets are never inherited: set them explicitly per environment. Secret values are never sent back to the browser.</div>\
<div>{rows}</div>{editor}",
                env = esc(&env),
                switches = switches,
                rows = rows,
                editor = editor
            ));
        }
        "domains" => {
            let mut domains = st.db.domains_for(&app.name).unwrap_or_default();
            if let Some(primary) = &app.domain {
                if !domains.contains(primary) {
                    domains.push(primary.clone());
                }
            }
            domains.sort();
            domains.dedup();
            let mut rows = String::new();
            for d in &domains {
                let primary = app.domain.as_deref() == Some(d.as_str());
                let promote = if can && !primary {
                    format!(
                        "<form class=inline method=post action=\"/dashboard/domains/{app}/primary\">{csrf}<input type=hidden name=host value=\"{host}\"><button class=\"btn link\">Make primary</button></form> ",
                        app = esc(&app.name),
                        csrf = csrf,
                        host = esc(d)
                    )
                } else {
                    String::new()
                };
                let remove = if can {
                    format!(
                        "<form class=inline method=post action=\"/dashboard/domains/{app}/remove\" {confirm}>{csrf}<input type=hidden name=host value=\"{host}\">\
<button class=\"btn link danger\">Remove</button></form>",
                        app = esc(&app.name),
                        confirm = confirm(&format!(
                            "Remove {d} from {}? The app stays available on its default URL, but this domain will stop resolving to it.",
                            app.name
                        )),
                        csrf = csrf,
                        host = esc(d),
                    )
                } else {
                    String::new()
                };
                rows.push_str(&format!(
                    "<tr><td><code>{host}</code>{badge}</td><td style=\"text-align:right\">{promote}{remove}</td></tr>",
                    host = esc(d),
                    badge = if primary { " <span class=\"badge env\">primary</span>" } else { "" },
                    promote = promote,
                    remove = remove
                ));
            }
            if rows.is_empty() {
                rows.push_str("<tr><td colspan=2><div class=empty><strong>No custom domains</strong><div class=\"small\" style=\"margin-top:6px\">The default hostname below already works. Add a domain once its DNS points here.</div></div></td></tr>");
            }
            let add_domain = if can {
                format!(
                    "<form class=\"row\" method=post action=\"/dashboard/domains/{app}\" style=\"margin-top:18px;max-width:520px\">{csrf}\
<label class=sr for=new-domain>Domain</label><input id=new-domain name=host placeholder=\"app.example.com\" required><button class=\"btn primary\">Add domain</button></form>",
                    app = esc(&app.name),
                    csrf = csrf
                )
            } else {
                String::new()
            };
            content.push_str(&format!(
                "<h2 style=\"margin-top:18px\">Domains</h2>\
<div class=muted small style=\"margin-bottom:12px\">Default: <code>{default}</code>. Point DNS at this host before adding a custom domain.</div>\
<div style=\"overflow:auto\"><table><thead><tr><th>Domain</th><th><span class=sr>Actions</span></th></tr></thead><tbody>{rows}</tbody></table></div>\
{add_domain}\
<h2 style=\"margin-top:36px\">Certificates</h2>{certs}",
                default = esc(&domain_of(st, app)),
                rows = rows,
                add_domain = add_domain,
                certs = cert_table(st, app, &csrf, can)
            ));
        }
        "proxy" => {
            let s = st.db.proxy_settings(&app.name).unwrap_or_default();
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
            let form = if can {
                format!(
                    "<form method=post action=\"/dashboard/proxy/{app}\" class=form>{csrf}\
<label for=proxy-headers>Custom response headers (Name: value, one per line)</label>\
<textarea id=proxy-headers name=headers placeholder=\"X-Frame-Options: DENY\nContent-Security-Policy: default-src 'self'\">{headers}</textarea>\
<label for=basic-user>Basic auth user</label><input id=basic-user name=basic_auth_user value=\"{user}\" placeholder=\"leave blank to disable\">\
<label for=basic-pass>Basic auth password</label><input id=basic-pass name=basic_auth_password type=password placeholder=\"{pw}\">\
<label for=redirect>www / apex redirect</label><select id=redirect name=redirect>{opts}</select>\
<div class=\"row\" style=\"margin-top:12px\"><label style=\"margin:0\"><input type=checkbox name=compression value=1{comp} style=\"width:auto\"> Compress static text</label>\
<label style=\"margin:0\"><input type=checkbox name=force_https value=1{https} style=\"width:auto\"> Force HTTPS</label></div>\
<label for=max-body>Max request body (MB, 0 = unlimited)</label><input id=max-body name=max_body_mb type=number min=0 max=512 value=\"{max}\">\
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
                )
            } else {
                format!(
                    "<div class=card><span class=muted>You have read-only access to proxy settings.</span><div class=\"small\" style=\"margin-top:8px\">Custom headers: <code>{headers}</code><br>Basic auth user: {user}<br>Redirect: {redirect}<br>Compression: {comp}<br>Force HTTPS: {https}<br>Max body: {max} MB</div></div>",
                    headers = if headers.is_empty() { "none".to_string() } else { esc(&headers) },
                    user = esc(s.basic_auth_user.as_deref().unwrap_or("none")),
                    redirect = esc(redirect),
                    comp = if s.compression { "on" } else { "off" },
                    https = if s.force_https { "on" } else { "off" },
                    max = s.max_body_mb.map(|m| m.to_string()).unwrap_or_else(|| "unlimited".into())
                )
            };
            content.push_str(&format!(
                "<h2 style=\"margin-top:18px\">Proxy</h2>\
<div class=\"muted small\" style=\"margin-bottom:12px\">Applied by the built-in edge/proxy on every request to this project.</div>\
{form}",
                form = form
            ));
        }
        "settings" => {
            let deploy_count = st.db.deploys(&app.name, 500).unwrap_or_default().len();
            let domain_count = st.db.domains_for(&app.name).unwrap_or_default().len();
            let env_count = ["production", "preview", "development"]
                .iter()
                .map(|e| st.db.env(&app.name, e).unwrap_or_default().len())
                .sum::<usize>();
            let delete_msg = format!(
                "Delete project {}? This stops its {kind}, deletes {deploy_count} deployments, {domain_count} custom domains and {env_count} environment variables, and removes build artifacts and certificates. This cannot be undone.",
                app.name,
                kind = if app.kind == "docker" { "container(s)" } else { "served files" }
            );
            let settings_form = if can {
                format!(
                    "<form method=post action=\"/dashboard/settings/{app}\" class=form>{csrf}\
<label for=repo>Git repository</label><input id=repo name=repo value=\"{repo}\" placeholder=\"https://github.com/you/app.git\">\
<label for=framework>Framework</label><input id=framework name=framework value=\"{framework}\" placeholder=\"next, vite, static…\">\
<button class=\"btn primary\">Save settings</button></form>",
                    app = esc(&app.name),
                    csrf = csrf,
                    repo = esc(app.repo.as_deref().unwrap_or("")),
                    framework = esc(app.framework.as_deref().unwrap_or(""))
                )
            } else {
                format!(
                    "<div class=card><span class=muted>You have read-only access to project settings.</span><div class=\"small\" style=\"margin-top:8px\">Git repository: <code>{repo}</code><br>Framework: {framework}</div></div>",
                    repo = esc(app.repo.as_deref().unwrap_or("none")),
                    framework = esc(app.framework.as_deref().unwrap_or("auto"))
                )
            };
            let branch = st
                .db
                .get_setting(&format!("webhook_branch:{}", app.name))
                .ok()
                .flatten()
                .unwrap_or_default();
            let secret_set = st
                .db
                .get_setting(&format!("webhook_secret:{}", app.name))
                .ok()
                .flatten()
                .is_some();
            let webhook_url = format!(
                "{}/api/v1/hooks/github/{}",
                st.public_url.trim_end_matches('/'),
                app.name
            );
            let webhook_form = if can {
                format!(
                    "<h2 style=\"margin-top:36px\">GitHub webhook</h2>\
<div class=\"muted small\" style=\"margin-bottom:10px\">Payload URL <code>{url}</code> · content type <code>application/json</code>.</div>\
<form method=post action=\"/dashboard/webhook/{app}\" class=form>{csrf}\
<label for=hook-branch>Production branch</label><input id=hook-branch name=branch value=\"{branch}\" placeholder=\"main\">\
<label for=hook-secret>Webhook secret (write-only)</label><input id=hook-secret name=secret type=password placeholder=\"{secret}\" autocomplete=off>\
<div class=\"muted small\" style=\"margin:6px 0 10px\">Other branches pushed to this project become preview deployments.</div>\
<button class=\"btn primary\">Save webhook</button></form>",
                    url = esc(&webhook_url),
                    app = esc(&app.name),
                    csrf = csrf,
                    branch = esc(&branch),
                    secret = if secret_set { "configured — leave blank to keep" } else { "at least 16 characters" }
                )
            } else {
                format!(
                    "<h2 style=\"margin-top:36px\">GitHub webhook</h2><div class=card><div class=muted small>Payload URL</div><code>{}</code><div class=\"muted small\" style=\"margin-top:6px\">Production branch: {branch} · secret {secret}</div></div>",
                    esc(&webhook_url),
                    branch = if branch.is_empty() { "not set".to_string() } else { esc(&branch) },
                    secret = if secret_set { "configured" } else { "not set" }
                )
            };
            let danger = if can {
                format!(
                    "<h2 style=\"margin-top:36px\">Danger zone</h2>\
<div class=danger-zone><strong>Delete this project</strong><div class=small style=\"margin:6px 0 12px;color:var(--muted)\">Removes {deploy_count} deployments, {domain_count} custom domains and {env_count} environment variables. It cannot be undone.</div>\
<form method=post action=\"/dashboard/delete/{app}\" {confirm}>{csrf}\
<button class=\"btn danger\">Delete {app}</button></form></div>",
                    deploy_count = deploy_count,
                    domain_count = domain_count,
                    env_count = env_count,
                    app = esc(&app.name),
                    csrf = csrf,
                    confirm = confirm(&delete_msg)
                )
            } else {
                String::new()
            };
            let local_repo = app.repo.as_deref().filter(|r| {
                std::path::Path::new(r).is_absolute()
                    && std::path::Path::new(r).starts_with(st.data.join("repos"))
            });
            let git_hint = if let Some(repo) = local_repo {
                format!(
                    "<div class=\"muted small\" style=\"margin-top:10px\">Server git remote: <code>{}</code></div>",
                    esc(repo)
                )
            } else if app.repo.is_none() {
                format!(
                    "<div class=\"muted small\" style=\"margin-top:10px\">Or initialize a server-side git remote with <code>dok git-init {}</code>.</div>",
                    esc(&app.name)
                )
            } else {
                String::new()
            };
            content.push_str(&format!(
                "<h2 style=\"margin-top:18px\">Settings</h2>{settings_form}{git_hint}{webhook_form}{danger}"
            ));
        }
        _ => {
            let deploys = st.db.deploys(&app.name, 100_000).unwrap_or_default();
            let mut rows = String::new();
            for d in &deploys {
                let short = if d.id.len() > 8 { &d.id[..8] } else { &d.id };
                let live = if app.production_deploy.as_deref() == Some(d.id.as_str()) {
                    " <span class=\"badge env\">live</span>"
                } else {
                    ""
                };
                let mut actions = String::new();
                if deployer && d.status == "ok" && d.env != "production" {
                    actions.push_str(&format!(
                        "<form class=inline method=post action=\"/dashboard/promote/{}/{}\" onsubmit=\"this.querySelector('button').disabled=true;this.querySelector('button').textContent='Working…'\">{}<button class=\"btn link\">Promote to production</button></form> ",
                        esc(&app.name),
                        esc(&d.id),
                        csrf
                    ));
                }
                if deployer
                    && d.env == "production"
                    && d.status == "ok"
                    && app.production_deploy.as_deref() != Some(d.id.as_str())
                {
                    actions.push_str(&format!(
                        "<form class=inline method=post action=\"/dashboard/rollback/{}/{}\" {confirm}>{csrf}<button class=\"btn link\">Roll back here</button></form>",
                        esc(&app.name),
                        esc(&d.id),
                        csrf = csrf,
                        confirm = confirm(&format!(
                            "Roll production back to deploy {}? Visitors switch to this build as soon as it is ready.",
                            d.id.chars().take(8).collect::<String>()
                        ))
                    ));
                }
                if !deployer {
                    actions = "<span class=\"muted small\">read-only role</span>".to_string();
                }
                let mut meta = esc(d
                    .commit_author
                    .as_deref()
                    .or(d.author.as_deref())
                    .unwrap_or("token"));
                if let Some(branch) = &d.branch {
                    meta.push_str(&format!(" <span class=\"badge\">{}</span>", esc(branch)));
                }
                if let Some(sha) = &d.commit_sha {
                    let s8 = &sha[..sha.len().min(7)];
                    let msg = d.commit_message.as_deref().unwrap_or("");
                    meta.push_str(&format!(
                        " · <code>{}</code> {}",
                        esc(s8),
                        esc(&msg.chars().take(60).collect::<String>())
                    ));
                }
                if let Some(url) = d.url.as_deref().filter(|u| !u.is_empty()) {
                    meta.push_str(&format!(
                        "<br><a class=small href=\"{}\" target=_blank rel=noreferrer>{}</a>",
                        esc(url),
                        esc(url
                            .trim_start_matches("https://")
                            .trim_start_matches("http://"))
                    ));
                }
                rows.push_str(&format!(
                    "<tr><td><a href=\"/dashboard/project/{app}/logs?deploy={id}\"><code>{short}</code></a>{live}<div class=\"muted small\">{meta}</div></td><td>{env}</td><td data-deploy-status=\"{id}\">{status}</td><td>{age}</td><td style=\"text-align:right\">{actions}</td></tr>",
                    app = esc(&app.name),
                    id = esc(&d.id),
                    short = esc(short),
                    live = live,
                    meta = meta,
                    env = env_badge(&d.env),
                    status = status_badge(&d.status),
                    age = when(d.created_at),
                    actions = actions
                ));
            }
            if rows.is_empty() {
                rows.push_str("<tr><td colspan=5><div class=empty><strong>No deployments yet</strong><div class=\"small\" style=\"margin-top:6px\">Run <code>dok push</code> from the project directory to create the first one.</div></div></td></tr>");
            }
            let redeploy = if deployer && app.repo.is_some() {
                format!(
                    "<form class=inline method=post action=\"/dashboard/redeploy/{}\" onsubmit=\"this.querySelector('button').disabled=true;this.querySelector('button').textContent='Deploying…'\">{}<button class=\"btn\">Redeploy from git</button></form>",
                    esc(&app.name),
                    csrf
                )
            } else if can {
                format!(
                    "<a class=btn href=\"/dashboard/project/{}/settings\">Connect a repo</a>",
                    esc(&app.name)
                )
            } else {
                String::new()
            };
            let (total, prod_n, preview_n) =
                st.db
                    .deploy_counts(&app.name)
                    .unwrap_or((deploys.len() as i64, 0, 0));
            let additional_domains = st.db.domains_for(&app.name).unwrap_or_default();
            let domain_n = additional_domains.len()
                + usize::from(
                    app.domain
                        .as_ref()
                        .map(|d| !additional_domains.contains(d))
                        .unwrap_or(false),
                );
            let env_total = ["production", "preview", "development"]
                .iter()
                .map(|e| st.db.env(&app.name, e).unwrap_or_default().len())
                .sum::<usize>();
            let runtime = if app.kind == "docker" {
                let stats = crate::deploy::container_stats(&app.name, "production", None).await;
                format!(
                    "CPU {} · memory {}",
                    esc(stats["cpu"].as_str().unwrap_or("n/a")),
                    esc(stats["memory"].as_str().unwrap_or("n/a"))
                )
            } else {
                "static files".to_string()
            };
            let runtime_history = st.metric_history(&app.name, 60);
            let spark = if app.kind == "docker" {
                sparkline(&runtime_history)
            } else {
                String::new()
            };
            let (reqs, errs, p95) = st
                .db
                .access_stats(&app.name, crate::db::now() - 3600)
                .unwrap_or((0, 0, 0));
            let traffic = format!(
                "{reqs} req · {:.1}% 5xx · p95 {p95} ms",
                if reqs > 0 {
                    errs as f64 * 100.0 / reqs as f64
                } else {
                    0.0
                }
            );
            content.push_str(&format!(
                "<div class=\"row between\" style=\"margin-top:24px\"><h2 style=\"margin:0\">Overview</h2><div>{redeploy}</div></div>\
<div class=kpis>\
<div class=kpi><div class=kpi-label>Deployments</div><div class=kpi-val>{total}</div><div class=kpi-sub>{prod_n} production · {preview_n} preview</div></div>\
<div class=kpi><div class=kpi-label>Custom domains</div><div class=kpi-val>{domain_n}</div><div class=kpi-sub>custom + primary</div></div>\
<div class=kpi><div class=kpi-label>Environment keys</div><div class=kpi-val>{env_total}</div><div class=kpi-sub>across all environments</div></div>\
<div class=kpi><div class=kpi-label>Traffic · 1h</div><div class=\"kpi-val sm\">{traffic}</div><div class=kpi-sub>from the access log</div></div>\
<div class=kpi><div class=kpi-label>Runtime</div><div class=\"kpi-val sm\">{runtime}</div>{spark}<div class=kpi-sub>{runtime_note}</div></div></div>\
<div class=section-head><h2>Deployment history</h2><span class=count>{shown} retained</span></div>\
<div class=\"muted small\" style=\"margin:0 0 10px\">Older builds are pruned automatically; the current production deploy is always kept.</div>\
<div style=\"overflow:auto\"><table><thead><tr><th>Deploy</th><th>Env</th><th>Status</th><th>Deployed</th><th><span class=sr>Actions</span></th></tr></thead><tbody>{rows}</tbody></table></div>",
                redeploy = redeploy,
                rows = rows,
                total = total,
                shown = deploys.len(),
                prod_n = prod_n,
                preview_n = preview_n,
                domain_n = domain_n,
                env_total = env_total,
                runtime = runtime,
                traffic = traffic,
                spark = spark,
                runtime_note = if app.kind == "docker" { "docker stats · last 30 min" } else { "served from disk" }
            ));
        }
    }

    let open_btn = if app.production_deploy.is_some() {
        public_url_for(st, app)
            .map(|u| {
                format!(
                    "<a class=btn href=\"{}\" target=_blank rel=noreferrer>Open app</a>",
                    esc(&u)
                )
            })
            .unwrap_or_default()
    } else {
        String::new()
    };
    let domain_html = match public_url_for(st, app) {
        Some(url) => format!(
            "<a href=\"{}\" target=_blank rel=noreferrer>{}</a>",
            esc(&url),
            esc(&domain_of(st, app))
        ),
        None => esc(&domain_of(st, app)),
    };
    let team_label = selected
        .as_ref()
        .map(|t| t.name.clone())
        .unwrap_or_else(|| "Team".into());
    let kind_label = if app.kind == "docker" {
        "container"
    } else {
        "static"
    };
    let main = format!(
        "<div class=phero><div><div class=eyebrow><span class=tick></span>{team} · Project</div><h1>{n}</h1>\
<div class=phero-domain>{domain}</div>\
<div class=phero-meta>{badge}{prod}<span class=badge>{kind}</span><span class=badge>{team}</span></div></div>\
<div class=phero-actions>{open_btn}<a class=btn href=\"/dashboard/project/{n}/logs\">Logs</a><a class=btn href=\"/dashboard/project/{n}/settings\">Settings</a></div></div>\
{tabs}<div class=tab-body>{content}</div>",
        n = esc(&app.name),
        team = esc(&team_label),
        kind = kind_label,
        domain = domain_html,
        badge = status_badge(&app.status),
        open_btn = open_btn,
        prod = app
            .production_deploy
            .as_ref()
            .map(|d| format!("<span class=\"badge env\">prod {}</span>", esc(&d[..d.len().min(8)])))
            .unwrap_or_default(),
        tabs = tabs(&app.name, tab),
        content = content
    );
    status_html(
        StatusCode::OK,
        layout_with_sidebar(
            st,
            &st.db,
            user,
            &teams,
            selected.as_ref(),
            &session.csrf,
            Some(&app.name),
            None,
            &format!("{} · {}", app.name, tab_label(tab)),
            flash,
            &main,
        ),
    )
}

fn render_account(
    st: &AppState,
    user: &User,
    session: &Session,
    q: &HashMap<String, String>,
    flash: Option<&Flash>,
    new_token: Option<&str>,
) -> Response<Body> {
    let (teams, selected, _) = team_and_apps(&st.db, user, q);
    let csrf = csrf_input(session);
    let tokens = st.db.tokens_for_user(&user.id).unwrap_or_default();
    let mut rows = String::new();
    for (hash, name, created) in &tokens {
        rows.push_str(&format!(
            "<tr><td><strong>{name}</strong></td><td class=muted>{created}</td><td style=\"text-align:right\">\
<form class=inline method=post action=/dashboard/tokens/revoke {confirm}>{csrf}\
<input type=hidden name=hash value=\"{hash}\"><button class=\"btn link danger\">Revoke</button></form></td></tr>",
            name = esc(name),
            created = when(*created),
            hash = esc(hash),
            csrf = csrf,
            confirm = confirm(&format!("Revoke the token {name}? Any CLI using it will stop working."))
        ));
    }
    if rows.is_empty() {
        rows.push_str("<tr><td colspan=3><div class=empty><strong>No personal tokens yet</strong><div class=\"small\" style=\"margin-top:6px\">Create one to use <code>dok login</code> and <code>dok push</code> from your machine.</div></div></td></tr>");
    }
    let token_notice = new_token
        .map(|token| {
            format!(
                "<div class=notice>New API token — copy it now, it is shown only once:<br>\
<code id=new-token>{token}</code> \
<button class=\"btn link\" type=button data-token=\"{token}\" onclick=\"copyText(this.dataset.token,this)\">Copy</button>\
<div class=\"small\" style=\"margin-top:8px\">dok login {url} {token}</div></div>",
                token = esc(token),
                url = esc(&st.public_url)
            )
        })
        .unwrap_or_default();
    let main = format!(
        "<div class=page-head><div><div class=eyebrow><span class=tick></span>Settings</div><h1>Account</h1>\
<p class=lede>Personal API tokens for the CLI, and the passkeys that sign you in.</p></div>\
<div class=head-actions><span class=badge>{name}</span></div></div>\
{notice}<h2>Create a token</h2>\
<form class=\"row\" method=post action=/dashboard/tokens style=\"max-width:560px\">{csrf}\
<label class=sr for=token-name>Token name</label><input id=token-name name=name placeholder=\"laptop\" maxlength=64>\
<button class=\"btn primary\">Create token</button></form>\
<h2 style=\"margin-top:34px\">Passkeys</h2>\
<div class=card><div class=muted small>Add a backup passkey so losing one device does not lock you out.</div>\
<div id=passkey-err class=error hidden role=alert></div>\
<button id=add-passkey class=\"btn\" type=button onclick=\"addPasskey()\">Add passkey</button></div>\
<h2 style=\"margin-top:34px\">Your tokens</h2>\
<div style=\"overflow:auto\"><table><thead><tr><th>Name</th><th>Created</th><th></th></tr></thead><tbody>{rows}</tbody></table></div>\
<form method=post action=/dashboard/logout-all style=\"margin-top:28px\">{csrf}\
<button class=btn>Sign out everywhere</button></form><script>{B64_JS}{PASSKEY_JS}</script>",
        notice = token_notice,
        csrf = csrf,
        rows = rows,
        name = esc(&user.name)
    );
    let resp = status_html(
        StatusCode::OK,
        layout_with_sidebar(
            st,
            &st.db,
            user,
            &teams,
            selected.as_ref(),
            &session.csrf,
            None,
            Some("account"),
            "Account",
            flash,
            &main,
        ),
    );
    if new_token.is_some() {
        let mut resp = with_token_cleared(resp);
        resp.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-store"),
        );
        resp
    } else {
        resp
    }
}

fn tab_label(tab: &str) -> &'static str {
    match tab {
        "logs" => "Logs",
        "env" => "Environment",
        "domains" => "Domains",
        "proxy" => "Proxy",
        "settings" => "Settings",
        _ => "Deployments",
    }
}

fn render_team(
    st: &AppState,
    user: &User,
    session: &Session,
    q: &HashMap<String, String>,
    invite: Option<String>,
    flash: Option<&Flash>,
) -> Response<Body> {
    let (teams, selected, _) = team_and_apps(&st.db, user, q);
    let Some(team) = selected else {
        return no_team_page(st, user, session, flash);
    };
    let csrf = csrf_input(session);
    let actor_role = st.db.role(&team.id, &user.id).ok().flatten();
    let can_manage = crate::db::can_write(actor_role.as_deref());
    let granted: &[&str] = if actor_role.as_deref() == Some("owner") {
        &["owner", "admin", "member", "viewer"]
    } else {
        &["member", "viewer"]
    };

    let members = st.db.members(&team.id).unwrap_or_default();
    let owner_count = st.db.owner_count(&team.id).unwrap_or(0);
    let actor_is_owner = actor_role.as_deref() == Some("owner");
    let mut rows = String::new();
    for (u, role) in &members {
        let current = role.as_str();
        let last_owner = current == "owner" && owner_count <= 1;
        let protected = matches!(current, "owner" | "admin") && !actor_is_owner;
        let editable = can_manage && !last_owner && !protected;
        let mut opts = String::new();
        let mut seen: Vec<&str> = Vec::new();
        for r in granted.iter().copied().chain(std::iter::once(current)) {
            if seen.contains(&r) {
                continue;
            }
            seen.push(r);
            opts.push_str(&format!(
                "<option value=\"{r}\"{}>{r}</option>",
                if r == current { " selected" } else { "" }
            ));
        }
        let actions = if editable {
            format!(
                "<form class=row method=post action=/dashboard/team/role>{csrf}\
<input type=hidden name=team value=\"{slug}\"><input type=hidden name=user value=\"{uid}\">\
<label class=sr for=r-{uid}>Role for {name}</label><select id=r-{uid} name=role style=\"max-width:130px\">{opts}</select>\
<button class=btn>Save</button>\
<button class=\"btn link danger\" name=action value=remove formnovalidate {confirm_remove}>Remove</button>\
</form>",
                csrf = csrf,
                slug = esc(&team.slug),
                uid = esc(&u.id),
                name = esc(&u.name),
                opts = opts,
                confirm_remove = confirm_click(&format!(
                    "Remove {} from {}? They immediately lose access to this team's projects, deploys and environment variables.",
                    u.name, team.name
                ))
            )
        } else {
            format!("<span class=badge>{}</span>", esc(role))
        };
        rows.push_str(&format!(
            "<tr><td><strong>{name}</strong><div class=\"muted small\">{email}</div></td><td>{actions}</td><td class=muted>{you}</td></tr>",
            name = esc(&u.name),
            email = esc(&u.email),
            actions = actions,
            you = if u.id == user.id { "you" } else { "" }
        ));
    }

    let invites = st.db.invites_for(&team.id).unwrap_or_default();
    let now = crate::db::now();
    let mut invite_rows = String::new();
    for (token, role, created, expires, used, invited_by) in &invites {
        let role_html = format!("<span class=badge>{}</span>", esc(role));
        let state = if used.is_some() {
            "<span class=badge>used</span>".to_string()
        } else if *expires <= now {
            "<span class=\"badge failed\">expired</span>".to_string()
        } else {
            "<span class=\"badge ok\">active</span>".to_string()
        };
        let link = esc(&format!(
            "{}/join/{}",
            st.public_url.trim_end_matches('/'),
            token
        ));
        let copy = if used.is_none() && *expires > now {
            format!(
                "<button class=\"btn link\" type=button onclick=\"copyText('{link}',this)\">Copy link</button>"
            )
        } else {
            String::new()
        };
        let revoke = if can_manage && used.is_none() && *expires > now {
            format!(
                "<form class=inline method=post action=/dashboard/team/invite/revoke {confirm}>{csrf}\
<input type=hidden name=team value=\"{slug}\"><input type=hidden name=token value=\"{token}\">\
<button class=\"btn link danger\">Revoke</button></form>",
                confirm = confirm("Revoke this invite link? Anyone who has it will no longer be able to join."),
                csrf = csrf,
                slug = esc(&team.slug),
                token = esc(token)
            )
        } else {
            String::new()
        };
        invite_rows.push_str(&format!(
            "<tr><td><span class=invite>{short}…</span><div class=\"muted small\">{who}</div></td><td>{role}</td><td>{state}</td><td>{created}</td><td>{expires}</td><td style=\"text-align:right\">{copy} {revoke}</td></tr>",
            short = esc(&token.chars().take(10).collect::<String>()),
            who = esc(&format!(
                "by {}",
                invited_by.as_deref().unwrap_or("an owner")
            )),
            role = role_html,
            state = state,
            created = when(*created),
            expires = when(*expires),
            copy = copy,
            revoke = revoke
        ));
    }
    let invite_table = if !can_manage {
        "<div class=empty><strong>Invite links are private</strong><div class=\"small\" style=\"margin-top:6px\">Only owners and admins can view or create invite links.</div></div>".to_string()
    } else if invite_rows.is_empty() {
        "<div class=empty><strong>No invite links</strong><div class=\"small\" style=\"margin-top:6px\">Create one above; it is valid for seven days and can be used once.</div></div>".to_string()
    } else {
        format!("<div style=\"overflow:auto\"><table><thead><tr><th>Invite</th><th>Role</th><th>Status</th><th>Created</th><th>Expires</th><th></th></tr></thead><tbody>{invite_rows}</tbody></table></div><p class=\"muted small\">Active links can be copied below; anyone with a link can join until it expires or is revoked.</p>")
    };
    let notice = invite
        .map(|t| {
            format!(
                "<div class=notice>Invite link (valid 7 days, single use):<br><span class=invite>{}/join/{}</span> <button class=\"btn link\" type=button data-token=\"{}/join/{}\" onclick=\"copyText(this.dataset.token,this)\">Copy link</button></div>",
                esc(st.public_url.trim_end_matches('/')),
                esc(&t),
                esc(st.public_url.trim_end_matches('/')),
                esc(&t)
            )
        })
        .unwrap_or_default();

    let mut role_options = String::new();
    for r in ["member", "viewer", "admin", "owner"] {
        if granted.contains(&r) {
            role_options.push_str(&format!("<option>{r}</option>"));
        }
    }
    let invite_form = if can_manage {
        format!(
            "<h2>Invite a teammate</h2><form id=invite-create class=\"row\" method=post action=/dashboard/team/invite style=\"max-width:560px\">{csrf}<input type=hidden name=team value=\"{slug}\">\
<label class=sr for=invite-role>Role</label><select id=invite-role name=role style=\"max-width:160px\">{options}</select>\
<button class=\"btn primary\">Create invite link</button></form>\
<p class=\"muted small\">owner: everything, including admins · admin: projects, settings, invites · member: deploy and read · viewer: read-only.</p>",
            csrf = csrf,
            slug = esc(&team.slug),
            options = role_options
        )
    } else {
        "<p class=muted>Only owners and admins can invite members or manage roles.</p>".into()
    };
    let create_team = format!(
        "<h2 style=\"margin-top:0\">Create a team</h2><form class=\"row\" method=post action=/dashboard/team/create style=\"max-width:560px\">{csrf}\
<input type=hidden name=from value=\"{slug}\"><label class=sr for=team-name>Team name</label>\
<input id=team-name name=name placeholder=\"New team\" maxlength=60 required><button class=\"btn\">Create team</button></form>",
        csrf = csrf,
        slug = esc(&team.slug)
    );

    let main = format!(
        "<div class=page-head><div><div class=eyebrow><span class=tick></span>{slug} · Team</div><h1>{name}</h1>\
<p class=lede>Members, roles and single-use invite links.</p></div>\
<div class=head-actions><a class=btn href=\"/dashboard?team={slug}\">Projects</a></div></div>{notice}\
<div class=section-head><h2>Members</h2><span class=count>{member_count}</span></div>\
<div style=\"overflow:auto\"><table><thead><tr><th>User</th><th>Role</th><th></th></tr></thead><tbody>{rows}</tbody></table></div>\
{invite_form}<div class=section-head style=\"margin-top:36px\"><h2>Invite links</h2></div>{invite_table}\
<div class=panel style=\"margin-top:34px;padding:20px\">{create_team}</div>",
        name = esc(&team.name),
        slug = esc(&team.slug),
        member_count = members.len(),
        notice = notice,
        rows = rows,
        invite_form = invite_form,
        invite_table = invite_table,
        create_team = create_team
    );
    status_html(
        StatusCode::OK,
        layout_with_sidebar(
            st,
            &st.db,
            user,
            &teams,
            Some(&team),
            &session.csrf,
            None,
            Some("team"),
            &format!("{} · Team", team.name),
            flash,
            &main,
        ),
    )
}

async fn read_form(req: Request<Body>) -> Result<HashMap<String, String>, Response<Body>> {
    let bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
        .await
        .map_err(|_| {
            status_html(
                StatusCode::PAYLOAD_TOO_LARGE,
                page(
                    "Form too large",
                    &error_body(
                        "413",
                        "Form too large",
                        "That form was too large. Use the bulk editor with fewer lines and try again.",
                        None,
                        None,
                        "-",
                        Some("/dashboard"),
                    ),
                ),
            )
        })?;
    serde_urlencoded::from_bytes::<HashMap<String, String>>(&bytes).map_err(|_| {
        status_html(
            StatusCode::BAD_REQUEST,
            page(
                "Bad form",
                &error_body(
                    "400",
                    "Bad form",
                    "The form could not be read. Reload the page and submit it again.",
                    None,
                    None,
                    "-",
                    Some("/dashboard"),
                ),
            ),
        )
    })
}

fn check_csrf(session: &Session, form: &HashMap<String, String>) -> Result<(), Response<Body>> {
    match form.get("csrf") {
        Some(v) if v == &session.csrf => Ok(()),
        _ => Err(forbidden("invalid CSRF token")),
    }
}

fn parse_vars(text: &str) -> Result<Vec<(String, String, bool)>, String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(format!("Expected KEY=VALUE, got: {line}"));
        };
        let k = k.trim();
        if !k.is_empty() {
            out.push((k.to_string(), v.trim().to_string(), false));
        }
    }
    Ok(out)
}

fn create_project(
    st: &AppState,
    user: &User,
    form: &HashMap<String, String>,
) -> Result<(), Response<Body>> {
    let name = form
        .get("name")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let back = form
        .get("team")
        .filter(|s| !s.is_empty())
        .map(|s| format!("/dashboard?team={s}"))
        .unwrap_or_else(|| "/dashboard".into());
    if !crate::api::valid_name(&name) {
        return Err(unprocessable(
            "Project names must be lowercase letters, digits and dashes (max 63 characters).",
            &back,
        ));
    }
    let team = if let Some(slug) = form.get("team").filter(|s| !s.is_empty()) {
        let t = st
            .db
            .team_by_slug(slug)
            .ok()
            .flatten()
            .ok_or_else(|| unprocessable("No such team.", &back))?;
        let role = st.db.role(&t.id, &user.id).ok().flatten();
        if !crate::db::can_write(role.as_deref()) {
            return Err(unprocessable(
                "You cannot create projects in this team.",
                &back,
            ));
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
        return Err(unprocessable(
            "You are not an owner or admin of any team.",
            &back,
        ));
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
    if !st
        .db
        .create_app_if_absent(&app)
        .map_err(|e| crate::ui::server_error(&e.to_string()))?
    {
        let writable = st
            .db
            .get_app(&name)
            .ok()
            .flatten()
            .map(|a| {
                let role = a
                    .team_id
                    .as_deref()
                    .and_then(|tid| st.db.role(tid, &user.id).ok().flatten());
                crate::db::can_write(role.as_deref())
            })
            .unwrap_or(false);
        return Err(unprocessable(
            if writable {
                "A project with that name already exists."
            } else {
                "You cannot create that project."
            },
            &back,
        ));
    }
    st.sync_traefik();
    Ok(())
}

fn server_error(msg: &str) -> Response<Body> {
    tracing::error!("dashboard error: {msg}");
    page_html(
        StatusCode::INTERNAL_SERVER_ERROR,
        page(
            "Something went wrong",
            &error_body(
                "500",
                "Something went wrong",
                "The server hit an internal error. It has been logged; try again or check the service log.",
                None,
                None,
                "-",
                Some("/dashboard"),
            ),
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
    let flash = take_flash(&req);

    let is_ui = path == "/dashboard" || path.starts_with("/dashboard/");
    if !is_ui {
        return None;
    }
    let Some(session) = session else {
        let had_cookie = req
            .headers()
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("dok_session="))
            .unwrap_or(false);
        return Some(if had_cookie {
            redirect_flash("/", false, "Your session expired — sign in again.")
        } else {
            auth::redirect("/")
        });
    };
    let user = session.user.clone();

    if method == Method::GET {
        if path == "/dashboard/status" {
            let mut apps = serde_json::Map::new();
            let mut deploys = serde_json::Map::new();
            for team in st.db.teams_for(&user.id).unwrap_or_default() {
                for app in st.db.apps_for_team(&team.id).unwrap_or_default() {
                    apps.insert(
                        app.name.clone(),
                        serde_json::json!({"status": app.status, "badge": status_badge(&app.status)}),
                    );
                    for d in st.db.deploys(&app.name, 100_000).unwrap_or_default() {
                        deploys.insert(
                            d.id.clone(),
                            serde_json::json!({"status": d.status, "badge": status_badge(&d.status)}),
                        );
                    }
                }
            }
            return Some(
                axum::Json(serde_json::json!({"apps": apps, "deploys": deploys})).into_response(),
            );
        }
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
                .map(|d| {
                    if d.image.is_some() {
                        "docker"
                    } else {
                        "static"
                    }
                })
                .unwrap_or(app.kind.as_str());
            let container = deploy.as_ref().and_then(|d| d.container.clone());
            return Some(
                crate::api::log_stream_response(st, &app.name, &env, kind, container.as_deref())
                    .await,
            );
        }
        if path == "/dashboard/account" {
            let token = take_token_once(&req);
            return Some(render_account(
                st,
                &user,
                &session,
                &query,
                flash.as_ref(),
                token.as_deref(),
            ));
        }
        if path == "/dashboard" || path == "/dashboard/" {
            return Some(render_dashboard(
                st,
                &user,
                &session,
                &query,
                flash.as_ref(),
            ));
        }
        if path == "/dashboard/team" {
            let invite = take_invite_once(&req);
            let has_invite = invite.is_some();
            let resp = render_team(st, &user, &session, &query, invite, flash.as_ref());
            return Some(if has_invite {
                with_invite_cleared(resp)
            } else {
                resp
            });
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
        return Some(render_project(st, &user, &session, &query, &app, tab, flash.as_ref()).await);
    }

    if method != Method::POST {
        return Some(status_html(
            StatusCode::METHOD_NOT_ALLOWED,
            page(
                "Method not allowed",
                &error_body(
                    "405",
                    "Method not allowed",
                    "Use the form on the dashboard instead of sending this request directly.",
                    None,
                    None,
                    "-",
                    Some("/dashboard"),
                ),
            ),
        ));
    }

    // All dashboard writes are CSRF-protected.
    let rest = path.trim_start_matches("/dashboard/").to_string();
    let form = match read_form(req).await {
        Ok(f) => f,
        Err(r) => return Some(r),
    };
    if check_csrf(&session, &form).is_err() {
        return Some(forbidden(
            "Your session changed — reload the page and submit the form again.",
        ));
    }

    if rest == "tokens" {
        let name = form
            .get("name")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("cli")
            .chars()
            .take(64)
            .collect::<String>();
        return match st.db.create_token_for_user(&user.id, &name) {
            Ok(token) => Some(redirect_new_token("/dashboard/account", &token)),
            Err(e) => Some(server_error(&e.to_string())),
        };
    }
    if rest == "tokens/revoke" {
        let hash = form.get("hash").cloned().unwrap_or_default();
        return match st.db.revoke_token_for_user(&user.id, &hash) {
            Ok(0) => Some(unprocessable(
                "That token does not belong to you.",
                "/dashboard/account",
            )),
            Ok(_) => Some(redirect_flash("/dashboard/account", true, "Token revoked.")),
            Err(e) => Some(server_error(&e.to_string())),
        };
    }
    if rest == "logout" {
        // Scope a normal sign-out to this device; "everywhere" is explicit.
        let _ = st.db.delete_session(&session.hash);
        return Some(redirect_flash("/", true, "Signed out."));
    }
    if rest == "logout-all" {
        let _ = st.db.delete_sessions_for_user(&user.id);
        return Some(redirect_flash("/", true, "Signed out on every device."));
    }

    if rest == "projects" {
        let team_slug = form.get("team").cloned().unwrap_or_default();
        return match create_project(st, &user, &form) {
            Ok(_) => Some(redirect_flash(
                &format!("/dashboard?team={team_slug}"),
                true,
                &format!(
                    "Project {} created. Push to it with dok push --app {}.",
                    form.get("name").map(String::as_str).unwrap_or(""),
                    form.get("name").map(String::as_str).unwrap_or("")
                ),
            )),
            Err(r) => Some(r),
        };
    }

    if rest == "team/create" {
        let name = form
            .get("name")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if name.is_empty() || name.len() > 60 {
            return Some(unprocessable(
                "Team names are 1–60 characters.",
                "/dashboard/team",
            ));
        }
        let base = crate::auth::slugify(&name);
        return match st.db.create_team_unique(&name, &base, &user.id) {
            Ok(team) => Some(redirect_flash(
                &format!("/dashboard/team?team={}", team.slug),
                true,
                &format!("Team {} created.", team.name),
            )),
            Err(e) => Some(server_error(&e.to_string())),
        };
    } else if rest == "team/invite" {
        let Some(team) = form
            .get("team")
            .and_then(|slug| st.db.team_by_slug(slug).ok().flatten())
        else {
            return Some(unprocessable("No such team.", "/dashboard"));
        };
        let role = form.get("role").cloned().unwrap_or_else(|| "member".into());
        let role = match role.as_str() {
            "owner" | "admin" | "viewer" | "member" => role,
            _ => "member".into(),
        };
        let actor_role = st.db.role(&team.id, &user.id).ok().flatten();
        if !crate::db::can_write(actor_role.as_deref()) {
            return Some(forbidden("only owners and admins can invite"));
        }
        if !crate::db::can_grant(actor_role.as_deref(), &role) {
            return Some(unprocessable(
                "You cannot grant that role. Only owners may grant admin or owner.",
                &format!("/dashboard/team?team={}", team.slug),
            ));
        }
        return match st
            .db
            .create_invite(&team.id, &role, 7 * 86400, Some(&user.id))
        {
            Ok(token) => Some(redirect_invite(
                &format!("/dashboard/team?team={}", team.slug),
                &token,
            )),
            Err(e) => Some(server_error(&e.to_string())),
        };
    } else if rest == "team/invite/revoke" {
        let Some(team) = form
            .get("team")
            .and_then(|slug| st.db.team_by_slug(slug).ok().flatten())
        else {
            return Some(unprocessable("No such team.", "/dashboard"));
        };
        let actor_role = st.db.role(&team.id, &user.id).ok().flatten();
        if !crate::db::can_write(actor_role.as_deref()) {
            return Some(forbidden("only owners and admins can revoke invites"));
        }
        let token = form.get("token").cloned().unwrap_or_default();
        return match st.db.revoke_invite(&team.id, &token) {
            Ok(0) => Some(unprocessable(
                "That invite no longer exists.",
                &format!("/dashboard/team?team={}", team.slug),
            )),
            Ok(_) => Some(redirect_flash(
                &format!("/dashboard/team?team={}", team.slug),
                true,
                "Invite revoked.",
            )),
            Err(e) => Some(server_error(&e.to_string())),
        };
    } else if rest == "team/role" {
        let Some(team) = form
            .get("team")
            .and_then(|slug| st.db.team_by_slug(slug).ok().flatten())
        else {
            return Some(unprocessable("No such team.", "/dashboard"));
        };
        let actor_role = st.db.role(&team.id, &user.id).ok().flatten();
        if !crate::db::can_write(actor_role.as_deref()) {
            return Some(forbidden("only owners and admins can manage members"));
        }
        let target = form.get("user").cloned().unwrap_or_default();
        let target_role = st.db.role(&team.id, &target).ok().flatten();
        if target_role.is_none() {
            return Some(unprocessable(
                "That person is not a member of this team.",
                &format!("/dashboard/team?team={}", team.slug),
            ));
        }
        let back = format!("/dashboard/team?team={}", team.slug);
        if form.get("action").map(String::as_str) == Some("remove") {
            if matches!(target_role.as_deref(), Some("owner") | Some("admin"))
                && actor_role.as_deref() != Some("owner")
            {
                return Some(unprocessable(
                    "Only an owner can remove an owner or admin.",
                    &back,
                ));
            }
            return match st.db.remove_member_checked(&team.id, &target) {
                Ok(crate::db::MemberChange::Changed) => {
                    Some(redirect_flash(&back, true, "Member removed."))
                }
                Ok(crate::db::MemberChange::NotFound) => {
                    Some(unprocessable("That person is not a member.", &back))
                }
                Ok(crate::db::MemberChange::LastOwner) => Some(unprocessable(
                    "This is the last owner. Promote someone else to owner first.",
                    &back,
                )),
                Err(e) => Some(server_error(&e.to_string())),
            };
        }
        let role = form.get("role").cloned().unwrap_or_default();
        if !matches!(role.as_str(), "owner" | "admin" | "member" | "viewer") {
            return Some(unprocessable("Unknown role.", &back));
        }
        if matches!(target_role.as_deref(), Some("owner") | Some("admin"))
            && actor_role.as_deref() != Some("owner")
        {
            return Some(unprocessable(
                "Only an owner can change an owner or admin role.",
                &back,
            ));
        }
        if !crate::db::can_grant(actor_role.as_deref(), &role) {
            return Some(unprocessable(
                "You cannot grant that role. Only owners may grant admin or owner.",
                &back,
            ));
        }
        return match st.db.update_member_role_checked(&team.id, &target, &role) {
            Ok(crate::db::MemberChange::Changed) => Some(redirect_flash(
                &back,
                true,
                &format!("Role changed to {role}."),
            )),
            Ok(crate::db::MemberChange::NotFound) => {
                Some(unprocessable("That person is not a member.", &back))
            }
            Ok(crate::db::MemberChange::LastOwner) => Some(unprocessable(
                "This is the last owner. Promote someone else to owner first.",
                &back,
            )),
            Err(e) => Some(server_error(&e.to_string())),
        };
    }

    if rest == "certificates/renew" {
        let host = form.get("host").cloned().unwrap_or_default();
        let app_name = form.get("app").cloned().unwrap_or_default();
        let Some(app) = project_for(&st.db, &user, &app_name) else {
            return Some(unprocessable(
                "Project not found in your teams.",
                "/dashboard",
            ));
        };
        if !can_write(&st.db, &user, &app) {
            return Some(forbidden("you need owner or admin rights for this project"));
        }
        let back = format!("/dashboard/project/{}/domains", app.name);
        if !crate::api::valid_host(&host) {
            return Some(unprocessable("That does not look like a hostname.", &back));
        }
        if !crate::api::hosts_for_app(st, &app).contains(&host) {
            return Some(unprocessable(
                "That hostname does not belong to this project.",
                &back,
            ));
        }
        if !st.edge {
            return Some(unprocessable(
                "Certificates are managed by your external proxy; use its renewal flow.",
                &back,
            ));
        }
        return match crate::api::request_cert_renewal(st, &host) {
            Ok(_) => Some(redirect_flash(
                &back,
                true,
                &format!("Certificate renewal requested for {host}. It can take a minute."),
            )),
            Err(e) => Some(server_error(&e.to_string())),
        };
    }

    let mut parts = rest.splitn(3, '/');
    let action = parts.next().unwrap_or("");
    let app_name = parts.next().unwrap_or("").to_string();
    let tail = parts.next().unwrap_or("").to_string();

    let Some(app) = project_for(&st.db, &user, &app_name) else {
        return Some(unprocessable(
            "Project not found in your teams.",
            "/dashboard",
        ));
    };
    let deploy_action = matches!(action, "redeploy" | "promote" | "rollback");
    let permitted = if deploy_action {
        can_deploy(&st.db, &user, &app)
    } else {
        can_write(&st.db, &user, &app)
    };
    if !permitted {
        return Some(forbidden("you need the right role for this project action"));
    }
    let deployments = format!("/dashboard/project/{}/deployments", app.name);

    match action {
        "redeploy" => {
            let bg = st.clone();
            let name = app.name.clone();
            tokio::spawn(async move {
                if let Err(e) = crate::api::deploy_from_repo(&bg, &name).await {
                    tracing::error!("dashboard redeploy {name}: {e}");
                }
            });
            Some(redirect_flash(
                &format!("/dashboard/project/{}/deployments", app.name),
                true,
                "Deploy started. The status badge updates automatically; open Logs for build output.",
            ))
        }
        "promote" | "rollback" => {
            if tail.is_empty() {
                return Some(unprocessable("Missing deploy id.", &deployments));
            }
            let bg = st.clone();
            let name = app.name.clone();
            let author = user.email.clone();
            let deploy_id = tail.clone();
            tokio::spawn(async move {
                if let Err(e) = crate::deploy::promote(&bg, &name, &deploy_id, Some(&author)).await
                {
                    tracing::error!("dashboard {name} promote {deploy_id}: {e}");
                }
            });
            Some(redirect_flash(
                &deployments,
                true,
                if action == "promote" {
                    "Promotion started — production switches when the new container is ready."
                } else {
                    "Rollback started — production switches when the new container is ready."
                },
            ))
        }
        "env" => {
            let env = form
                .get("env")
                .cloned()
                .unwrap_or_else(|| "production".into());
            let back = format!("/dashboard/project/{}/env?env={}", app.name, env);
            if !matches!(env.as_str(), "production" | "preview" | "development") {
                return Some(unprocessable("Unknown environment.", &back));
            }
            let existing = st.db.env(&app.name, &env).unwrap_or_default();
            let op = form.get("op").map(String::as_str).unwrap_or("bulk");
            if op == "delete" {
                let key = form.get("key").cloned().unwrap_or_default();
                if key.is_empty() {
                    return Some(unprocessable("Missing key.", &back));
                }
                if let Err(e) = st.db.delete_env(&app.name, &env, &key) {
                    return Some(server_error(&e.to_string()));
                }
                return Some(redirect_flash(
                    &back,
                    true,
                    &format!("{key} removed from {env}. It disappears on the next deploy."),
                ));
            }
            if op == "bulk" {
                let vars = match parse_vars(form.get("vars").map(String::as_str).unwrap_or("")) {
                    Ok(v) => v,
                    Err(msg) => return Some(unprocessable(&msg, &back)),
                };
                for (k, _, _) in &vars {
                    if !valid_env_key(k) {
                        return Some(unprocessable(
                            &format!(
                                "{k} is not a valid variable name (letters, digits and _ only)."
                            ),
                            &back,
                        ));
                    }
                }
                let mut merged = Vec::new();
                for (k, v, _) in vars {
                    let old = existing.iter().find(|(ek, _, _)| ek == &k);
                    let secret = old.map(|(_, _, s)| *s).unwrap_or(false);
                    let value = if v.is_empty() {
                        old.map(|(_, ov, _)| ov.clone()).unwrap_or_default()
                    } else {
                        v
                    };
                    merged.push((k, value, secret));
                }
                if let Err(e) = st.db.put_env(&app.name, &env, &merged) {
                    return Some(server_error(&e.to_string()));
                }
                return Some(redirect_flash(
                    &back,
                    true,
                    &format!(
                        "Saved {} variable(s) for {env}. They apply on the next deploy.",
                        merged.len()
                    ),
                ));
            }
            let key = form
                .get("key")
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            if !valid_env_key(&key) {
                return Some(unprocessable(
                    "Variable names use letters, digits and _ and cannot start with a digit.",
                    &back,
                ));
            }
            let value = form.get("value").cloned().unwrap_or_default();
            let old = existing.iter().find(|(ek, _, _)| ek == &key);
            let secret = form.contains_key("secret");
            let value = if value.is_empty() {
                old.map(|(_, ov, _)| ov.clone()).unwrap_or_default()
            } else {
                value
            };
            if let Err(e) = st
                .db
                .put_env(&app.name, &env, &[(key.clone(), value, secret)])
            {
                return Some(server_error(&e.to_string()));
            }
            Some(redirect_flash(
                &back,
                true,
                &format!("{key} saved in {env}. It applies on the next deploy."),
            ))
        }
        "domains" => {
            let back = format!("/dashboard/project/{}/domains", app.name);
            if tail == "primary" {
                let host = form
                    .get("host")
                    .map(|s| s.trim().trim_end_matches('.').to_ascii_lowercase())
                    .unwrap_or_default();
                let known = st
                    .db
                    .domains_for(&app.name)
                    .unwrap_or_default()
                    .contains(&host)
                    || app.domain.as_deref() == Some(host.as_str());
                if !known || !crate::api::valid_host(&host) {
                    return Some(unprocessable("Unknown domain for this project.", &back));
                }
                if let Err(e) = st.db.set_domain(&app.name, &host) {
                    return Some(server_error(&e.to_string()));
                }
                st.sync_traefik();
                return Some(redirect_flash(
                    &back,
                    true,
                    &format!("{host} is now the primary domain."),
                ));
            }
            if tail == "remove" {
                let host = form
                    .get("host")
                    .map(|s| s.trim().trim_end_matches('.').to_ascii_lowercase())
                    .unwrap_or_default();
                let additional = st.db.remove_domain(&app.name, &host).unwrap_or(0);
                let primary = app.domain.as_deref() == Some(host.as_str());
                if primary {
                    if let Err(e) = st.db.clear_domain(&app.name) {
                        return Some(server_error(&e.to_string()));
                    }
                }
                if additional == 0 && !primary {
                    return Some(unprocessable("Domain not found on this project.", &back));
                }
                st.sync_traefik();
                return Some(redirect_flash(
                    &back,
                    true,
                    &format!(
                        "{host} removed{}.",
                        if primary {
                            " (primary domain cleared)"
                        } else {
                            ""
                        }
                    ),
                ));
            }
            let host = form
                .get("host")
                .map(|s| s.trim().trim_end_matches('.').to_ascii_lowercase())
                .unwrap_or_default();
            if !crate::api::valid_host(&host) {
                return Some(unprocessable(
                    "Enter a hostname with a dot, e.g. app.example.com.",
                    &back,
                ));
            }
            if st.reserved_host(&host) || st.is_base_subdomain(&host) {
                return Some(unprocessable(
                    "That host is reserved for the dashboard. Use a custom domain.",
                    &back,
                ));
            }
            if let Ok(Some(other)) = st.db.domain_owner(&host) {
                if other != app.name {
                    return Some(unprocessable(
                        "That domain already belongs to another project.",
                        &back,
                    ));
                }
            }
            if let Ok(Some(other)) = st.db.app_domain_owner(&host) {
                if other != app.name {
                    return Some(unprocessable(
                        "That domain is the primary domain of another project.",
                        &back,
                    ));
                }
            }
            match st.db.add_domain(&app.name, &host) {
                Ok(true) => {}
                Ok(false) => {
                    return Some(unprocessable(
                        "That domain is already in use by another project.",
                        &back,
                    ))
                }
                Err(e) => return Some(server_error(&e.to_string())),
            }
            st.sync_traefik();
            Some(redirect_flash(
                &back,
                true,
                &format!("{host} added. Point its DNS here; the certificate appears below."),
            ))
        }
        "proxy" => {
            let back = format!("/dashboard/project/{}/proxy", app.name);
            let mut ps = st.db.proxy_settings(&app.name).unwrap_or_default();
            ps.headers.clear();
            if let Some(text) = form.get("headers") {
                match crate::api::parse_proxy_headers(text) {
                    Ok(headers) => ps.headers = headers,
                    Err(msg) => return Some(unprocessable(&msg, &back)),
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
            } else if ps.basic_auth_hash.is_none() {
                return Some(unprocessable(
                    "Enter a basic-auth password the first time you enable a user.",
                    &back,
                ));
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
            Some(redirect_flash(
                &format!("/dashboard/project/{}/proxy", app.name),
                true,
                "Proxy settings saved. They apply to the next request.",
            ))
        }
        "webhook" => {
            let branch = form.get("branch").map(|s| s.trim()).unwrap_or("");
            if !branch.is_empty()
                && !branch
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'))
            {
                return Some(unprocessable(
                    "Branch names may only contain letters, digits, /, -, _ and .",
                    &format!("/dashboard/project/{}/settings", app.name),
                ));
            }
            let secret = form.get("secret").map(|s| s.trim()).unwrap_or("");
            if !secret.is_empty() && secret.len() < 16 {
                return Some(unprocessable(
                    "The webhook secret must be at least 16 characters.",
                    &format!("/dashboard/project/{}/settings", app.name),
                ));
            }
            if let Err(e) = st
                .db
                .set_setting(&format!("webhook_branch:{}", app.name), branch)
            {
                return Some(server_error(&e.to_string()));
            }
            if !secret.is_empty() {
                if let Err(e) = st
                    .db
                    .set_setting(&format!("webhook_secret:{}", app.name), secret)
                {
                    return Some(server_error(&e.to_string()));
                }
            }
            Some(redirect_flash(
                &format!("/dashboard/project/{}/settings", app.name),
                true,
                "Webhook settings saved.",
            ))
        }
        "settings" => {
            let repo = form.get("repo").map(|s| s.trim()).unwrap_or("");
            let local_repo = crate::api::local_repo_allowed(st, &app.name, repo);
            if !repo.is_empty() && !local_repo && !crate::api::valid_repo_url(repo) {
                return Some(unprocessable(
                    "Enter a public https://, ssh:// or git@host:path repository, or leave the field blank.",
                    &format!("/dashboard/project/{}/settings", app.name),
                ));
            }
            let framework = form.get("framework").map(|s| s.trim()).unwrap_or("");
            let repo = if repo.is_empty() { None } else { Some(repo) };
            let framework = if framework.is_empty() {
                None
            } else {
                Some(framework)
            };
            if let Err(e) = st.db.set_repo_framework(&app.name, repo, framework) {
                return Some(server_error(&e.to_string()));
            }
            Some(redirect_flash(
                &format!("/dashboard/project/{}/settings", app.name),
                true,
                "Settings saved.",
            ))
        }
        "delete" => {
            let _lock = match st.lock_deploy(&app.name).await {
                Ok(l) => l,
                Err(e) => return Some(server_error(&e.to_string())),
            };
            let images: Vec<String> = st
                .db
                .deploys(&app.name, 100_000)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|d| d.image)
                .collect();
            crate::deploy::stop_container(&app.name, "production").await;
            crate::deploy::stop_container(&app.name, "preview").await;
            crate::deploy::stop_all_previews(&app.name).await;
            let _ = tokio::fs::remove_dir_all(st.builds_dir().join(&app.name)).await;
            let _ =
                tokio::fs::remove_dir_all(st.data.join("repos").join(format!("{}.git", app.name)))
                    .await;
            if let Err(e) = st.db.delete_app(&app.name) {
                return Some(server_error(&e.to_string()));
            }
            for image in images {
                let _ = tokio::process::Command::new("docker")
                    .args(["rmi", &image])
                    .output()
                    .await;
            }
            st.sync_traefik();
            let slug = app
                .team_id
                .as_deref()
                .and_then(|tid| st.db.team(tid).ok().flatten())
                .map(|t| t.slug)
                .unwrap_or_default();
            Some(redirect_flash(
                &format!("/dashboard?team={slug}"),
                true,
                &format!("Project {} deleted.", app.name),
            ))
        }
        _ => Some(unprocessable("Unknown action.", "/dashboard")),
    }
}

fn valid_env_key(k: &str) -> bool {
    crate::api::valid_env_key(k)
}

#[cfg(test)]
mod tests {
    use super::{age, dec_cookie, enc_cookie, fmt_time, parse_vars};

    #[test]
    fn parses_env_lines() {
        let v = parse_vars("# comment\nA=1\n B = two words \n\nC=\n").unwrap();
        assert_eq!(
            v,
            vec![
                ("A".into(), "1".into(), false),
                ("B".into(), "two words".into(), false),
                ("C".into(), "".into(), false),
            ]
        );
        assert!(parse_vars("A=1\nbadline").is_err());
    }

    #[test]
    fn formats_absolute_time() {
        assert_eq!(fmt_time(0), "1970-01-01 00:00 UTC");
        assert_eq!(fmt_time(1_700_000_000), "2023-11-14 22:13 UTC");
        assert_eq!(fmt_time(1_793_891_740), "2026-11-05 15:15 UTC");
    }

    #[test]
    fn confirm_escapes_javascript_context() {
        let h = super::confirm_click("x')+fetch('/evil')//");
        assert_eq!(h.matches('\'').count(), 2, "{h}");
        assert!(h.contains("&#39;"));
        let h = super::confirm_click("say \"hi\"");
        assert!(h.contains("\\&quot;"), "{h}");
        let h = super::confirm("line\nbreak");
        assert!(!h.contains("\nline\n"));
    }

    #[test]
    fn flash_cookies_roundtrip() {
        let raw = "ok|Project my-app created. 100%";
        assert_eq!(dec_cookie(&enc_cookie(raw)).unwrap(), raw);
        assert!(enc_cookie(raw)
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b';'));
    }

    #[test]
    fn redirect_flash_rejects_header_injection() {
        let r = super::redirect_flash("/dashboard?team=x\r\nX-Evil: 1", true, "ok");
        assert_eq!(r.status(), axum::http::StatusCode::SEE_OTHER);
        assert_eq!(
            r.headers().get(axum::http::header::LOCATION).unwrap(),
            "/dashboard?team=xX-Evil: 1"
        );
        let r = super::redirect_flash("/dashboard?team=ěščř", true, "ok");
        assert!(r.headers().contains_key(axum::http::header::LOCATION));
    }

    #[tokio::test]
    async fn branded_error_has_context_and_no_store() {
        let r = super::branded_error(
            axum::http::StatusCode::BAD_GATEWAY,
            "Bad gateway",
            Some("my-app"),
            "container is restarting",
            Some(1_700_000_000),
            "abc123",
        );
        assert_eq!(r.status(), axum::http::StatusCode::BAD_GATEWAY);
        assert_eq!(
            r.headers().get(axum::http::header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        let body = axum::body::to_bytes(r.into_body(), 128 * 1024)
            .await
            .unwrap();
        let page = String::from_utf8_lossy(&body);
        assert!(page.contains("my-app"));
        assert!(page.contains("abc123"));
        assert!(page.contains("2023-11-14 22:13 UTC"));
        assert!(page.contains("Try again"));
    }

    #[tokio::test]
    async fn env_editor_keeps_secrets_write_only() {
        let path = std::env::temp_dir().join(format!("dok-ui-env-{}.db", uuid::Uuid::new_v4()));
        let db = crate::db::Db::open(&path).unwrap();
        let u = db.create_user("env@example.com", "Env").unwrap();
        let t = db.create_team("Env", "env", &u.id).unwrap();
        db.upsert_app(&crate::db::App {
            name: "app".into(),
            team_id: Some(t.id.clone()),
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
        let data = std::env::temp_dir().join(format!("dok-ui-env-data-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data).unwrap();
        let st = crate::state::AppState::new(db.clone(), data, None, "http://localhost".into());
        let auth =
            crate::auth::AuthState::new(url::Url::parse("http://localhost").unwrap()).unwrap();
        let session = || crate::auth::Session {
            user: u.clone(),
            csrf: "good".into(),
            hash: String::new(),
        };
        let post = |body: &str| {
            axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri("/dashboard/env/app")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap()
        };
        let get = || {
            axum::http::Request::builder()
                .uri("/dashboard/project/app/env")
                .body(axum::body::Body::empty())
                .unwrap()
        };

        let r = super::handle(
            &st,
            &auth,
            post("csrf=good&env=production&op=set&key=TOKEN&value=s3cret&secret=1"),
            Some(session()),
        )
        .await
        .unwrap();
        assert_eq!(r.status(), axum::http::StatusCode::SEE_OTHER);

        let r = super::handle(&st, &auth, get(), Some(session()))
            .await
            .unwrap();
        let body = axum::body::to_bytes(r.into_body(), 512 * 1024)
            .await
            .unwrap();
        let page = String::from_utf8_lossy(&body);
        assert!(!page.contains("s3cret"), "secret leaked into the page");
        assert!(
            page.contains("TOKEN="),
            "secret key missing from bulk editor"
        );

        // `KEY=` in the bulk editor keeps the stored secret value.
        let r = super::handle(
            &st,
            &auth,
            post("csrf=good&env=production&op=bulk&vars=TOKEN%3D%0AOTHER%3D1"),
            Some(session()),
        )
        .await
        .unwrap();
        assert_eq!(r.status(), axum::http::StatusCode::SEE_OTHER);
        let vars = db.env("app", "production").unwrap();
        assert_eq!(
            vars,
            vec![
                ("OTHER".into(), "1".into(), false),
                ("TOKEN".into(), "s3cret".into(), true),
            ]
        );

        // Unchecking the per-row secret box is the only way to declassify.
        super::handle(
            &st,
            &auth,
            post("csrf=good&env=production&op=set&key=TOKEN"),
            Some(session()),
        )
        .await
        .unwrap();
        let vars = db.env("app", "production").unwrap();
        let token = vars.iter().find(|(k, _, _)| k == "TOKEN").unwrap();
        assert_eq!(token.1, "s3cret");
        assert!(!token.2);

        // A viewer sees masked values and no editor, matching the API.
        let viewer = db.create_user("viewer@example.com", "Viewer").unwrap();
        db.add_member(&t.id, &viewer.id, "viewer").unwrap();
        let viewer_session = || crate::auth::Session {
            user: viewer.clone(),
            csrf: "good".into(),
            hash: String::new(),
        };
        let r = super::handle(&st, &auth, get(), Some(viewer_session()))
            .await
            .unwrap();
        let body = axum::body::to_bytes(r.into_body(), 512 * 1024)
            .await
            .unwrap();
        let page = String::from_utf8_lossy(&body);
        assert!(!page.contains("s3cret"), "viewer saw a value");
        assert!(!page.contains("<code>1</code>"), "viewer saw a value");
        assert!(page.contains("read-only"));
        assert!(!page.contains("id=env-add"), "viewer got the editor");
        let _ = std::fs::remove_file(path);
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
        let auth =
            crate::auth::AuthState::new(url::Url::parse("http://localhost").unwrap()).unwrap();
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
                hash: String::new(),
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
                hash: String::new(),
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
        assert_eq!(age(now + 7 * 86400), "in 7 days");
    }
    #[tokio::test]
    async fn user_without_a_team_gets_a_create_page() {
        let path = std::env::temp_dir().join(format!("dok-ui-{}.db", uuid::Uuid::new_v4()));
        let db = crate::db::Db::open(&path).unwrap();
        let u = db.create_user("solo@example.com", "Solo").unwrap();
        let data = std::env::temp_dir().join(format!("dok-ui-data-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data).unwrap();
        let st = crate::state::AppState::new(db, data, None, "http://localhost".into());
        let auth =
            crate::auth::AuthState::new(url::Url::parse("http://localhost").unwrap()).unwrap();
        for uri in ["/dashboard", "/dashboard/team"] {
            let req = axum::http::Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap();
            let r = super::handle(
                &st,
                &auth,
                req,
                Some(crate::auth::Session {
                    user: u.clone(),
                    csrf: "good".into(),
                    hash: String::new(),
                }),
            )
            .await
            .unwrap();
            assert_eq!(r.status(), axum::http::StatusCode::OK, "{uri}");
            let body = axum::body::to_bytes(r.into_body(), 512 * 1024)
                .await
                .unwrap();
            assert!(
                String::from_utf8_lossy(&body).contains("Create a team"),
                "{uri}"
            );
        }
        let _ = std::fs::remove_file(path);
    }
}
