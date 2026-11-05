use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "dok", version, about = "Tiny self-hosted deploy platform")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Run the server.
    Serve(ServeArgs),
    /// Save server URL and token.
    Login { server: String, token: String },
    /// Upload the current directory and deploy it.
    Push(PushArgs),
    /// List apps.
    Apps,
    /// Show build and runtime logs (-f to follow).
    Logs {
        app: String,
        #[arg(short, long)]
        follow: bool,
    },
    /// Set environment variables (KEY=VALUE ...).
    Env {
        app: String,
        #[arg(value_name = "KEY=VALUE")]
        vars: Vec<String>,
    },
    /// Point a domain at an app.
    Domain { app: String, host: String },
    /// Set the git repo used by webhooks / redeploy.
    Git { app: String, repo: String },
    /// Redeploy from the configured repo.
    Redeploy { app: String },
    /// List recent deploys (id, status, age).
    Deploys { app: String },
    /// Roll back to an earlier deploy id.
    Rollback { app: String, deploy: String },
    /// Create a bare repo on the server and print the git remote.
    GitInit { app: String },
    /// Internal: deploy a pushed git repo (called by the post-receive hook).
    #[command(hide = true)]
    DeployGit { app: String, #[arg(long)] data: String },
    /// Delete an app and stop its container.
    Rm { app: String },
}

#[derive(Args, Clone)]
pub struct ServeArgs {
    /// Data directory.
    #[arg(long, env = "DOK_DATA", default_value = "~/.local/share/dok")]
    pub data: String,
    /// Plain HTTP port.
    #[arg(long, env = "DOK_HTTP_PORT", default_value_t = 8080)]
    pub http_port: u16,
    /// HTTPS port (only with --acme-domain).
    #[arg(long, env = "DOK_TLS_PORT", default_value_t = 443)]
    pub tls_port: u16,
    /// Base domain for `<app>.base` routing, e.g. dok.example.com.
    #[arg(long, env = "DOK_BASE_DOMAIN")]
    pub base_domain: Option<String>,
    /// Host shown in CLI output when no domain is set.
    #[arg(long, env = "DOK_PUBLIC_HOST", default_value = "127.0.0.1")]
    pub public_host: String,
    /// Public origin for the passkey dashboard, e.g. https://dok.example.com.
    #[arg(long, env = "DOK_DASHBOARD_ORIGIN")]
    pub dashboard_origin: Option<String>,
    /// Number of daily database backups to keep (0 disables).
    #[arg(long, env = "DOK_BACKUP_KEEP", default_value_t = 7)]
    pub backup_keep: usize,
    /// Traefik file-provider directory; dok writes per-app routers there.
    #[arg(long, env = "DOK_TRAEFIK_DIR")]
    pub traefik_dir: Option<PathBuf>,
    /// Register this API token (otherwise one is generated on first start).
    #[arg(long, env = "DOK_TOKEN")]
    pub token: Option<String>,
    /// Domain to obtain a Let's Encrypt certificate for (repeatable).
    #[arg(long = "acme-domain", env = "DOK_ACME_DOMAIN")]
    pub acme_domain: Vec<String>,
    /// Contact email for Let's Encrypt (repeatable).
    #[arg(long = "acme-email", env = "DOK_ACME_EMAIL")]
    pub acme_email: Vec<String>,
    /// Use the Let's Encrypt production directory (default: staging).
    #[arg(long, env = "DOK_ACME_PROD")]
    pub acme_prod: bool,
}

#[derive(Args)]
pub struct PushArgs {
    /// Directory to upload (default: current).
    #[arg(default_value = ".")]
    pub dir: PathBuf,
    /// App name (default: dok.toml name or directory name).
    #[arg(long)]
    pub app: Option<String>,
    /// Domain to route to the app.
    #[arg(long)]
    pub domain: Option<String>,
    /// Container port (default: dok.toml, Dockerfile EXPOSE, then 3000).
    #[arg(long)]
    pub port: Option<u16>,
}

