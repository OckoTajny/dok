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
    /// Use the Let's Encrypt production directory (default: staging).
    #[arg(long, env = "DOK_ACME_PROD")]
    pub acme_prod: bool,
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
            "no config at {}; run `dok login <url> <token>`",
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
            println!("== build ==\n{}", v["build"].as_str().unwrap_or(""));
            println!("== runtime ==\n{}", v["runtime"].as_str().unwrap_or(""));
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
                        println!("{k}={}", val.as_str().unwrap_or(""));
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
            println!("{url}");
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
    let pushed = reference
        .as_deref()
        .and_then(|r| r.strip_prefix("refs/heads/"))
        .filter(|b| !b.is_empty());
    let env = crate::api::git_deploy_env(production_branch.as_deref(), pushed);
    let branch = pushed.map(str::to_string).or(production_branch);
    let st = crate::state::AppState::new(db, data, base_domain, public_url);
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
    Ok(())
}
