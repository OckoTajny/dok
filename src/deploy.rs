use crate::state::AppState;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

const MAX_ARCHIVE_ENTRIES: usize = 20_000;
const MAX_ARCHIVE_BYTES: u64 = 500 * 1024 * 1024;
const BUILD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const RUN_TIMEOUT: Duration = Duration::from_secs(3 * 60);
const READY_TIMEOUT: Duration = Duration::from_secs(45);

async fn output_with_timeout(cmd: &mut Command, limit: Duration) -> Result<std::process::Output> {
    cmd.kill_on_drop(true);
    match tokio::time::timeout(limit, cmd.output()).await {
        Ok(out) => out.context("run docker command"),
        Err(_) => bail!(
            "docker command timed out after {}s; the container or build was killed",
            limit.as_secs()
        ),
    }
}

/// A container can exit immediately after `docker run` returns. Wait until the
/// published loopback port accepts a TCP connection before retiring the old
/// production container.
/// Dockerfile paths in `dok.toml` must stay inside the uploaded build tree.
/// Absolute paths and `..` would otherwise let a deploy member make Docker read
/// any host file and echo it into the deploy log.
fn safe_dockerfile_path(dir: &Path, name: &str) -> bool {
    let candidate = Path::new(name);
    if candidate.as_os_str().is_empty()
        || candidate
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return false;
    }
    let (Ok(root), Ok(file)) = (
        std::fs::canonicalize(dir),
        std::fs::canonicalize(dir.join(candidate)),
    ) else {
        return false;
    };
    file.starts_with(&root) && file.is_file()
}

async fn wait_until_ready(port: u16, limit: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        if http_ready(port).await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// A TCP accept is not readiness: Docker's userland proxy accepts before the
/// app serves HTTP. Require a real non-5xx HTTP response (404 is fine).
async fn http_ready(port: u16) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Ok(mut stream) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await else {
        return false;
    };
    let req = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    if tokio::time::timeout(Duration::from_secs(2), stream.write_all(req))
        .await
        .ok()
        .and_then(|r| r.ok())
        .is_none()
    {
        return false;
    }
    let mut buf = [0u8; 512];
    let Ok(Ok(n)) = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut buf)).await
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&buf[..n]);
    text.starts_with("HTTP/1.1 2")
        || text.starts_with("HTTP/1.1 3")
        || text.starts_with("HTTP/1.1 4")
        || text.starts_with("HTTP/1.0 2")
        || text.starts_with("HTTP/1.0 3")
        || text.starts_with("HTTP/1.0 4")
}

fn archive_limits_ok(entries: usize, total: u64) -> bool {
    entries <= MAX_ARCHIVE_ENTRIES && total <= MAX_ARCHIVE_BYTES
}

#[derive(Debug, Deserialize, Default)]
pub struct DokToml {
    #[allow(dead_code)]
    pub name: Option<String>,
    pub port: Option<u16>,
    pub domain: Option<String>,
    pub dockerfile: Option<String>,
    pub memory_mb: Option<i64>,
    pub cpus: Option<f64>,
    pub framework: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct DeployOutcome {
    pub app: String,
    pub deploy_id: String,
    pub env: String,
    pub kind: String,
    pub url: String,
    pub host_port: Option<u16>,
}

pub fn read_dok_toml(dir: &Path) -> DokToml {
    let p = dir.join("dok.toml");
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

/// Extract a .tar.gz into `dest`, refusing paths that escape it.
pub fn extract_tarball(archive: &Path, dest: &Path) -> Result<()> {
    let f = std::fs::File::open(archive).context("open upload")?;
    let gz = flate2::read::GzDecoder::new(f);
    let mut ar = tar::Archive::new(gz);
    std::fs::create_dir_all(dest)?;
    let canonical_dest = dest.canonicalize()?;
    let mut entries = 0usize;
    let mut total: u64 = 0;
    for entry in ar.entries()? {
        let mut entry = entry?;
        entries += 1;
        let size = entry.header().size().unwrap_or(0);
        total = total.saturating_add(size);
        if !archive_limits_ok(entries, total) {
            bail!("archive exceeds {MAX_ARCHIVE_ENTRIES} entries or 500 MB uncompressed");
        }
        let etype = entry.header().entry_type();
        if etype.is_pax_global_extensions() || etype.is_pax_local_extensions() {
            continue;
        }
        if !etype.is_file() && !etype.is_dir() {
            bail!(
                "archive contains a symlink or special file ({}), refusing to extract",
                entry.path()?.display()
            );
        }
        let path = entry.path()?.into_owned();
        if path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            bail!("unsafe path in archive: {}", path.display());
        }
        entry.unpack_in(&canonical_dest)?;
    }
    Ok(())
}

/// Look at a Dockerfile and return the first EXPOSE port.
fn exposed_port(dir: &Path, dockerfile: &str) -> Option<u16> {
    let text = std::fs::read_to_string(dir.join(dockerfile)).ok()?;
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l
            .strip_prefix("EXPOSE ")
            .or_else(|| l.strip_prefix("expose "))
        {
            if let Some(tok) = rest.split_whitespace().next() {
                if let Ok(p) = tok.split('/').next()?.parse::<u16>() {
                    return Some(p);
                }
            }
        }
    }
    None
}

