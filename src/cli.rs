use crate::term;
use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "dok", version, about = "Tiny self-hosted deploy platform")]
pub struct Cli {
    /// Print raw JSON instead of the human-readable output (for scripts).
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Run the server.
    Serve(ServeArgs),
    /// Link this CLI to a dok server with an account API token.
    Login {
        server: String,
        /// Avoid passing the token in argv; prefer DOK_TOKEN or --token-file.
        token: Option<String>,
        /// Read the token from a file (for example ~/.config/dok/token).
        #[arg(long)]
        token_file: Option<PathBuf>,
    },
    /// Upload the current directory and deploy it.
    Push(PushArgs),
    /// List apps.
    Apps,
    /// List projects, or create/remove one.
    Projects {
        #[command(subcommand)]
        cmd: Option<ProjectsCmd>,
    },
    /// Show build and runtime logs (-f to follow).
    Logs {
        app: String,
        #[arg(short, long)]
        follow: bool,
        #[arg(long, default_value = "production")]
        env: String,
        /// Select one deploy id (useful for preview containers).
        #[arg(long)]
        deploy: Option<String>,
    },
    /// Set environment variables (KEY=VALUE ...). `env pull <app>` prints them.
    Env {
        app: String,
        #[arg(value_name = "KEY=VALUE")]
        vars: Vec<String>,
        /// Environment to write: production, preview or development.
        #[arg(short, long, default_value = "production")]
        env: String,
        /// Store the values as write-only secrets.
        #[arg(long)]
        secret: bool,
    },
    /// Point a domain at an app.
    Domain { app: String, host: String },
    /// List or remove project domains.
    Domains {
        #[command(subcommand)]
        cmd: DomainsCmd,
    },
    /// Set the git repo used by webhooks / redeploy.
    Git { app: String, repo: String },
    /// Redeploy from the configured repo.
    Redeploy { app: String },
    /// List recent deploys (id, status, age).
    Deploys { app: String },
    /// Roll back to an earlier deploy id.
    Rollback { app: String, deploy: String },
    /// Promote an earlier deploy onto production.
    Promote { app: String, deploy: String },
    /// Print the public URL of a project.
    Open { app: String },
    /// Create a bare repo on the server and print the git remote.
    GitInit { app: String },
    /// Internal: deploy a pushed git repo (called by the post-receive hook).
    #[command(hide = true)]
    DeployGit {
        app: String,
        #[arg(long)]
        data: String,
        /// Ref being pushed (refs/heads/<branch>), supplied by post-receive.
        #[arg(long = "ref")]
        r#ref: Option<String>,
    },
    /// Delete an app and stop its container.
    Rm { app: String },
    /// Show the user and teams behind the saved token.
    Whoami,
    /// List teams.
    Teams,
    /// Team members and invites.
    Team {
        #[command(subcommand)]
        cmd: TeamCmd,
    },
    /// Show deploy, domain and runtime metrics for a project.
    Stats { app: String },
    /// Show or change per-project proxy settings.
    Proxy {
        #[command(subcommand)]
        cmd: ProxyCmd,
    },
    /// List or renew TLS certificates.
    Certs {
        #[command(subcommand)]
        cmd: CertsCmd,
    },
    /// Show recent proxy access log entries.
    AccessLog {
        app: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Show or update GitHub webhook settings.
    Webhook {
        app: String,
        #[arg(long)]
        secret: Option<String>,
        #[arg(long)]
        branch: Option<String>,
    },
    /// Manage personal API tokens.
    Tokens {
        #[command(subcommand)]
        cmd: TokensCmd,
    },
}

#[derive(Subcommand)]
pub enum TokensCmd {
    /// List your tokens (hashes, never the secret).
    List,
    /// Create a token and print it once.
    Create {
        #[arg(long, default_value = "cli")]
        name: String,
    },
    /// Revoke a token by its id/hash from `dok tokens list`.
    Revoke { hash: String },
}

#[derive(Subcommand)]
pub enum ProxyCmd {
    /// Print the current proxy settings.
    Show { app: String },
    /// Update settings; unspecified flags are kept.
    Set {
        app: String,
        /// Response header, e.g. --header "X-Frame-Options: DENY" (repeatable).
        #[arg(long = "header")]
        headers: Vec<String>,
        #[arg(long)]
        basic_user: Option<String>,
        #[arg(long)]
        basic_password: Option<String>,
        /// off, www-to-apex or apex-to-www.
        #[arg(long)]
        redirect: Option<String>,
        #[arg(long)]
        compression: bool,
        #[arg(long = "no-compression")]
        no_compression: bool,
        #[arg(long)]
        force_https: bool,
        #[arg(long = "no-force-https")]
        no_force_https: bool,
        #[arg(long)]
        max_body_mb: Option<u64>,
    },
}

#[derive(Subcommand)]
pub enum CertsCmd {
    /// List certificates for projects you can read.
    List,
    /// Request a fresh certificate for a host.
    Renew { host: String },
}

#[derive(Subcommand)]
pub enum ProjectsCmd {
    /// Create a project.
    Create {
        name: String,
        /// Team slug or id; defaults to the first writable team.
        #[arg(long)]
        team: Option<String>,
    },
    /// Delete a project and stop its containers.
    Rm { name: String },
}

#[derive(Subcommand)]
pub enum TeamCmd {
    /// List members of a team.
    Members { team: String },
    /// Create an invite link (default role: member).
    Invite {
        team: String,
        #[arg(long, default_value = "member")]
        role: String,
    },
    /// List invite links.
    Invites { team: String },
    /// Revoke an invite link.
    Revoke { team: String, token: String },
    /// Create a team.
    Create { name: String },
    /// Change a member role.
    Role {
        team: String,
        user: String,
        role: String,
    },
    /// Remove a member.
    Remove { team: String, user: String },
}

#[derive(Subcommand)]
pub enum DomainsCmd {
    /// List domains for a project.
    List { app: String },
    /// Check whether a domain's DNS points at this server.
    Check { app: String, host: String },
    /// Remove a domain from a project.
    Rm { app: String, host: String },
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
    /// Use the Let's Encrypt staging directory (untrusted certs, for testing).
    #[arg(long, env = "DOK_ACME_STAGING")]
    pub acme_staging: bool,
    /// Deprecated no-op: production is now the default.
    #[arg(long = "acme-prod", env = "DOK_ACME_PROD", hide = true)]
    pub acme_prod_legacy: bool,
    /// Built-in edge: bind :80/:443, issue certs for every DB host, redirect HTTP to HTTPS.
    #[arg(long, env = "DOK_EDGE")]
    pub edge: bool,
    /// CIDR of an additional trusted reverse proxy (X-Forwarded-Proto).
    #[arg(long = "trusted-proxy", env = "DOK_TRUSTED_PROXY")]
    pub trusted_proxy: Vec<String>,
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
    /// Deployment environment: production or preview.
    #[arg(long, default_value = "production")]
    pub env: String,
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
    if let (Ok(server), Ok(token)) = (std::env::var("DOK_SERVER"), std::env::var("DOK_TOKEN")) {
        return Ok(Config { server, token });
    }
    let p = config_path()?;
    let s = std::fs::read_to_string(&p).with_context(|| {
        format!(
            "no config at {}; run `dok login <url>` with DOK_TOKEN set or use --token-file",
            p.display()
        )
    })?;
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

fn scheme_of(url: &str) -> &str {
    if url.starts_with("https://") {
        "https"
    } else {
        "http"
    }
}

/// Split `http://host:port/base` into (scheme, host[:port]).
fn split_server(url: &str) -> (&str, &str) {
    let scheme = scheme_of(url);
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let host = rest.split('/').next().unwrap_or(rest);
    (scheme, host)
}

fn server_error(status: reqwest::StatusCode, text: &str) -> String {
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return "not signed in (HTTP 401). Run `dok login <server> <token>` or `dok whoami` to check your token.".to_string();
    }
    let detail = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| text.trim().to_string());
    if detail.is_empty() {
        format!("server returned {status}")
    } else {
        format!("{detail} (HTTP {})", status.as_u16())
    }
}

