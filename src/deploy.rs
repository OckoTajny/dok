use crate::state::AppState;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, Deserialize, Default)]
pub struct DokToml {
    #[allow(dead_code)]
    pub name: Option<String>,
    pub port: Option<u16>,
    pub domain: Option<String>,
    pub dockerfile: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct DeployOutcome {
    pub app: String,
    pub deploy_id: String,
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
    for entry in ar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        }) {
            bail!("unsafe path in archive: {}", path.display());
        }
        let out = canonical_dest.join(&path);
        if !out.starts_with(&canonical_dest) {
            bail!("unsafe path in archive: {}", path.display());
        }
        entry.unpack(&out)?;
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

fn pick_port() -> Result<u16> {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(l.local_addr()?.port())
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

pub fn container_name(app: &str) -> String {
    format!("dok-{app}")
}

pub async fn stop_container(app: &str) {
    let _ = Command::new("docker")
        .args(["rm", "-f", &container_name(app)])
        .output()
        .await;
}

/// Deploy an unpacked directory: build a Dockerfile if present, otherwise serve it statically.
pub async fn deploy_dir(
    st: &AppState,
    app: &str,
    dir: &Path,
    port_hint: Option<u16>,
    domain_hint: Option<String>,
) -> Result<DeployOutcome> {
    let deploy_id = uuid::Uuid::new_v4().simple().to_string();
    st.db.add_deploy(&deploy_id, app, "running")?;

    let prev = st.db.get_app(app)?;
    let conf = read_dok_toml(dir);
    let chosen_domain = domain_hint.clone().or_else(|| conf.domain.clone());
    let dockerfile = conf.dockerfile.clone().unwrap_or_else(|| "Dockerfile".into());
    let has_dockerfile = dir.join(&dockerfile).is_file();
    let mut log = String::new();

    let (kind, host_port, container_port) = if has_dockerfile {
        if !docker_available().await {
            let msg = "docker is not available on this host".to_string();
            st.db.finish_deploy(&deploy_id, "failed", &msg)?;
            bail!(msg);
        }
        let container_port = port_hint
            .or(conf.port)
            .or_else(|| exposed_port(dir, &dockerfile))
            .unwrap_or(3000);

        let tag = format!("dok/{app}:{deploy_id}");
        log.push_str(&format!("$ docker build -t {tag} .\n"));
        let out = Command::new("docker")
            .args(["build", "-t", &tag, "."])
            .current_dir(dir)
            .output()
            .await
            .context("run docker build")?;
        log.push_str(&String::from_utf8_lossy(&out.stdout));
        log.push_str(&String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            st.db.finish_deploy(&deploy_id, "failed", &log)?;
            st.db.set_status(app, "failed")?;
            bail!("docker build failed (see `dok logs {app}`)");
        }

        // Start the replacement first; the old container keeps serving until the
        // new one is running and the DB points at it.
        let host_port = pick_port()?;
        let temp_name = format!("dok-{app}-{}", &deploy_id[..8]);
        let mut cmd = Command::new("docker");
        cmd.args([
            "run",
            "-d",
            "--name",
            &temp_name,
            "--restart",
            "unless-stopped",
            "-l",
            &format!("dok.app={app}"),
            "-p",
            &format!("127.0.0.1:{host_port}:{container_port}"),
        ]);
        for (k, v) in st.db.env(app)? {
            cmd.args(["-e", &format!("{k}={v}")]);
        }
        cmd.arg(&tag);
        let out = cmd.output().await.context("run docker run")?;
        log.push_str(&String::from_utf8_lossy(&out.stdout));
        log.push_str(&String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            let _ = Command::new("docker")
                .args(["rm", "-f", &temp_name])
                .output()
                .await;
            st.db.finish_deploy(&deploy_id, "failed", &log)?;
            st.db.set_status(app, "failed")?;
            bail!("docker run failed (see `dok logs {app}`)");
        }
        ("docker", Some(host_port), Some(container_port))
    } else {
        ("static", None, None)
    };

    let url = chosen_domain
        .clone()
        .map(|d| format!("https://{d}"))
        .unwrap_or_else(|| st.public_url.clone());

    let row = crate::db::App {
        name: app.to_string(),
        kind: kind.to_string(),
        host_port,
        container_port,
        domain: chosen_domain,
        status: "running".into(),
        dir: Some(dir.to_string_lossy().to_string()),
        repo: None,
        updated_at: crate::db::now(),
    };
    st.db.upsert_app(&row)?;

    // Point traffic at the new container, then retire the old one.
    if kind == "docker" {
        let temp_name = format!("dok-{app}-{}", &deploy_id[..8]);
        stop_container(app).await;
        let _ = Command::new("docker")
            .args(["rename", &temp_name, &container_name(app)])
            .output()
            .await;
    } else {
        // Switched to static serving: no container should keep running.
        stop_container(app).await;
    }

    // Drop the previous build tree.
    if let Some(old) = prev.and_then(|p| p.dir) {
        let old = PathBuf::from(old);
        if old != dir && old.starts_with(st.builds_dir()) {
            let _ = tokio::fs::remove_dir_all(&old).await;
        }
    }

    st.db.finish_deploy(&deploy_id, "ok", &log)?;

    Ok(DeployOutcome {
        app: app.to_string(),
        deploy_id,
        kind: kind.to_string(),
        url,
        host_port,
    })
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

pub async fn container_logs(app: &str, tail: u32) -> String {
    let out = Command::new("docker")
        .args([
            "logs",
            "--tail",
            &tail.to_string(),
            &container_name(app),
        ])
        .output()
        .await;
    match out {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).to_string();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            s
        }
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::exposed_port;

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
    fn no_expose() {
        let dir = std::env::temp_dir().join(format!("dok-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Dockerfile"), "FROM scratch\n").unwrap();
        assert_eq!(exposed_port(&dir, "Dockerfile"), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