struct Detected {
    framework: String,
    dockerfile: Option<String>,
}

/// Static output directory produced by common frontend builders.
fn node_output_dir(pkg: &serde_json::Value) -> String {
    let dep = |name: &str| {
        ["dependencies", "devDependencies"]
            .iter()
            .any(|k| pkg.get(k).and_then(|d| d.get(name)).is_some())
    };
    if dep("nuxt") || dep("nuxt3") {
        ".output/public".into()
    } else if dep("react-scripts") || dep("@sveltejs/kit") {
        "build".into()
    } else {
        // Vite, Astro, Svelte, and most other builders.
        "dist".into()
    }
}

/// Auto-detect a build without a Dockerfile. An explicit `framework` wins;
/// `static` means static assets. Unknown projects fail instead of silently
/// being served as static.
fn detect_build(dir: &Path, hint: Option<&str>) -> Option<Detected> {
    let hint = hint.map(|h| h.trim().to_ascii_lowercase());
    if hint.as_deref() == Some("static") {
        return Some(Detected {
            framework: "static".into(),
            dockerfile: None,
        });
    }
    let node = matches!(
        hint.as_deref(),
        Some("node" | "next" | "nextjs" | "vite" | "react" | "remix" | "nuxt" | "svelte" | "astro")
    );
    let python = matches!(
        hint.as_deref(),
        Some("python" | "flask" | "django" | "fastapi")
    );
    let go = matches!(hint.as_deref(), Some("go" | "golang"));
    if node || dir.join("package.json").is_file() {
        let pkg = std::fs::read_to_string(dir.join("package.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        let scripts = pkg
            .as_ref()
            .and_then(|v| v.get("scripts"))
            .and_then(|v| v.as_object());
        let has_build = scripts.map(|s| s.contains_key("build")).unwrap_or(false);
        let has_start = scripts.map(|s| s.contains_key("start")).unwrap_or(false);
        let framework = hint.clone().unwrap_or_else(|| "node".into());
        let install =
            "RUN if [ -f package-lock.json ]; then npm ci || npm install; else npm install; fi\n";
        if has_start {
            let build = if has_build { "RUN npm run build\n" } else { "" };
            return Some(Detected {
                framework,
                dockerfile: Some(format!(
                    "FROM node:22-alpine\nWORKDIR /app\nCOPY package*.json ./\n{install}COPY . .\n{build}ENV NODE_ENV=production PORT=3000\nEXPOSE 3000\nCMD [\"npm\",\"start\"]\n"
                )),
            });
        }
        if has_build {
            let out = pkg
                .as_ref()
                .map(node_output_dir)
                .unwrap_or_else(|| "dist".into());
            return Some(Detected {
                framework,
                dockerfile: Some(format!(
                    "FROM node:22-alpine AS build\nWORKDIR /app\nCOPY package*.json ./\n{install}COPY . .\nRUN npm run build\nFROM node:22-alpine\nRUN npm install -g serve@14\nWORKDIR /app\nCOPY --from=build /app ./app\nWORKDIR /app/app\nEXPOSE 3000\nCMD [\"serve\",\"-s\",\"{out}\",\"-l\",\"3000\"]\n"
                )),
            });
        }
        if dir.join("index.html").is_file() {
            return Some(Detected {
                framework: "static".into(),
                dockerfile: None,
            });
        }
        // A package.json with neither start nor build would only fail later;
        // return None so the deploy error tells the user what to add.
        return None;
    }
    if python || dir.join("requirements.txt").is_file() || dir.join("pyproject.toml").is_file() {
        return Some(Detected {
            framework: hint.clone().unwrap_or_else(|| "python".into()),
            dockerfile: Some(
                "FROM python:3.12-slim\nWORKDIR /app\nCOPY . .\nRUN if [ -f requirements.txt ]; then pip install --no-cache-dir -r requirements.txt; fi\nRUN if [ -f pyproject.toml ]; then pip install --no-cache-dir .; fi\nENV PORT=8000\nEXPOSE 8000\nCMD [\"sh\",\"-c\",\"python main.py 2>/dev/null || python app.py\"]\n".into(),
            ),
        });
    }
    if go || dir.join("go.mod").is_file() {
        return Some(Detected {
            framework: hint.clone().unwrap_or_else(|| "go".into()),
            dockerfile: Some(
                "FROM golang:1.22-alpine AS build\nWORKDIR /app\nCOPY go.mod go.sum* ./\nRUN go mod download || true\nCOPY . .\nRUN CGO_ENABLED=0 go build -o /app/server .\nFROM alpine:3.20\nWORKDIR /app\nCOPY --from=build /app/server /app/server\nENV PORT=8080\nEXPOSE 8080\nCMD [\"/app/server\"]\n".into(),
            ),
        });
    }
    if dir.join("index.html").is_file() {
        return Some(Detected {
            framework: "static".into(),
            dockerfile: None,
        });
    }
    None
}

pub async fn docker_available() -> bool {
    Command::new("docker")
        .arg("version")
        .arg("--format")
        .arg("{{.Server.Version}}")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// One container per project and environment.
pub fn container_name(app: &str, env: &str) -> String {
    format!("dok-{app}-{env}")
}

pub async fn stop_all_previews(app: &str) {
    let out = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("label=dok.app={app}"),
            "--format",
            "{{.Names}}",
        ])
        .output()
        .await;
    if let Ok(out) = out {
        let prefix = format!("dok-{app}-preview-");
        for name in String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|n| n.starts_with(&prefix))
        {
            remove_container(name).await;
        }
    }
}