fn handle(resp: reqwest::blocking::Response) -> Result<()> {
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        bail!("{}", server_error(status, &text));
    }
    if term::json() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            println!("{}", serde_json::to_string_pretty(&value)?);
        } else {
            println!("{text}");
        }
        return Ok(());
    }
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => render(&value),
        Err(_) => print!("{text}"),
    }
    Ok(())
}

fn field<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
    value.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

fn number(value: &serde_json::Value, key: &str) -> Option<i64> {
    value.get(key).and_then(|v| v.as_i64())
}

fn flag(value: &serde_json::Value, key: &str) -> bool {
    value.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn shorten(id: &str, len: usize) -> String {
    let head: String = id.chars().take(len).collect();
    if id.chars().count() > len {
        format!("{head}…")
    } else {
        head
    }
}

fn short_id(id: &str) -> String {
    shorten(id, 8)
}

fn status_colored(status: &str) -> String {
    match status {
        "ok" | "running" | "active" | "issued" | "ready" | "deployed" => term::green(status),
        "failed" | "error" | "unhealthy" | "revoked" => term::red(status),
        "building" | "pending" | "queued" | "new" | "renewing" | "paused" => term::yellow(status),
        "stopped" | "sleeping" | "used" | "expired" => term::dim(status),
        other => other.to_string(),
    }
}

fn on_off(on: bool) -> String {
    if on {
        term::green("on")
    } else {
        term::dim("off")
    }
}

fn expires_in(ts: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(ts);
    let left = ts - now;
    if left <= 0 {
        term::dim("expired")
    } else if left < 3600 {
        format!("in {}m", left / 60)
    } else if left < 86_400 {
        format!("in {}h", left / 3600)
    } else {
        format!("in {}d", left / 86_400)
    }
}

fn render_scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => term::dim("-"),
        serde_json::Value::String(s) if s.is_empty() => term::dim("-"),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(true) => term::green("yes"),
        serde_json::Value::Bool(false) => term::dim("no"),
        serde_json::Value::Array(items) if items.is_empty() => term::dim("none"),
        serde_json::Value::Array(items) => items
            .iter()
            .map(render_scalar)
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    }
}

fn detail_row(label: &str, value: impl std::fmt::Display) {
    println!("  {}  {value}", term::dim(&format!("{label:<12}")));
}

fn print_dns(dns: &serde_json::Value) {
    let hint = field(dns, "hint");
    if hint.is_empty() {
        return;
    }
    let marker = match field(dns, "status") {
        "ok" => term::green("✓"),
        "missing" | "mismatch" => term::yellow("!"),
        _ => term::dim("·"),
    };
    println!("  {} {} {hint}", term::dim("DNS"), marker);
}

fn apps_table(items: &[serde_json::Value]) {
    let rows = items
        .iter()
        .map(|app| {
            let kind = field(app, "kind");
            let status = field(app, "status");
            let url = field(app, "url");
            vec![
                term::bold(field(app, "name")),
                if kind.is_empty() {
                    term::dim("-")
                } else {
                    kind.to_string()
                },
                if status.is_empty() {
                    term::dim("-")
                } else {
                    status_colored(status)
                },
                if url.is_empty() {
                    term::dim("-")
                } else {
                    term::cyan(url)
                },
            ]
        })
        .collect::<Vec<_>>();
    term::print_table(&["NAME", "KIND", "STATUS", "URL"], &rows);
    if !items.is_empty() {
        println!("{}", term::dim(&format!("{} project(s)", items.len())));
    }
}

fn deployments_table(items: &[serde_json::Value]) {
    let rows = items
        .iter()
        .map(|item| {
            let id = short_id(field(item, "id"));
            let status = field(item, "status");
            let env = field(item, "env");
            let age = number(item, "created_at")
                .map(term::human_age)
                .unwrap_or_else(|| term::dim("-"));
            let mut detail = String::new();
            if let Some(sha) = item
                .get("commit_sha")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                detail.push_str(&format!("{} ", shorten(sha, 7)));
            }
            if let Some(msg) = item
                .get("commit_message")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                detail.push_str(&msg.chars().take(40).collect::<String>());
            }
            if detail.is_empty() {
                let url = field(item, "url");
                detail = if url.is_empty() {
                    term::dim("-")
                } else {
                    url.to_string()
                };
            }
            vec![
                term::bold(&id),
                status_colored(status),
                if env.is_empty() {
                    term::dim("-")
                } else {
                    env.to_string()
                },
                age,
                detail,
            ]
        })
        .collect::<Vec<_>>();
    term::print_table(&["ID", "STATUS", "ENV", "AGE", "DETAIL"], &rows);
}