#[derive(Serialize, Deserialize)]
struct Config {
    server: String,
    token: String,
}

fn config_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".config/dok/config.toml"))
}

fn load_config() -> Result<Config> {
    if let (Ok(server), Ok(token)) = (
        std::env::var("DOK_SERVER"),
        std::env::var("DOK_TOKEN"),
    ) {
        return Ok(Config { server, token });
    }
    let p = config_path()?;
    let s = std::fs::read_to_string(&p)
        .with_context(|| format!("no config at {}; run `dok login <url> <token>`", p.display()))?;
    Ok(toml::from_str(&s)?)
}

fn save_config(c: &Config) -> Result<()> {
    let p = config_path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, toml::to_string(c)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(1800))
        .build()?)
}

fn base(cfg: &Config) -> String {
    cfg.server.trim_end_matches('/').to_string()
}

fn handle(resp: reqwest::blocking::Response) -> Result<()> {
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        bail!("server returned {status}: {text}");
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!("{text}");
    }
    Ok(())
}

pub fn run(cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Serve(_) => unreachable!("serve is async"),
        Cmd::Login { server, token } => {
            let cfg = Config {
                server: server.trim_end_matches('/').to_string(),
                token,
            };
            save_config(&cfg)?;
            println!("saved {}", config_path()?.display());
            Ok(())
        }
        Cmd::Apps => {
            let cfg = load_config()?;
            let resp = client()?
                .get(format!("{}/api/v1/apps", base(&cfg)))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
        Cmd::Push(args) => push(args),
        Cmd::Logs { app, follow } => {
            let cfg = load_config()?;
            if follow {
                let mut resp = client()?
                    .get(format!("{}/api/v1/apps/{app}/logs/stream", base(&cfg)))
                    .bearer_auth(&cfg.token)
                    .send()?;
                if !resp.status().is_success() {
                    return handle(resp);
                }
                let mut out = std::io::stdout();
                std::io::copy(&mut resp, &mut out)?;
                return Ok(());
            }
            let resp = client()?
                .get(format!("{}/api/v1/apps/{app}/logs", base(&cfg)))
                .bearer_auth(&cfg.token)
                .send()?;
            if !resp.status().is_success() {
                return handle(resp);
            }
            let v: serde_json::Value = resp.json()?;
            println!("== build ==\n{}", v["build"].as_str().unwrap_or(""));
            println!("== runtime ==\n{}", v["runtime"].as_str().unwrap_or(""));
            Ok(())
        }
        Cmd::Env { app, vars } => {
            let cfg = load_config()?;
            let mut map = serde_json::Map::new();
            for v in vars {
                let (k, val) = v
                    .split_once('=')
                    .with_context(|| format!("expected KEY=VALUE, got {v}"))?;
                map.insert(k.to_string(), serde_json::Value::String(val.to_string()));
            }
            let resp = client()?
                .put(format!("{}/api/v1/apps/{app}/env", base(&cfg)))
                .bearer_auth(&cfg.token)
                .json(&map)
                .send()?;
            handle(resp)
        }
        Cmd::Domain { app, host } => {
            let cfg = load_config()?;
            let resp = client()?
                .post(format!("{}/api/v1/apps/{app}/domain", base(&cfg)))
                .bearer_auth(&cfg.token)
                .json(&serde_json::json!({"domain": host}))
                .send()?;
            handle(resp)
        }
        Cmd::Git { app, repo } => {
            let cfg = load_config()?;
            let resp = client()?
                .post(format!("{}/api/v1/apps/{app}/repo", base(&cfg)))
                .bearer_auth(&cfg.token)
                .json(&serde_json::json!({"repo": repo}))
                .send()?;
            handle(resp)
        }
        Cmd::GitInit { app } => {
            let cfg = load_config()?;
            let resp = client()?
                .post(format!("{}/api/v1/apps/{app}/git-init", base(&cfg)))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
        Cmd::DeployGit { .. } => unreachable!("deploy-git is async"),
        Cmd::Deploys { app } => {
            let cfg = load_config()?;
            let resp = client()?
                .get(format!("{}/api/v1/apps/{app}/deploys", base(&cfg)))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
        Cmd::Rollback { app, deploy } => {
            let cfg = load_config()?;
            let resp = client()?
                .post(format!(
                    "{}/api/v1/apps/{app}/rollback/{deploy}",
                    base(&cfg)
                ))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
        Cmd::Redeploy { app } => {
            let cfg = load_config()?;
            let resp = client()?
                .post(format!("{}/api/v1/apps/{app}/redeploy", base(&cfg)))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
        Cmd::Rm { app } => {
            let cfg = load_config()?;
            let resp = client()?
                .delete(format!("{}/api/v1/apps/{app}", base(&cfg)))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
    }
}

pub async fn deploy_git(app: String, data: String) -> Result<()> {
    let data = PathBuf::from(expand_home(&data));
    let db = crate::db::Db::open(&data.join("dok.db"))?;
    let st = crate::state::AppState::new(db, data, None, String::new());
    let out = crate::api::deploy_from_repo(&st, &app).await?;
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

fn expand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest).to_string_lossy().to_string();
        }
    }
    p.to_string()
}

fn push(args: PushArgs) -> Result<()> {
    let cfg = load_config()?;
    let dir = args.dir.canonicalize().context("project dir")?;
    let app = args
        .app
        .or_else(|| {
            std::fs::read_to_string(dir.join("dok.toml"))
                .ok()
                .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
                .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
        })
        .or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .context("cannot infer app name; use --app")?;
    let app = slug(&app)?;

    let tmp = std::env::temp_dir().join(format!("dok-{}.tar.gz", uuid::Uuid::new_v4().simple()));
    pack(&dir, &tmp)?;

    let mut url = format!("{}/api/v1/apps/{}/deploy", base(&cfg), app);
    let mut q = Vec::new();
    if let Some(p) = args.port {
        q.push(format!("port={p}"));
    }
    if let Some(d) = &args.domain {
        q.push(format!("domain={d}"));
    }
    if !q.is_empty() {
        url.push('?');
        url.push_str(&q.join("&"));
    }

    let file = std::fs::File::open(&tmp)?;
    let resp = client()?
        .post(url)
        .bearer_auth(&cfg.token)
        .header("content-type", "application/gzip")
        .body(reqwest::blocking::Body::new(file))
        .send();
    let _ = std::fs::remove_file(&tmp);
    handle(resp?)
}

fn slug(raw: &str) -> Result<String> {
    let mut out = String::new();
    let mut dash = false;
    for c in raw.to_ascii_lowercase().chars() {
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
    if out.is_empty() || out.len() > 63 {
        bail!("cannot derive app name from {raw:?}; use --app");
    }
    Ok(out)
}

fn pack(dir: &Path, out: &Path) -> Result<()> {
    let f = std::fs::File::create(out)?;
    let enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
    let mut tb = tar::Builder::new(enc);
    let mut walk = WalkBuilder::new(dir)
        .standard_filters(true)
        .hidden(false)
        .build();
    let mut count = 0usize;
    while let Some(item) = walk.next() {
        let entry = item?;
        let path = entry.path();
        if path == dir || entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        if path
            .components()
            .any(|c| matches!(c.as_os_str().to_str(), Some(".git" | "node_modules" | "target" | ".venv" | "__pycache__")))
        {
            continue;
        }
        let rel = path.strip_prefix(dir)?;
        tb.append_path_with_name(path, rel)?;
        count += 1;
    }
    tb.finish()?;
    let mut inner = tb.into_inner()?;
    inner.flush()?;
    if count == 0 {
        bail!("nothing to upload (empty directory or everything ignored)");
    }
    Ok(())
}