pub async fn stop_container(app: &str, env: &str) {
    let _ = Command::new("docker")
        .args(["rm", "-f", &container_name(app, env)])
        .output()
        .await;
}

fn normalize_env(env: &str) -> &str {
    if env == "preview" {
        "preview"
    } else {
        "production"
    }
}

/// Deploy an unpacked directory: build a Dockerfile if present, otherwise serve
/// it statically. Preview deploys never touch the production container or the
/// project row's production pointer.
pub async fn deploy_dir(
    st: &AppState,
    app: &str,
    dir: &Path,
    port_hint: Option<u16>,
    domain_hint: Option<String>,
    env: &str,
    author: Option<&str>,
    require_existing: bool,
) -> Result<DeployOutcome> {
    let env = normalize_env(env);
    // Bound concurrent builds and refuse to fill the disk completely.
    let _gate = st
        .deploy_gate
        .acquire()
        .await
        .map_err(|_| anyhow::anyhow!("deploy queue is closed"))?;
    if let Ok(free) = fs2::available_space(&st.data) {
        if free < 1024 * 1024 * 1024 {
            bail!("not enough free disk space to build (need at least 1 GB)");
        }
    }
    // One deploy per app at a time, across processes (the git hook runs its own).
    let _lock = st.lock_deploy(app).await?;
    let prev = st.db.get_app(app)?;
    if require_existing && prev.is_none() {
        bail!("project was deleted while the deploy was being prepared");
    }
    let deploy_id = uuid::Uuid::new_v4().simple().to_string();
    st.db.add_deploy(
        &deploy_id,
        app,
        "running",
        Some(&dir.to_string_lossy()),
        env,
        author,
    )?;
    let mut guard = DeployGuard {
        st,
        app,
        id: deploy_id.clone(),
        env,
        done: false,
    };
    let conf = read_dok_toml(dir);
    let chosen_domain = if env == "production" {
        // `dok.toml` may claim a domain on the first deploy. Later domain
        // changes go through the Domains tab/API so a deploy-only member
        // cannot silently move an existing project to another hostname.
        let toml_domain = if prev.as_ref().and_then(|p| p.domain.clone()).is_none() {
            conf.domain.clone()
        } else {
            None
        };
        match domain_hint.clone().or(toml_domain) {
            Some(d) => {
                let d = d.trim().trim_end_matches('.').to_ascii_lowercase();
                if !crate::api::valid_host(&d) {
                    bail!("invalid domain: {d}");
                }
                if st.reserved_host(&d) || st.is_base_subdomain(&d) {
                    bail!("domain {d} is reserved for the dashboard");
                }
                if let Ok(Some(other)) = st.db.domain_owner(&d) {
                    if other != app {
                        bail!("domain {d} belongs to another project");
                    }
                }
                if let Ok(Some(other)) = st.db.app_domain_owner(&d) {
                    if other != app {
                        bail!("domain {d} belongs to another project");
                    }
                }
                Some(d)
            }
            None => None,
        }
    } else {
        None
    };
    let framework_hint = conf
        .framework
        .clone()
        .or_else(|| prev.as_ref().and_then(|p| p.framework.clone()));
    let mut dockerfile = conf
        .dockerfile
        .clone()
        .unwrap_or_else(|| "Dockerfile".into());
    if conf.dockerfile.is_some() && !safe_dockerfile_path(dir, &dockerfile) {
        bail!("dockerfile must be a relative path inside the project");
    }
    let mut has_dockerfile = dir.join(&dockerfile).is_file();
    let mut framework = framework_hint.clone().unwrap_or_default();
    if conf.dockerfile.is_some() && !has_dockerfile {
        bail!("configured Dockerfile {dockerfile} does not exist");
    }
    if !has_dockerfile {
        let Some(detected) = detect_build(dir, framework_hint.as_deref()) else {
            bail!(
                "cannot detect a build: add a Dockerfile, package.json, requirements.txt/pyproject.toml, go.mod, or index.html"
            );
        };
        framework = detected.framework;
        if let Some(content) = detected.dockerfile {
            dockerfile = "Dockerfile.dok".into();
            std::fs::write(dir.join(&dockerfile), content)?;
            has_dockerfile = true;
        }
    }
    let memory_mb = conf.memory_mb.unwrap_or(512).max(32);
    let cpus = conf.cpus.unwrap_or(1.0);
    let mut log = String::new();
    if !framework.is_empty() {
        log.push_str(&format!("detected framework: {framework}\n"));
    }

    let (kind, host_port, container_port, routing_container) = if has_dockerfile {
        if !docker_available().await {
            bail!("docker is not available on this host");
        }
        let container_port = port_hint
            .or(conf.port)
            .or_else(|| exposed_port(dir, &dockerfile))
            .unwrap_or(3000);

        let tag = format!("dok/{app}:{deploy_id}");
        log.push_str(&format!("$ docker build -f {dockerfile} -t {tag} .\n"));
        let mut build = Command::new("docker");
        build
            .args([
                "build",
                "-f",
                &dockerfile,
                "-t",
                &tag,
                "--label",
                &format!("dok.app={app}"),
                ".",
            ])
            .current_dir(dir);
        let out = match output_with_timeout(&mut build, BUILD_TIMEOUT).await {
            Ok(o) => o,
            Err(e) => {
                log.push_str(&format!("\n{e}\n"));
                st.db.finish_deploy(&deploy_id, "failed", &log)?;
                guard.done = true;
                if env == "production" {
                    st.db.set_status(app, "failed")?;
                }
                return Err(e);
            }
        };
        log.push_str(&String::from_utf8_lossy(&out.stdout));
        log.push_str(&String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            st.db.finish_deploy(&deploy_id, "failed", &log)?;
            guard.done = true;
            if env == "production" {
                st.db.set_status(app, "failed")?;
            }
            bail!("docker build failed (see `dok logs {app}`)");
        }

        // Start the replacement first; the old container keeps serving until the
        // new one is running.
        let temp_name = format!("dok-{app}-{env}-{}", &deploy_id[..8]);
        let restart_policy = if env == "preview" {
            "no"
        } else {
            "unless-stopped"
        };
        let mut cmd = Command::new("docker");
        cmd.args([
            "run",
            "-d",
            "--name",
            &temp_name,
            "--restart",
            restart_policy,
            "-l",
            &format!("dok.app={app}"),
            "-l",
            &format!("dok.env={env}"),
            "-p",
            &format!("127.0.0.1::{container_port}"),
            "--memory",
            &format!("{memory_mb}m"),
            "--cpus",
            &format!("{cpus}"),
            "--pids-limit",
            "512",
            "--security-opt",
            "no-new-privileges",
        ]);
        // Pass secrets through a 0600 file, never through the process argv.
        let env_file = st.data.join("tmp").join(format!("env-{deploy_id}.env"));
        if let Some(parent) = env_file.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&env_file)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&env_file, std::fs::Permissions::from_mode(0o600))?;
            }
            for (k, v) in st.db.env_resolved(app, env)? {
                if v.contains('\n') || v.contains('\r') {
                    bail!(
                        "environment variable {k} contains a newline; Docker env files cannot represent it (store it base64-encoded)"
                    );
                }
                writeln!(f, "{k}={v}")?;
            }
        }
        cmd.args(["--env-file", &env_file.to_string_lossy()]);
        cmd.arg(&tag);
        let output = output_with_timeout(&mut cmd, RUN_TIMEOUT).await;
        let _ = tokio::fs::remove_file(&env_file).await;
        let out = match output {
            Ok(o) => o,
            Err(e) => {
                log.push_str(&format!("\n{e}\n"));
                remove_container(&temp_name).await;
                st.db.finish_deploy(&deploy_id, "failed", &log)?;
                guard.done = true;
                if env == "production" {
                    st.db.set_status(app, "failed")?;
                }
                return Err(e);
            }
        };
        log.push_str(&String::from_utf8_lossy(&out.stdout));
        log.push_str(&String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            remove_container(&temp_name).await;
            st.db.finish_deploy(&deploy_id, "failed", &log)?;
            guard.done = true;
            if env == "production" {
                st.db.set_status(app, "failed")?;
            }
            bail!("docker run failed (see `dok logs {app}`)");
        }
        let host_port = match mapped_port(&temp_name, container_port).await {
            Some(p) => p,
            None => {
                remove_container(&temp_name).await;
                bail!("docker did not report a host port");
            }
        };
        if !wait_until_ready(host_port, READY_TIMEOUT).await {
            remove_container(&temp_name).await;
            st.db.finish_deploy(
                &deploy_id,
                "failed",
                &format!("{log}\nnew container did not become ready on 127.0.0.1:{host_port}"),
            )?;
            guard.done = true;
            if env == "production" {
                st.db.set_status(app, "failed")?;
            }
            bail!("new container did not become ready; the previous production deploy was kept");
        }
        (
            "docker",
            Some(host_port),
            Some(container_port),
            Some(temp_name),
        )
    } else {
        ("static", None, None, None)
    };

    let image = if kind == "docker" {
        Some(format!("dok/{app}:{deploy_id}"))
    } else {
        None
    };

    if env == "production" {
        let row = crate::db::App {
            name: app.to_string(),
            team_id: prev.as_ref().and_then(|p| p.team_id.clone()),
            kind: kind.to_string(),
            image: image.clone(),
            memory_mb: Some(memory_mb),
            cpus: Some(cpus),
            host_port,
            container_port,
            domain: chosen_domain.or_else(|| prev.as_ref().and_then(|p| p.domain.clone())),
            status: "running".into(),
            dir: Some(dir.to_string_lossy().to_string()),
            // A plain push must not forget how the project is deployed.
            repo: prev.as_ref().and_then(|p| p.repo.clone()),
            production_deploy: Some(deploy_id.clone()),
            framework: if framework.is_empty() {
                None
            } else {
                Some(framework.clone())
            },
            updated_at: crate::db::now(),
        };
        st.db.upsert_app(&row)?;
    } else if prev.is_none() {
        // A preview can bootstrap a project row, but it must not claim to be live.
        let row = crate::db::App {
            name: app.to_string(),
            team_id: None,
            kind: kind.to_string(),
            image: None,
            memory_mb: Some(memory_mb),
            cpus: Some(cpus),
            host_port: None,
            container_port: None,
            domain: None,
            status: "new".into(),
            dir: None,
            repo: None,
            production_deploy: None,
            framework: if framework.is_empty() {
                None
            } else {
                Some(framework.clone())
            },
            updated_at: crate::db::now(),
        };
        st.db.upsert_app(&row)?;
    }
    st.sync_traefik();

    // Point traffic at the new container, then retire the old one. Only the
    // selected environment is touched.
    if kind == "docker" && env == "production" {
        let temp_name = routing_container
            .clone()
            .ok_or_else(|| anyhow::anyhow!("missing production container"))?;
        stop_container(app, "production").await;
        let _ = Command::new("docker")
            .args(["rename", &temp_name, &container_name(app, "production")])
            .output()
            .await;
    } else if kind == "static" && env == "production" {
        // Switched to static serving: no container should keep running.
        stop_container(app, "production").await;
    }
    let final_container = if kind == "docker" {
        if env == "production" {
            Some(container_name(app, "production"))
        } else {
            routing_container
        }
    } else {
        None
    };
    st.db
        .set_deploy_routing(&deploy_id, final_container.as_deref(), host_port)?;

    // Keep earlier images so promote/rollback can start them directly; build
    // trees and deploy rows are pruned to the last 10 per environment.
    if env == "production" {
        st.db.set_production(app, &deploy_id)?;
    }
    let removed = st.db.prune_deploys_env(app, env, 10)?;
    cleanup_removed(&removed).await;
    cleanup_builds(st, app).await;
    if env == "preview" {
        prune_running_previews(st, app, 3).await;
    }

    let url = if env == "production" {
        st.db
            .get_app(app)?
            .and_then(|a| a.domain)
            .map(|d| format!("{}://{d}", st.scheme))
            .or_else(|| {
                st.base_domain
                    .as_ref()
                    .map(|b| format!("{}://{app}.{b}", st.scheme))
            })
            .or_else(|| {
                st.db
                    .domains_for(app)
                    .ok()
                    .and_then(|d| d.into_iter().next())
                    .map(|d| format!("{}://{d}", st.scheme))
            })
            .unwrap_or_default()
    } else if let Some(base) = &st.base_domain {
        format!("{}://{}-{}.{}", st.scheme, app, &deploy_id[..8], base)
    } else {
        match host_port {
            Some(p) => format!("http://127.0.0.1:{p}"),
            None => String::new(),
        }
    };

    st.db
        .finish_deploy_meta(&deploy_id, "ok", &log, image.as_deref(), Some(&url))?;
    if env == "production" {
        st.db.set_status(app, "ok")?;
    }
    guard.done = true;

    Ok(DeployOutcome {
        app: app.to_string(),
        deploy_id,
        env: env.to_string(),
        kind: kind.to_string(),
        url,
        host_port,
    })
}