fn domains_table(obj: &serde_json::Map<String, serde_json::Value>, items: &[serde_json::Value]) {
    let primary = obj.get("primary").and_then(|v| v.as_str()).unwrap_or("");
    let rows = items
        .iter()
        .map(|item| {
            let host = item
                .as_str()
                .map(str::to_string)
                .or_else(|| {
                    item.get("host")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_default();
            let marker = if !primary.is_empty() && host == primary {
                term::green("primary")
            } else {
                term::dim("-")
            };
            vec![term::cyan(&host), marker]
        })
        .collect::<Vec<_>>();
    term::print_table(&["HOST", "PRIMARY"], &rows);
}

fn certificates_table(items: &[serde_json::Value]) {
    let rows = items
        .iter()
        .map(|item| {
            let expires = number(item, "expires")
                .map(expires_in)
                .unwrap_or_else(|| term::dim("-"));
            let note = field(item, "note");
            vec![
                term::cyan(field(item, "host")),
                field(item, "project").to_string(),
                status_colored(field(item, "status")),
                expires,
                if note.is_empty() {
                    term::dim("-")
                } else {
                    term::dim(note)
                },
            ]
        })
        .collect::<Vec<_>>();
    term::print_table(&["HOST", "PROJECT", "STATUS", "EXPIRES", "NOTE"], &rows);
}

fn members_table(items: &[serde_json::Value]) {
    let rows = items
        .iter()
        .map(|item| {
            let role = field(item, "role");
            let role = match role {
                "owner" => term::magenta(role),
                "admin" => term::yellow(role),
                other => other.to_string(),
            };
            vec![
                term::bold(field(item, "name")),
                term::dim(field(item, "email")),
                role,
            ]
        })
        .collect::<Vec<_>>();
    term::print_table(&["NAME", "EMAIL", "ROLE"], &rows);
}

fn invites_table(items: &[serde_json::Value]) {
    let rows = items
        .iter()
        .map(|item| {
            let used = flag(item, "used");
            let expired = flag(item, "expired");
            let status = if used {
                term::dim("used")
            } else if expired {
                term::dim("expired")
            } else {
                term::green("active")
            };
            let expires = number(item, "expires_at")
                .map(expires_in)
                .unwrap_or_else(|| term::dim("-"));
            vec![
                field(item, "role").to_string(),
                status,
                expires,
                term::cyan(field(item, "link")),
            ]
        })
        .collect::<Vec<_>>();
    term::print_table(&["ROLE", "STATUS", "EXPIRES", "LINK"], &rows);
}

fn tokens_table(items: &[serde_json::Value]) {
    let rows = items
        .iter()
        .map(|item| {
            let created = number(item, "created_at")
                .map(term::human_age)
                .unwrap_or_else(|| term::dim("-"));
            vec![
                term::bold(field(item, "name")),
                term::dim(&shorten(field(item, "id"), 12)),
                created,
            ]
        })
        .collect::<Vec<_>>();
    term::print_table(&["NAME", "ID", "CREATED"], &rows);
}

fn http_status(code: i64) -> String {
    let text = code.to_string();
    match code {
        200..=299 => term::green(&text),
        300..=399 => term::cyan(&text),
        400..=499 => term::yellow(&text),
        500..=599 => term::red(&text),
        _ => text,
    }
}

fn access_table(items: &[serde_json::Value], total: Option<i64>) {
    let rows = items
        .iter()
        .map(|item| {
            let status = number(item, "status").map(http_status).unwrap_or_default();
            let time = number(item, "ts")
                .map(term::human_age)
                .unwrap_or_else(|| term::dim("-"));
            let latency = number(item, "latency_ms")
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_else(|| "-".into());
            vec![
                term::dim(&time),
                status,
                field(item, "method").to_string(),
                field(item, "path").to_string(),
                term::dim(field(item, "ip")),
                latency,
            ]
        })
        .collect::<Vec<_>>();
    term::print_table(&["TIME", "STATUS", "METHOD", "PATH", "IP", "LAT"], &rows);
    if let Some(total) = total {
        println!("{}", term::dim(&format!("{total} request(s) in the log")));
    }
}

fn teams_table(items: &[serde_json::Value]) {
    let rows = items
        .iter()
        .map(|item| {
            vec![
                term::bold(field(item, "name")),
                field(item, "slug").to_string(),
                term::dim(&shorten(field(item, "id"), 12)),
            ]
        })
        .collect::<Vec<_>>();
    term::print_table(&["NAME", "SLUG", "ID"], &rows);
}

fn whoami_view(obj: &serde_json::Map<String, serde_json::Value>) {
    let user = obj.get("user").cloned().unwrap_or(serde_json::Value::Null);
    let name = field(&user, "name");
    let email = field(&user, "email");
    let label = if name.is_empty() {
        email.to_string()
    } else {
        name.to_string()
    };
    println!(
        "{} {}",
        term::bold(&label),
        term::dim(&format!("<{email}>"))
    );
    if let Some(teams) = obj.get("teams").and_then(|v| v.as_array()) {
        let names = teams
            .iter()
            .map(|team| format!("{} ({})", field(team, "name"), field(team, "slug")))
            .collect::<Vec<_>>();
        detail_row("teams", &names.join(" · "));
    }
}

fn proxy_view(obj: &serde_json::Map<String, serde_json::Value>) {
    let value = serde_json::Value::Object(obj.clone());
    detail_row("compression", on_off(flag(&value, "compression")));
    detail_row("force https", on_off(flag(&value, "force_https")));
    let redirect = field(&value, "redirect");
    detail_row(
        "redirect",
        if redirect.is_empty() {
            term::dim("off")
        } else {
            redirect.to_string()
        },
    );
    let basic_auth = if flag(&value, "basic_auth_configured") {
        format!(
            "{} ({})",
            term::green("on"),
            field(&value, "basic_auth_user")
        )
    } else {
        term::dim("off")
    };
    detail_row("basic auth", basic_auth);
    let max_body = number(&value, "max_body_mb")
        .map(|mb| format!("{mb} MB"))
        .unwrap_or_else(|| "unlimited".to_string());
    detail_row("max body", max_body);
    let headers = obj
        .get("headers")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    if let Some(pair) = item.as_array() {
                        pair.iter()
                            .filter_map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join(": ")
                    } else {
                        render_scalar(item)
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    detail_row(
        "headers",
        if headers.is_empty() {
            term::dim("none")
        } else {
            headers.join(" · ")
        },
    );
}

fn webhook_view(obj: &serde_json::Map<String, serde_json::Value>) {
    let value = serde_json::Value::Object(obj.clone());
    let status = if flag(&value, "configured") {
        term::green("configured")
    } else {
        term::dim("not configured")
    };
    detail_row("status", status);
    let branch = field(&value, "branch");
    detail_row(
        "branch",
        if branch.is_empty() {
            term::dim("-")
        } else {
            branch.to_string()
        },
    );
    let secret = if value.get("secret").map(|v| !v.is_null()).unwrap_or(false) {
        term::green("set")
    } else {
        term::dim("not set")
    };
    detail_row("secret", secret);
}

fn stats_view(obj: &serde_json::Map<String, serde_json::Value>) {
    let value = serde_json::Value::Object(obj.clone());
    println!(
        "{} {} {}",
        term::bold(field(&value, "project")),
        term::dim("·"),
        term::dim(field(&value, "kind")),
    );
    println!("  {}", status_colored(field(&value, "status")));
    if let Some(deployments) = value.get("deployments").and_then(|v| v.as_object()) {
        let total = deployments
            .get("total")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let production = deployments
            .get("production")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let preview = deployments
            .get("preview")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        detail_row(
            "deployments",
            &format!("{total} total · {production} production · {preview} preview"),
        );
    }
    detail_row(
        "domains",
        &number(&value, "domains").unwrap_or(0).to_string(),
    );
    if let Some(env) = value.get("env_keys").and_then(|v| v.as_object()) {
        let parts = ["production", "preview", "development"]
            .iter()
            .map(|key| {
                format!(
                    "{key} {}",
                    env.get(*key).and_then(|v| v.as_i64()).unwrap_or(0)
                )
            })
            .collect::<Vec<_>>();
        detail_row("env keys", &parts.join(" · "));
    }
    if let Some(latest) = value.get("latest").filter(|v| !v.is_null()) {
        let age = number(latest, "created_at")
            .map(term::human_age)
            .unwrap_or_else(|| term::dim("-"));
        detail_row(
            "latest",
            &format!(
                "{} · {} · {age}",
                short_id(field(latest, "id")),
                status_colored(field(latest, "status"))
            ),
        );
    }
    if let Some(traffic) = value.get("traffic").and_then(|v| v.as_object()) {
        let requests = traffic
            .get("requests")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let errors = traffic
            .get("errors_5xx")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let p95 = traffic
            .get("p95_latency_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let window = traffic
            .get("window_secs")
            .and_then(|v| v.as_i64())
            .map(term::human_window)
            .unwrap_or_else(|| "1h".into());
        detail_row(
            "traffic",
            &format!("{requests} req · {errors} 5xx · p95 {p95}ms ({window})"),
        );
    }
    if let Some(runtime) = value.get("runtime").and_then(|v| v.as_object()) {
        let container = runtime
            .get("container")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let note = runtime.get("note").and_then(|v| v.as_str()).unwrap_or("");
        let mut text = container.to_string();
        if let Some(cpu) = runtime.get("cpu").and_then(|v| v.as_f64()) {
            text.push_str(&format!(" · cpu {cpu:.2}"));
        }
        if let Some(mem) = runtime.get("memory").and_then(|v| v.as_f64()) {
            text.push_str(&format!(" · mem {mem:.0} MB"));
        }
        if !note.is_empty() {
            text.push_str(&format!(" · {note}"));
        }
        detail_row("runtime", text.trim());
    }
}

fn deploy_outcome(value: &serde_json::Value) {
    let mut meta = Vec::new();
    let env = field(value, "env");
    let kind = field(value, "kind");
    if !env.is_empty() {
        meta.push(env.to_string());
    }
    if !kind.is_empty() {
        meta.push(kind.to_string());
    }
    let suffix = if meta.is_empty() {
        String::new()
    } else {
        format!(" · {}", meta.join(" · "))
    };
    term::success(&format!(
        "Deployed {}{suffix}",
        term::bold(field(value, "app"))
    ));
    let url = field(value, "url");
    if !url.is_empty() {
        println!("  {}", term::cyan(url));
    }
}

fn render_array(items: &[serde_json::Value]) {
    if items.is_empty() {
        println!("  {}", term::dim("(none)"));
        return;
    }
    for item in items {
        match item {
            serde_json::Value::String(s) => println!("  {s}"),
            serde_json::Value::Object(obj) => {
                let pairs = obj
                    .iter()
                    .map(|(key, value)| {
                        format!("{} {}", term::dim(&format!("{key}:")), render_scalar(value))
                    })
                    .collect::<Vec<_>>();
                println!("  {}", pairs.join("  "));
            }
            other => println!("  {other}"),
        }
    }
}

fn render(value: &serde_json::Value) {
    let Some(obj) = value.as_object() else {
        match value {
            serde_json::Value::String(s) => println!("{s}"),
            serde_json::Value::Array(items) => render_array(items),
            other => println!("{other}"),
        }
        return;
    };

    for key in ["apps", "projects"] {
        if let Some(items) = obj.get(key).and_then(|v| v.as_array()) {
            apps_table(items);
            return;
        }
    }
    if let Some(items) = obj
        .get("deployments")
        .and_then(|v| v.as_array())
        .or_else(|| obj.get("deploys").and_then(|v| v.as_array()))
    {
        deployments_table(items);
        return;
    }
    if obj.get("project").is_some()
        && obj
            .get("deployments")
            .map(|v| v.is_object())
            .unwrap_or(false)
    {
        stats_view(obj);
        return;
    }
    if let Some(items) = obj.get("domains").and_then(|v| v.as_array()) {
        domains_table(obj, items);
        return;
    }
    if let Some(items) = obj.get("certificates").and_then(|v| v.as_array()) {
        certificates_table(items);
        return;
    }
    if let Some(items) = obj.get("members").and_then(|v| v.as_array()) {
        members_table(items);
        return;
    }
    if let Some(items) = obj.get("invites").and_then(|v| v.as_array()) {
        invites_table(items);
        return;
    }
    if let Some(items) = obj.get("tokens").and_then(|v| v.as_array()) {
        tokens_table(items);
        return;
    }
    if let Some(items) = obj.get("requests").and_then(|v| v.as_array()) {
        access_table(items, number(value, "total"));
        return;
    }
    if obj.contains_key("user") && obj.get("teams").map(|v| v.is_array()).unwrap_or(false) {
        whoami_view(obj);
        return;
    }
    if let Some(items) = obj.get("teams").and_then(|v| v.as_array()) {
        teams_table(items);
        return;
    }
    if obj.contains_key("basic_auth_configured") && obj.contains_key("force_https") {
        proxy_view(obj);
        return;
    }
    if obj.contains_key("configured") {
        webhook_view(obj);
        return;
    }
    if obj.contains_key("link") && obj.contains_key("role") && obj.contains_key("token") {
        term::success(&format!(
            "Invite created for role {}",
            term::yellow(field(value, "role"))
        ));
        println!("  {}", term::cyan(field(value, "link")));
        println!("  {}", term::dim("share the link; it expires in 7 days"));
        return;
    }
    if obj.contains_key("login") && obj.contains_key("token") {
        term::success(&format!(
            "Token {} created",
            term::bold(field(value, "name"))
        ));
        println!("  {}", term::cyan(field(value, "token")));
        println!("  {}", term::dim("store it now; it is not shown again"));
        return;
    }
    if obj.contains_key("remote") || obj.contains_key("ssh_remote") {
        term::success(&format!(
            "Git remote for {}",
            term::bold(field(value, "app"))
        ));
        let remote = if field(value, "ssh_remote").is_empty() {
            field(value, "remote")
        } else {
            field(value, "ssh_remote")
        };
        println!("  {}", term::cyan(remote));
        let hint = field(value, "hint");
        if !hint.is_empty() {
            println!("  {}", term::dim(hint));
        }
        return;
    }
    if obj.contains_key("app") && obj.contains_key("deploy_id") {
        deploy_outcome(value);
        return;
    }
    if obj.contains_key("domain") && flag(value, "ok") {
        term::success(&format!(
            "Domain {} now points at this project",
            term::cyan(field(value, "domain"))
        ));
        if let Some(dns) = obj.get("dns") {
            print_dns(dns);
        }
        return;
    }
    if let Some(dns) = obj.get("dns") {
        print_dns(dns);
        return;
    }
    if let Some(removed) = obj.get("removed").and_then(|v| v.as_str()) {
        let mut message = format!("Removed {}", term::bold(removed));
        if let Some(env) = obj.get("env").and_then(|v| v.as_str()) {
            message = format!("Removed {} from the {env} environment", term::bold(removed));
        }
        if flag(value, "primary") {
            message.push_str(" (was the primary domain)");
        }
        term::success(&message);
        return;
    }
    if let Some(deleted) = obj.get("deleted").and_then(|v| v.as_str()) {
        let message = if let Some(env) = obj.get("env").and_then(|v| v.as_str()) {
            format!("Removed {} from the {env} environment", term::bold(deleted))
        } else {
            format!("Deleted {}", term::bold(deleted))
        };
        term::success(&message);
        return;
    }
    if flag(value, "ok") {
        if let Some(env) = obj.get("env").and_then(|v| v.as_str()) {
            if let Some(keys) = obj.get("keys").and_then(|v| v.as_array()) {
                let names = keys.iter().filter_map(|k| k.as_str()).collect::<Vec<_>>();
                let extra = if names.is_empty() {
                    String::new()
                } else {
                    format!(": {}", names.join(", "))
                };
                term::success(&format!(
                    "Saved {} variable(s) to {env}{extra}",
                    names.len()
                ));
                return;
            }
        }
        if let Some(role) = obj.get("role").and_then(|v| v.as_str()) {
            term::success(&format!("Role set to {}", term::yellow(role)));
            return;
        }
        if let Some(repo) = obj.get("repo").and_then(|v| v.as_str()) {
            term::success(&format!("Repository set to {}", term::cyan(repo)));
            return;
        }
        if let Some(host) = obj.get("host").and_then(|v| v.as_str()) {
            term::success(&format!(
                "Certificate renewal started for {} ({})",
                term::cyan(host),
                status_colored(field(value, "status"))
            ));
            return;
        }
        term::success("Done");
        return;
    }
    if obj.contains_key("team_id") && obj.contains_key("kind") {
        term::success(&format!("Project {}", term::bold(field(value, "name"))));
        let mut details = Vec::new();
        if !field(value, "kind").is_empty() {
            details.push(format!("kind {}", field(value, "kind")));
        }
        if !field(value, "status").is_empty() {
            details.push(format!("status {}", status_colored(field(value, "status"))));
        }
        if !details.is_empty() {
            println!("  {}", details.join(" · "));
        }
        let url = field(value, "url");
        if !url.is_empty() {
            println!("  {}", term::cyan(url));
        }
        return;
    }

    for (key, value) in obj {
        println!(
            "  {}  {}",
            term::dim(&format!("{key:<16}")),
            render_scalar(value)
        );
    }
}

pub fn run(cmd: Cmd, json: bool) -> Result<()> {
    term::set_json(json);
    match cmd {
        Cmd::Serve(_) => unreachable!("serve is async"),
        Cmd::Login {
            server,
            token,
            token_file,
        } => {
            let token = match (token, token_file) {
                (Some(t), _) if !t.trim().is_empty() => t,
                (_, Some(path)) => std::fs::read_to_string(&path)
                    .with_context(|| format!("read token file {}", path.display()))?
                    .trim()
                    .to_string(),
                _ => std::env::var("DOK_TOKEN")
                    .ok()
                    .filter(|t| !t.trim().is_empty())
                    .context(
                        "missing token: pass it as an argument, set DOK_TOKEN, or use --token-file",
                    )?,
            };
            if token.trim().is_empty() {
                bail!("token is empty");
            }
            let cfg = Config {
                server: server.trim_end_matches('/').to_string(),
                token,
            };
            save_config(&cfg)?;
            let path = config_path()?;
            if term::json() {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "server": cfg.server,
                        "config": path.display().to_string(),
                    }))?
                );
            } else {
                term::success(&format!("Linked to {}", term::cyan(&cfg.server)));
                detail_row("config", &term::dim(&path.display().to_string()));
                println!(
                    "  {}",
                    term::dim("next: dok whoami to verify the account, then dok push <dir>")
                );
            }
            Ok(())
        }
        Cmd::Apps => {
            let cfg = load_config()?;
            let resp = client()?
                .get(format!("{}/api/v1/projects", base(&cfg)))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
        Cmd::Projects { cmd } => {
            let cfg = load_config()?;
            match cmd {
                None => {
                    let resp = client()?
                        .get(format!("{}/api/v1/projects", base(&cfg)))
                        .bearer_auth(&cfg.token)
                        .send()?;
                    handle(resp)
                }
                Some(ProjectsCmd::Create { name, team }) => {
                    let resp = client()?
                        .post(format!("{}/api/v1/projects", base(&cfg)))
                        .bearer_auth(&cfg.token)
                        .json(&serde_json::json!({"name": name, "team": team}))
                        .send()?;
                    handle(resp)
                }
                Some(ProjectsCmd::Rm { name }) => {
                    let resp = client()?
                        .delete(format!("{}/api/v1/projects/{name}", base(&cfg)))
                        .bearer_auth(&cfg.token)
                        .send()?;
                    handle(resp)
                }
            }
        }
        Cmd::Push(args) => push(args),
        Cmd::Logs {
            app,
            follow,
            env,
            deploy,
        } => {
            let cfg = load_config()?;
            let deploy_q = deploy
                .as_deref()
                .map(|d| format!("&deploy={d}"))
                .unwrap_or_default();
            if follow {
                if !term::json() && term::stderr_tty() {
                    eprintln!(
                        "{} following {app} logs ({env}) · ctrl-c to stop",
                        term::arrow()
                    );
                }
                let mut resp = client()?
                    .get(format!(
                        "{}/api/v1/projects/{app}/logs/stream?env={env}{deploy_q}",
                        base(&cfg)
                    ))
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
                .get(format!(
                    "{}/api/v1/projects/{app}/logs?env={env}{deploy_q}",
                    base(&cfg)
                ))
                .bearer_auth(&cfg.token)
                .send()?;
            if !resp.status().is_success() {
                return handle(resp);
            }
            let v: serde_json::Value = resp.json()?;
            if term::json() {
                println!("{}", serde_json::to_string_pretty(&v)?);
                return Ok(());
            }
            let build = v["build"].as_str().unwrap_or("").trim_end();
            let runtime = v["runtime"].as_str().unwrap_or("").trim_end();
            println!("{}", term::bold("== build =="));
            if build.is_empty() {
                println!("{}", term::dim("(no build output)"));
            } else {
                println!("{build}");
            }
            println!("{}", term::bold("== runtime =="));
            if runtime.is_empty() {
                println!("{}", term::dim("(no runtime output)"));
            } else {
                println!("{runtime}");
            }
            Ok(())
        }
        Cmd::Env {
            app,
            vars,
            env,
            secret,
        } => {
            if app == "rm" {
                let Some(target) = vars.first() else {
                    bail!("usage: dok env rm <app> <KEY> [env]");
                };
                let Some(key) = vars.get(1) else {
                    bail!("usage: dok env rm <app> <KEY> [env]");
                };
                let env = vars.get(2).map(String::as_str).unwrap_or("production");
                let cfg = load_config()?;
                let resp = client()?
                    .delete(format!(
                        "{}/api/v1/projects/{target}/env/{env}/{key}",
                        base(&cfg)
                    ))
                    .bearer_auth(&cfg.token)
                    .send()?;
                return handle(resp);
            }
            // `dok env pull <app>` — keep the historical `dok env <app> K=V` form.
            if app == "pull" {
                let Some(target) = vars.first() else {
                    bail!("usage: dok env pull <app>");
                };
                let cfg = load_config()?;
                let resp = client()?
                    .get(format!(
                        "{}/api/v1/projects/{target}/env/{}",
                        base(&cfg),
                        vars.get(1).map(String::as_str).unwrap_or(env.as_str())
                    ))
                    .bearer_auth(&cfg.token)
                    .send()?;
                if !resp.status().is_success() {
                    return handle(resp);
                }
                let v: serde_json::Value = resp.json()?;
                if let Some(map) = v["env"].as_object() {
                    for (k, val) in map {
                        println!("{}={}", term::bold(k), val.as_str().unwrap_or(""));
                    }
                }
                return Ok(());
            }
            let cfg = load_config()?;
            let mut map = serde_json::Map::new();
            for v in vars {
                let (k, val) = v
                    .split_once('=')
                    .with_context(|| format!("expected KEY=VALUE, got {v}"))?;
                map.insert(
                    k.to_string(),
                    if secret {
                        serde_json::json!({"value": val, "secret": true})
                    } else {
                        serde_json::Value::String(val.to_string())
                    },
                );
            }
            let resp = client()?
                .put(format!("{}/api/v1/projects/{app}/env/{env}", base(&cfg)))
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
        Cmd::Domains { cmd } => {
            let cfg = load_config()?;
            match cmd {
                DomainsCmd::List { app } => {
                    let resp = client()?
                        .get(format!("{}/api/v1/projects/{app}/domains", base(&cfg)))
                        .bearer_auth(&cfg.token)
                        .send()?;
                    handle(resp)
                }
                DomainsCmd::Check { app, host } => {
                    let resp = client()?
                        .get(format!(
                            "{}/api/v1/projects/{app}/domains/{host}/check",
                            base(&cfg)
                        ))
                        .bearer_auth(&cfg.token)
                        .send()?;
                    handle(resp)
                }
                DomainsCmd::Rm { app, host } => {
                    let resp = client()?
                        .delete(format!(
                            "{}/api/v1/projects/{app}/domains/{host}",
                            base(&cfg)
                        ))
                        .bearer_auth(&cfg.token)
                        .send()?;
                    handle(resp)
                }
            }
        }
        Cmd::Whoami => {
            let cfg = load_config()?;
            let resp = client()?
                .get(format!("{}/api/v1/whoami", base(&cfg)))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
        Cmd::Teams => {
            let cfg = load_config()?;
            let resp = client()?
                .get(format!("{}/api/v1/teams", base(&cfg)))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
        Cmd::Team { cmd } => {
            let cfg = load_config()?;
            let (method, path, body) = match cmd {
                TeamCmd::Members { team } => ("GET", format!("/api/v1/teams/{team}/members"), None),
                TeamCmd::Invite { team, role } => (
                    "POST",
                    format!("/api/v1/teams/{team}/invites"),
                    Some(serde_json::json!({"role": role})),
                ),
                TeamCmd::Invites { team } => ("GET", format!("/api/v1/teams/{team}/invites"), None),
                TeamCmd::Revoke { team, token } => (
                    "DELETE",
                    format!("/api/v1/teams/{team}/invites/{token}"),
                    None,
                ),
                TeamCmd::Create { name } => (
                    "POST",
                    "/api/v1/teams".to_string(),
                    Some(serde_json::json!({"name": name})),
                ),
                TeamCmd::Role { team, user, role } => (
                    "PUT",
                    format!("/api/v1/teams/{team}/members/{user}"),
                    Some(serde_json::json!({"role": role})),
                ),
                TeamCmd::Remove { team, user } => (
                    "DELETE",
                    format!("/api/v1/teams/{team}/members/{user}"),
                    None,
                ),
            };
            let req = match method {
                "POST" => client()?
                    .post(format!("{}{path}", base(&cfg)))
                    .bearer_auth(&cfg.token)
                    .json(&body.unwrap_or_default()),
                "PUT" => client()?
                    .put(format!("{}{path}", base(&cfg)))
                    .bearer_auth(&cfg.token)
                    .json(&body.unwrap_or_default()),
                "DELETE" => client()?
                    .delete(format!("{}{path}", base(&cfg)))
                    .bearer_auth(&cfg.token),
                _ => client()?
                    .get(format!("{}{path}", base(&cfg)))
                    .bearer_auth(&cfg.token),
            };
            handle(req.send()?)
        }
        Cmd::Stats { app } => {
            let cfg = load_config()?;
            let resp = client()?
                .get(format!("{}/api/v1/projects/{app}/metrics", base(&cfg)))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
        Cmd::Proxy { cmd } => {
            let cfg = load_config()?;
            match cmd {
                ProxyCmd::Show { app } => {
                    let resp = client()?
                        .get(format!("{}/api/v1/projects/{app}/proxy", base(&cfg)))
                        .bearer_auth(&cfg.token)
                        .send()?;
                    handle(resp)
                }
                ProxyCmd::Set {
                    app,
                    headers,
                    basic_user,
                    basic_password,
                    redirect,
                    compression,
                    no_compression,
                    force_https,
                    no_force_https,
                    max_body_mb,
                } => {
                    let resp = client()?
                        .get(format!("{}/api/v1/projects/{app}/proxy", base(&cfg)))
                        .bearer_auth(&cfg.token)
                        .send()?;
                    if !resp.status().is_success() {
                        return handle(resp);
                    }
                    let mut value: serde_json::Value = resp.json()?;
                    let Some(obj) = value.as_object_mut() else {
                        bail!("server returned invalid proxy settings");
                    };
                    if !headers.is_empty() {
                        let mut pairs = Vec::new();
                        for raw in &headers {
                            let (k, v) = raw
                                .split_once(':')
                                .with_context(|| format!("expected Name: value, got {raw}"))?;
                            pairs.push(serde_json::json!([k.trim(), v.trim()]));
                        }
                        obj.insert("headers".into(), serde_json::Value::Array(pairs));
                    }
                    if let Some(user) = basic_user {
                        obj.insert("basic_auth_user".into(), serde_json::json!(user));
                    }
                    if let Some(password) = basic_password {
                        obj.insert("basic_auth_password".into(), serde_json::json!(password));
                    }
                    if let Some(redirect) = redirect {
                        if !matches!(redirect.as_str(), "off" | "www-to-apex" | "apex-to-www") {
                            bail!("redirect must be off, www-to-apex or apex-to-www");
                        }
                        obj.insert("redirect".into(), serde_json::json!(redirect));
                    }
                    if compression || no_compression {
                        obj.insert(
                            "compression".into(),
                            serde_json::json!(compression && !no_compression),
                        );
                    }
                    if force_https || no_force_https {
                        obj.insert(
                            "force_https".into(),
                            serde_json::json!(force_https && !no_force_https),
                        );
                    }
                    if let Some(mb) = max_body_mb {
                        obj.insert("max_body_mb".into(), serde_json::json!(mb));
                    }
                    let resp = client()?
                        .put(format!("{}/api/v1/projects/{app}/proxy", base(&cfg)))
                        .bearer_auth(&cfg.token)
                        .json(&value)
                        .send()?;
                    handle(resp)
                }
            }
        }
        Cmd::Certs { cmd } => {
            let cfg = load_config()?;
            match cmd {
                CertsCmd::List => {
                    let resp = client()?
                        .get(format!("{}/api/v1/certificates", base(&cfg)))
                        .bearer_auth(&cfg.token)
                        .send()?;
                    handle(resp)
                }
                CertsCmd::Renew { host } => {
                    let resp = client()?
                        .post(format!("{}/api/v1/certificates/{host}/renew", base(&cfg)))
                        .bearer_auth(&cfg.token)
                        .send()?;
                    handle(resp)
                }
            }
        }
        Cmd::AccessLog { app, limit } => {
            let cfg = load_config()?;
            let resp = client()?
                .get(format!(
                    "{}/api/v1/projects/{app}/access-log?limit={limit}",
                    base(&cfg)
                ))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
        Cmd::Webhook {
            app,
            secret,
            branch,
        } => {
            let cfg = load_config()?;
            if secret.is_none() && branch.is_none() {
                let resp = client()?
                    .get(format!("{}/api/v1/projects/{app}/webhook", base(&cfg)))
                    .bearer_auth(&cfg.token)
                    .send()?;
                return handle(resp);
            }
            let mut body = serde_json::Map::new();
            if let Some(secret) = secret {
                body.insert("secret".into(), serde_json::json!(secret));
            }
            if let Some(branch) = branch {
                body.insert("branch".into(), serde_json::json!(branch));
            }
            let resp = client()?
                .put(format!("{}/api/v1/projects/{app}/webhook", base(&cfg)))
                .bearer_auth(&cfg.token)
                .json(&serde_json::Value::Object(body))
                .send()?;
            handle(resp)
        }
        Cmd::Tokens { cmd } => {
            let cfg = load_config()?;
            match cmd {
                TokensCmd::List => {
                    let resp = client()?
                        .get(format!("{}/api/v1/tokens", base(&cfg)))
                        .bearer_auth(&cfg.token)
                        .send()?;
                    handle(resp)
                }
                TokensCmd::Create { name } => {
                    let resp = client()?
                        .post(format!("{}/api/v1/tokens", base(&cfg)))
                        .bearer_auth(&cfg.token)
                        .json(&serde_json::json!({"name": name}))
                        .send()?;
                    handle(resp)
                }
                TokensCmd::Revoke { hash } => {
                    let resp = client()?
                        .delete(format!("{}/api/v1/tokens/{hash}", base(&cfg)))
                        .bearer_auth(&cfg.token)
                        .send()?;
                    handle(resp)
                }
            }
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
                .get(format!("{}/api/v1/projects/{app}/deployments", base(&cfg)))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
        Cmd::Rollback { app, deploy } => {
            let cfg = load_config()?;
            let resp = client()?
                .post(format!(
                    "{}/api/v1/projects/{app}/rollback/{deploy}",
                    base(&cfg)
                ))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
        Cmd::Promote { app, deploy } => {
            let cfg = load_config()?;
            let resp = client()?
                .post(format!(
                    "{}/api/v1/projects/{app}/promote/{deploy}",
                    base(&cfg)
                ))
                .bearer_auth(&cfg.token)
                .send()?;
            handle(resp)
        }
        Cmd::Open { app } => {
            let cfg = load_config()?;
            let resp = client()?
                .get(format!("{}/api/v1/projects/{app}", base(&cfg)))
                .bearer_auth(&cfg.token)
                .send()?;
            if !resp.status().is_success() {
                return handle(resp);
            }
            let v: serde_json::Value = resp.json()?;
            let base_url = base(&cfg);
            let url = v["url"].as_str().map(str::to_string).unwrap_or_else(|| {
                match v["domain"].as_str() {
                    Some(d) => format!("{}://{d}", scheme_of(&base_url)),
                    None => {
                        let (scheme, host) = split_server(&base_url);
                        format!("{scheme}://{app}.{host}")
                    }
                }
            });
            if url.is_empty() {
                bail!("project has no routable domain; add one with `dok domain {app} <host>` or start dok with --base-domain");
            }
            if term::json() {
                println!("{url}");
            } else {
                println!("{}", term::cyan(&url));
            }
            Ok(())
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

pub async fn deploy_git(app: String, data: String, reference: Option<String>) -> Result<()> {
    let data = PathBuf::from(expand_home(&data));
    let db = crate::db::Db::open(&data.join("dok.db"))?;
    let base_domain = db
        .get_setting("base_domain")
        .ok()
        .flatten()
        .filter(|s| !s.is_empty());
    let public_url = db
        .get_setting("public_url")
        .ok()
        .flatten()
        .unwrap_or_default();
    let production_branch = db
        .get_setting(&format!("webhook_branch:{app}"))
        .ok()
        .flatten()
        .filter(|s| !s.is_empty());
    let traefik_dir = db
        .get_setting("traefik_dir")
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let pushed = reference
        .as_deref()
        .and_then(|r| r.strip_prefix("refs/heads/"))
        .filter(|b| !b.is_empty());
    if reference.is_some() && pushed.is_none() {
        // Tag and other non-branch pushes must never deploy production.
        return Ok(());
    }
    if let Some(branch) = pushed {
        if !crate::api::valid_branch(branch) || branch.starts_with('-') {
            anyhow::bail!("invalid branch name in git hook");
        }
    }
    let env = crate::api::git_deploy_env(production_branch.as_deref(), pushed);
    let branch = pushed.map(str::to_string).or(production_branch);
    let st = crate::state::AppState::new(db, data, base_domain, public_url)
        .with_traefik_dir(traefik_dir);
    let out = crate::api::deploy_from_repo_branch_env(&st, &app, branch.as_deref(), env).await?;
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
    let json_mode = term::json();
    let started = Instant::now();
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
    let server = base(&cfg);

    if !json_mode {
        println!(
            "{} {} {} {}",
            term::bold("dok push"),
            term::cyan(&app),
            term::dim("·"),
            term::dim(&format!("{} · {server}", args.env))
        );
        println!("  {}", term::dim(&dir.display().to_string()));
    }

    let tmp = std::env::temp_dir().join(format!("dok-{}.tar.gz", uuid::Uuid::new_v4().simple()));
    let pack_started = Instant::now();
    let spinner = term::Spinner::start(format!("Packing {}", dir.display()));
    let stats = pack(&dir, &tmp)?;
    spinner.finish();
    if !json_mode {
        term::success(&format!(
            "Packed {} files · {} · {:.1}s",
            stats.files,
            term::human_bytes(stats.bytes),
            pack_started.elapsed().as_secs_f64()
        ));
    }

    let mut url = format!("{}/api/v1/projects/{}/deploy", base(&cfg), app);
    let mut q = Vec::new();
    if let Some(p) = args.port {
        q.push(format!("port={p}"));
    }
    if let Some(d) = &args.domain {
        q.push(format!("domain={d}"));
    }
    if args.env != "production" {
        q.push(format!("env={}", args.env));
    }
    if !q.is_empty() {
        url.push('?');
        url.push_str(&q.join("&"));
    }

    let total = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
    if !json_mode {
        term::info(&format!(
            "Uploading {} to {}",
            term::human_bytes(total),
            server
        ));
    }

    let file = std::fs::File::open(&tmp)?;
    let (body, uploaded) = term::ProgressReader::new(file, total);
    let http = client()?;
    let token = cfg.token.clone();
    let (tx, rx) = mpsc::channel();
    let uploader = std::thread::Builder::new()
        .name("dok-upload".into())
        .spawn(move || {
            let result = http
                .post(url)
                .bearer_auth(token)
                .header("content-type", "application/gzip")
                .header(reqwest::header::CONTENT_LENGTH, total.to_string())
                .body(reqwest::blocking::Body::new(body))
                .send();
            let _ = tx.send(result);
        })
        .context("spawn upload thread")?;

    // The upload finishes before the server finishes building. Watch the byte
    // counter so the UI can switch from "uploading" to "deploying" instead of
    // sitting silent for the whole build.
    let upload_started = Instant::now();
    let mut upload_done = false;
    let mut deploy_spinner: Option<term::Spinner> = None;
    let response = loop {
        if !upload_done && uploaded.load(Ordering::Relaxed) {
            upload_done = true;
            if !json_mode {
                term::success(&format!(
                    "Uploaded {} in {:.1}s",
                    term::human_bytes(total),
                    upload_started.elapsed().as_secs_f64()
                ));
                deploy_spinner = Some(term::Spinner::start(format!(
                    "Deploying {app} · building on the server"
                )));
            }
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(response) => break response,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("upload stopped before the server replied")
            }
        }
    };
    if let Some(spinner) = deploy_spinner.take() {
        spinner.finish();
    }
    let _ = std::fs::remove_file(&tmp);
    let _ = uploader.join();

    let response = response.context("upload failed")?;
    if json_mode {
        return handle(response);
    }
    if !upload_done {
        term::success(&format!(
            "Uploaded {} in {:.1}s",
            term::human_bytes(total),
            upload_started.elapsed().as_secs_f64()
        ));
    }

    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        bail!("{}", server_error(status, &text));
    }
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            print!("{text}");
            return Ok(());
        }
    };
    let mut meta = Vec::new();
    let kind = field(&value, "kind");
    let env = field(&value, "env");
    if !kind.is_empty() {
        meta.push(kind.to_string());
    }
    if !env.is_empty() {
        meta.push(env.to_string());
    }
    let suffix = if meta.is_empty() {
        String::new()
    } else {
        format!(" · {}", meta.join(" · "))
    };
    term::success(&format!(
        "Deployed {} in {:.1}s{suffix}",
        term::bold(&app),
        started.elapsed().as_secs_f64()
    ));
    let deployed_url = field(&value, "url");
    if !deployed_url.is_empty() {
        println!("  {}", term::cyan(deployed_url));
    }
    Ok(())
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

struct PackStats {
    files: usize,
    bytes: u64,
}

fn pack(dir: &Path, out: &Path) -> Result<PackStats> {
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
        if entry.path_is_symlink()
            || std::fs::symlink_metadata(path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        {
            // Never follow a symlink: a hostile repo could point at ~/.ssh and
            // smuggle host files into the upload.
            eprintln!("skipping symlink {}", path.display());
            continue;
        }
        if path.components().any(|c| {
            matches!(
                c.as_os_str().to_str(),
                Some(".git" | "node_modules" | "target" | ".venv" | "__pycache__")
            )
        }) {
            continue;
        }
        let rel = path.strip_prefix(dir)?;
        // Never upload secrets or other dotfiles; .well-known must stay public.
        if rel.components().any(|c| {
            c.as_os_str()
                .to_str()
                .map(|name| name.starts_with('.') && name != ".well-known")
                .unwrap_or(false)
        }) {
            continue;
        }
        tb.append_path_with_name(path, rel)?;
        count += 1;
    }
    tb.finish()?;
    let mut inner = tb.into_inner()?;
    inner.flush()?;
    if count == 0 {
        bail!("nothing to upload (empty directory or everything ignored)");
    }
    let bytes = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    Ok(PackStats {
        files: count,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::pack;

    #[test]
    fn pack_skips_symlinks_outside_the_tree() {
        let dir = std::env::temp_dir().join(format!("dok-pack-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let secret = dir.join("secret.txt");
        std::fs::write(&secret, "TOP-SECRET").unwrap();
        std::fs::write(dir.join("index.html"), "hello").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, dir.join("leak.txt")).unwrap();
        let out = dir.join("out.tar.gz");
        pack(&dir, &out).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert!(!bytes.windows(10).any(|w| w == b"TOP-SECRET"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