struct DeployGuard<'a> {
    st: &'a AppState,
    app: &'a str,
    id: String,
    env: &'a str,
    done: bool,
}

impl Drop for DeployGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ =
                self.st
                    .db
                    .finish_deploy(&self.id, "failed", "deploy aborted before completion");
            if self.env == "production" {
                let _ = self.st.db.set_status(self.app, "failed");
            }
        }
    }
}

/// Keep build trees referenced by retained deploy rows.
async fn cleanup_builds(st: &AppState, app: &str) {
    use std::collections::HashSet;
    let keep: HashSet<String> = st
        .db
        .deploys(app, 100_000)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|d| d.dir)
        .collect();
    let root = st.builds_dir().join(app);
    let Ok(mut rd) = tokio::fs::read_dir(&root).await else {
        return;
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        let p = entry.path();
        if !keep.contains(&p.to_string_lossy().to_string()) {
            let _ = tokio::fs::remove_dir_all(&p).await;
        }
    }
}

/// Keep at most `keep` preview containers running per project. Older preview
/// rows stay for promote/debug, but their containers are stopped so a project
/// cannot leave ten always-on previews consuming memory and ports.
async fn prune_running_previews(st: &AppState, app: &str, keep: usize) {
    for d in st
        .db
        .deploys_env(app, Some("preview"), 100)
        .unwrap_or_default()
        .into_iter()
        .skip(keep)
    {
        if let Some(name) = d.container.as_deref().filter(|n| n.contains("-preview-")) {
            let _ = Command::new("docker").args(["stop", name]).output().await;
        }
    }
}

async fn cleanup_removed(removed: &[crate::db::Deploy]) {
    for d in removed {
        if let Some(name) = &d.container {
            if name.contains("-preview-") {
                let _ = Command::new("docker")
                    .args(["rm", "-f", name])
                    .output()
                    .await;
            }
        }
        if let Some(image) = d.image.as_deref() {
            // Images are unique per deploy id; once the row is pruned beyond
            // the rollback window the image is no longer needed.
            let _ = Command::new("docker").args(["rmi", image]).output().await;
        }
    }
}

/// Promote any earlier build onto production. When the recorded image still
/// exists we start it directly with the current resolved env; otherwise we
/// fall back to rebuilding from the retained build directory.
pub async fn promote(
    st: &AppState,
    app: &str,
    deploy_id: &str,
    author: Option<&str>,
) -> Result<DeployOutcome> {
    let deploy = st
        .db
        .deploy(app, deploy_id)?
        .ok_or_else(|| anyhow::anyhow!("no such deploy {deploy_id}"))?;
    if deploy.status != "ok" {
        bail!("deploy {deploy_id} did not finish successfully");
    }
    let dir = deploy
        .dir
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no build for deploy {deploy_id}"))?;
    let path = std::path::PathBuf::from(&dir);
    if !path.is_dir() {
        bail!("build for that deploy is gone (only the last 10 are kept)");
    }
    let prev = st
        .db
        .get_app(app)?
        .ok_or_else(|| anyhow::anyhow!("no such project"))?;
    let conf = read_dok_toml(&path);
    let dockerfile = conf
        .dockerfile
        .clone()
        .unwrap_or_else(|| "Dockerfile".into());
    let container_port = prev
        .container_port
        .or(conf.port)
        .or_else(|| exposed_port(&path, &dockerfile))
        .unwrap_or(3000);
    let memory_mb = conf.memory_mb.or(prev.memory_mb).unwrap_or(512).max(32);
    let cpus = conf.cpus.or(prev.cpus).unwrap_or(1.0);

    if let Some(image) = deploy.image.clone() {
        if docker_available().await && image_exists(&image).await {
            let _lock = st.lock_deploy(app).await?;
            let new_id = uuid::Uuid::new_v4().simple().to_string();
            st.db
                .add_deploy(&new_id, app, "running", Some(&dir), "production", author)?;
            let mut guard = DeployGuard {
                st,
                app,
                id: new_id.clone(),
                env: "production",
                done: false,
            };
            let temp = format!("dok-{app}-production-{}", &new_id[..8]);
            let host_port = match run_image(
                st,
                app,
                "production",
                &image,
                container_port,
                memory_mb,
                cpus,
                &temp,
                &new_id,
            )
            .await
            {
                Ok(p) => p,
                Err(e) => {
                    st.db
                        .finish_deploy(&new_id, "failed", &format!("promote failed: {e}"))?;
                    guard.done = true;
                    return Err(e);
                }
            };
            if !wait_until_ready(host_port, READY_TIMEOUT).await {
                remove_container(&temp).await;
                st.db.finish_deploy(
                    &new_id,
                    "failed",
                    "promoted container did not become ready; previous deploy kept",
                )?;
                guard.done = true;
                bail!("promoted container did not become ready; previous deploy was kept");
            }
            if let Some(sha) = deploy.commit_sha.as_deref() {
                let _ = st.db.set_deploy_commit(
                    &new_id,
                    sha,
                    deploy.commit_message.as_deref().unwrap_or(""),
                    deploy.commit_author.as_deref().unwrap_or(""),
                    deploy.branch.as_deref().unwrap_or(""),
                );
            }
            stop_container(app, "production").await;
            let _ = Command::new("docker")
                .args(["rename", &temp, &container_name(app, "production")])
                .output()
                .await;
            let row = crate::db::App {
                name: app.to_string(),
                team_id: prev.team_id.clone(),
                kind: "docker".into(),
                image: Some(image.clone()),
                memory_mb: Some(memory_mb),
                cpus: Some(cpus),
                host_port: Some(host_port),
                container_port: Some(container_port),
                domain: prev.domain.clone(),
                status: "running".into(),
                dir: Some(dir.clone()),
                repo: prev.repo.clone(),
                production_deploy: Some(new_id.clone()),
                framework: prev.framework.clone(),
                updated_at: crate::db::now(),
            };
            st.db.upsert_app(&row)?;
            st.db.set_production(app, &new_id)?;
            st.db.set_deploy_routing(
                &new_id,
                Some(&container_name(app, "production")),
                Some(host_port),
            )?;
            st.sync_traefik();
            let removed = st.db.prune_deploys_env(app, "production", 10)?;
            cleanup_removed(&removed).await;
            cleanup_builds(st, app).await;
            let url = row
                .domain
                .clone()
                .map(|d| format!("{}://{d}", st.scheme))
                .or_else(|| {
                    st.base_domain
                        .as_ref()
                        .map(|b| format!("{}://{app}.{b}", st.scheme))
                })
                .or_else(|| {
                    st.db
                        .domains_for(app)
                        .ok()
                        .and_then(|d| d.into_iter().next())
                        .map(|d| format!("{}://{d}", st.scheme))
                })
                .unwrap_or_default();
            st.db.finish_deploy_meta(
                &new_id,
                "ok",
                &format!("promoted image {image}\n"),
                Some(&image),
                Some(&url),
            )?;
            st.db.set_status(app, "ok")?;
            guard.done = true;
            return Ok(DeployOutcome {
                app: app.to_string(),
                deploy_id: new_id,
                env: "production".into(),
                kind: "docker".into(),
                url,
                host_port: Some(host_port),
            });
        }
    }

    deploy_dir(st, app, &path, None, None, "production", author, true).await
}

async fn image_exists(image: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", image])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
async fn run_image(
    st: &AppState,
    app: &str,
    env: &str,
    image: &str,
    container_port: u16,
    memory_mb: i64,
    cpus: f64,
    name: &str,
    deploy_id: &str,
) -> Result<u16> {
    let env_file = st.data.join("tmp").join(format!("env-{deploy_id}.env"));
    if let Some(parent) = env_file.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&env_file)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&env_file, std::fs::Permissions::from_mode(0o600))?;
        }
        for (k, v) in st.db.env_resolved(app, env)? {
            if v.contains('\n') || v.contains('\r') {
                bail!(
                    "environment variable {k} contains a newline; Docker env files cannot represent it (store it base64-encoded)"
                );
            }
            writeln!(f, "{k}={v}")?;
        }
    }
    let mut cmd = Command::new("docker");
    cmd.args([
        "run",
        "-d",
        "--name",
        name,
        "--restart",
        "unless-stopped",
        "-l",
        &format!("dok.app={app}"),
        "-l",
        &format!("dok.env={env}"),
        "-p",
        &format!("127.0.0.1::{container_port}"),
        "--memory",
        &format!("{memory_mb}m"),
        "--cpus",
        &format!("{cpus}"),
        "--env-file",
        &env_file.to_string_lossy(),
        image,
    ]);
    let output = cmd.output().await;
    let _ = tokio::fs::remove_file(&env_file).await;
    let out = output.context("run docker run")?;
    if !out.status.success() {
        remove_container(name).await;
        bail!(
            "docker run failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    mapped_port(name, container_port)
        .await
        .ok_or_else(|| anyhow::anyhow!("docker did not report a host port"))
}

/// Backwards-compatible name.
#[allow(dead_code)]
pub async fn rollback(st: &AppState, app: &str, deploy_id: &str) -> Result<DeployOutcome> {
    promote(st, app, deploy_id, None).await
}

async fn remove_container(name: &str) {
    let _ = Command::new("docker")
        .args(["rm", "-f", name])
        .output()
        .await;
}

async fn mapped_port(name: &str, container_port: u16) -> Option<u16> {
    let out = Command::new("docker")
        .args(["port", name, &format!("{container_port}/tcp")])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find_map(|l| l.rsplit(':').next()?.trim().parse::<u16>().ok())
}

/// Copy a directory into `<data>/builds/<app>/<deploy_id>` and return it.
pub fn new_build_dir(st: &AppState, app: &str) -> Result<PathBuf> {
    let dir = st
        .builds_dir()
        .join(app)
        .join(uuid::Uuid::new_v4().simple().to_string());
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Real CPU/memory for a container. Returns nulls (plus a note) when Docker
/// stats are unavailable instead of inventing numbers.
pub async fn container_stats(app: &str, env: &str, container: Option<&str>) -> serde_json::Value {
    let name = container
        .map(str::to_string)
        .unwrap_or_else(|| container_name(app, env));
    let out = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new("docker")
            .args([
                "stats",
                "--no-stream",
                "--format",
                "{{.CPUPerc}}|{{.MemUsage}}",
                &name,
            ])
            .output(),
    )
    .await;
    match out {
        Ok(Ok(o)) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let line = text.lines().next().unwrap_or("");
            let (cpu, mem) = line.split_once('|').unwrap_or(("", ""));
            serde_json::json!({
                "cpu": if cpu.trim().is_empty() { serde_json::Value::Null } else { serde_json::json!(cpu.trim()) },
                "memory": if mem.trim().is_empty() { serde_json::Value::Null } else { serde_json::json!(mem.trim()) },
                "container": name,
                "note": serde_json::Value::Null,
            })
        }
        _ => serde_json::json!({
            "cpu": serde_json::Value::Null,
            "memory": serde_json::Value::Null,
            "container": name,
            "note": "docker stats unavailable",
        }),
    }
}

pub async fn container_logs(app: &str, env: &str, container: Option<&str>, tail: u32) -> String {
    let name = container
        .map(str::to_string)
        .unwrap_or_else(|| container_name(app, env));
    let out = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("docker")
            .args(["logs", "--tail", &tail.to_string(), &name])
            .output(),
    )
    .await;
    match out {
        Ok(Ok(o)) => {
            let mut s = String::from_utf8_lossy(&o.stdout).to_string();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            s
        }
        Ok(Err(_)) => String::new(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{archive_limits_ok, container_name, detect_build, exposed_port, normalize_env};

    #[tokio::test]
    async fn docker_commands_have_a_timeout() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", "sleep 5"]);
        let started = std::time::Instant::now();
        let err = super::output_with_timeout(&mut cmd, std::time::Duration::from_millis(100))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("timed out"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn reads_expose() {
        let dir = std::env::temp_dir().join(format!("dok-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Dockerfile"),
            "FROM node:22\nEXPOSE 8080/tcp\nCMD [\"node\",\"index.js\"]\n",
        )
        .unwrap();
        assert_eq!(exposed_port(&dir, "Dockerfile"), Some(8080));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_symlink_entries() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("dok-tar-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("evil.tar.gz");
        {
            let f = std::fs::File::create(&archive).unwrap();
            let enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            let mut b = tar::Builder::new(enc);
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            h.set_mode(0o777);
            h.set_cksum();
            b.append_link(&mut h, "evil", "/etc/passwd").unwrap();
            b.into_inner().unwrap().flush().unwrap();
        }
        let out = dir.join("out");
        assert!(super::extract_tarball(&archive, &out).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn extracts_regular_files() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("dok-tar-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("ok.tar.gz");
        {
            let f = std::fs::File::create(&archive).unwrap();
            let enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            let mut b = tar::Builder::new(enc);
            let data = b"hello";
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, "index.html", &data[..]).unwrap();
            b.into_inner().unwrap().flush().unwrap();
        }
        let out = dir.join("out");
        super::extract_tarball(&archive, &out).unwrap();
        assert_eq!(std::fs::read(out.join("index.html")).unwrap(), b"hello");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_expose() {
        let dir = std::env::temp_dir().join(format!("dok-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Dockerfile"), "FROM scratch\n").unwrap();
        assert_eq!(exposed_port(&dir, "Dockerfile"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detects_builds() {
        let dir = std::env::temp_dir().join(format!("dok-detect-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(detect_build(&dir, None).is_none());
        std::fs::write(dir.join("index.html"), "x").unwrap();
        assert_eq!(detect_build(&dir, None).unwrap().framework, "static");
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"start":"node index.js"}}"#,
        )
        .unwrap();
        let node = detect_build(&dir, None).unwrap();
        assert_eq!(node.framework, "node");
        assert!(node.dockerfile.unwrap().contains("node:22"));
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"build":"vite build"},"devDependencies":{"vite":"1"}}"#,
        )
        .unwrap();
        let vite = detect_build(&dir, None).unwrap();
        let dockerfile = vite.dockerfile.unwrap();
        assert!(dockerfile.contains("npm run build"), "{dockerfile}");
        assert!(dockerfile.contains("serve"), "{dockerfile}");
        assert!(dockerfile.contains("dist"), "{dockerfile}");
        std::fs::remove_file(dir.join("package.json")).unwrap();
        std::fs::write(dir.join("go.mod"), "module x").unwrap();
        assert_eq!(detect_build(&dir, None).unwrap().framework, "go");
        assert_eq!(
            detect_build(&dir, Some("static")).unwrap().framework,
            "static"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn archive_budget() {
        assert!(archive_limits_ok(20_000, 500 * 1024 * 1024));
        assert!(!archive_limits_ok(20_001, 0));
        assert!(!archive_limits_ok(0, 500 * 1024 * 1024 + 1));
    }

    #[test]
    fn per_env_container_names() {
        assert_eq!(container_name("app", "production"), "dok-app-production");
        assert_eq!(container_name("app", "preview"), "dok-app-preview");
        assert_eq!(normalize_env("preview"), "preview");
        assert_eq!(normalize_env("staging"), "production");
    }

    #[tokio::test]
    async fn redeploy_keeps_domain_and_repo() {
        let base = std::env::temp_dir().join(format!("dok-keep-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let db = crate::db::Db::open(&base.join("dok.db")).unwrap();
        db.upsert_app(&crate::db::App {
            name: "app".into(),
            team_id: None,
            kind: "static".into(),
            image: None,
            memory_mb: None,
            cpus: None,
            host_port: None,
            container_port: None,
            domain: Some("app.example.com".into()),
            status: "ok".into(),
            dir: None,
            repo: Some("https://github.com/example/app.git".into()),
            production_deploy: None,
            framework: None,
            updated_at: crate::db::now(),
        })
        .unwrap();
        let src = base.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("index.html"), "hi").unwrap();
        let st = crate::state::AppState::new(
            db.clone(),
            base.join("data"),
            None,
            "http://localhost".into(),
        );
        super::deploy_dir(&st, "app", &src, None, None, "production", None, false)
            .await
            .unwrap();
        let app = db.get_app("app").unwrap().unwrap();
        assert_eq!(app.domain.as_deref(), Some("app.example.com"));
        assert_eq!(
            app.repo.as_deref(),
            Some("https://github.com/example/app.git")
        );
        let _ = std::fs::remove_dir_all(&base);
    }
    #[test]
    fn dockerfile_path_cannot_escape_the_build_tree() {
        let dir = std::env::temp_dir().join(format!("dok-df-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("Dockerfile"), "FROM scratch\n").unwrap();
        std::fs::write(dir.join("sub/Dockerfile"), "FROM scratch\n").unwrap();
        assert!(super::safe_dockerfile_path(&dir, "Dockerfile"));
        assert!(super::safe_dockerfile_path(&dir, "sub/Dockerfile"));
        assert!(!super::safe_dockerfile_path(&dir, "/etc/hostname"));
        assert!(!super::safe_dockerfile_path(&dir, "../etc/hostname"));
        assert!(!super::safe_dockerfile_path(&dir, "sub/../../etc/hostname"));
        assert!(!super::safe_dockerfile_path(&dir, ""));
        std::fs::remove_dir_all(&dir).ok();
    }
}
