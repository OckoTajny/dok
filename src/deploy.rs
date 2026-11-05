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
    pub memory_mb: Option<i64>,
    pub cpus: Option<f64>,
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
    // One deploy per app at a time, across processes (the git hook runs its own).
    let _lock = st.lock_deploy(app).await?;
    let deploy_id = uuid::Uuid::new_v4().simple().to_string();
    st.db
        .add_deploy(&deploy_id, app, "running", Some(&dir.to_string_lossy()))?;
    let mut guard = DeployGuard {
        st,
        app,
        id: deploy_id.clone(),
        done: false,
    };

    let prev = st.db.get_app(app)?;
    let conf = read_dok_toml(dir);
    let chosen_domain = match domain_hint.clone().or_else(|| conf.domain.clone()) {
        Some(d) => {
            let d = d.trim().trim_end_matches('.').to_ascii_lowercase();
            if !crate::api::valid_host(&d) {
                bail!("invalid domain: {d}");
            }
            Some(d)
        }
        None => None,
    };
    let dockerfile = conf.dockerfile.clone().unwrap_or_else(|| "Dockerfile".into());
    let memory_mb = conf.memory_mb.unwrap_or(512).max(32);
    let cpus = conf.cpus.unwrap_or(1.0);
    let has_dockerfile = dir.join(&dockerfile).is_file();
    let mut log = String::new();

    let (kind, host_port, container_port) = if has_dockerfile {
        if !docker_available().await {
            bail!("docker is not available on this host");
        }
        let container_port = port_hint
            .or(conf.port)
            .or_else(|| exposed_port(dir, &dockerfile))
            .unwrap_or(3000);

        let tag = format!("dok/{app}:{deploy_id}");
        log.push_str(&format!("$ docker build -f {dockerfile} -t {tag} .\n"));
        let out = Command::new("docker")
            .args(["build", "-f", &dockerfile, "-t", &tag, "--label", &format!("dok.app={app}"), "."])
            .current_dir(dir)
            .output()
            .await
            .context("run docker build")?;
        log.push_str(&String::from_utf8_lossy(&out.stdout));
        log.push_str(&String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            st.db.finish_deploy(&deploy_id, "failed", &log)?;
            guard.done = true;
            st.db.set_status(app, "failed")?;
            bail!("docker build failed (see `dok logs {app}`)");
        }

        // Start the replacement first; the old container keeps serving until the
        // new one is running and the DB points at it. Let Docker pick the port.
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
            &format!("127.0.0.1::{container_port}"),
            "--memory",
            &format!("{}m", memory_mb),
            "--cpus",
            &format!("{cpus}"),
        ]);
        for (k, v) in st.db.env(app)? {
            cmd.args(["-e", &format!("{k}={v}")]);
        }
        cmd.arg(&tag);
        let out = cmd.output().await.context("run docker run")?;
        log.push_str(&String::from_utf8_lossy(&out.stdout));
        log.push_str(&String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            remove_container(&temp_name).await;
            st.db.finish_deploy(&deploy_id, "failed", &log)?;
            guard.done = true;
            st.db.set_status(app, "failed")?;
            bail!("docker run failed (see `dok logs {app}`)");
        }
        let host_port = match mapped_port(&temp_name, container_port).await {
            Some(p) => p,
            None => {
                remove_container(&temp_name).await;
                bail!("docker did not report a host port");
            }
        };
        ("docker", Some(host_port), Some(container_port))
    } else {
        ("static", None, None)
    };

    let row = crate::db::App {
        name: app.to_string(),
        kind: kind.to_string(),
        image: if kind == "docker" {
            Some(format!("dok/{app}:{deploy_id}"))
        } else {
            None
        },
        memory_mb: Some(memory_mb),
        cpus: Some(cpus),
        host_port,
        container_port,
        domain: chosen_domain,
        status: "running".into(),
        dir: Some(dir.to_string_lossy().to_string()),
        repo: None,
        updated_at: crate::db::now(),
    };
    st.db.upsert_app(&row)?;
    st.sync_traefik();

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

    // Drop the previous image and build tree, keep the last few for rollback.
    if let Some(prev) = &prev {
        if kind == "docker" {
            if let Some(old_image) = &prev.image {
                if old_image != &format!("dok/{app}:{deploy_id}") {
                    let _ = Command::new("docker").args(["rmi", old_image]).output().await;
                }
            }
        }
    }
    let _ = st.db.prune_deploys(app, 3);
    cleanup_builds(st, app).await;

    st.db.finish_deploy(&deploy_id, "ok", &log)?;
    guard.done = true;

    let url = st
        .db
        .get_app(app)?
        .and_then(|a| a.domain)
        .map(|d| format!("{}://{d}", st.scheme))
        .or_else(|| {
            st.base_domain
                .as_ref()
                .map(|b| format!("{}://{app}.{b}", st.scheme))
        })
        .unwrap_or_else(|| st.public_url.clone());

    Ok(DeployOutcome {
        app: app.to_string(),
        deploy_id,
        kind: kind.to_string(),
        url,
        host_port,
    })
}

struct DeployGuard<'a> {
    st: &'a AppState,
    app: &'a str,
    id: String,
    done: bool,
}

impl Drop for DeployGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = self
                .st
                .db
                .finish_deploy(&self.id, "failed", "deploy aborted before completion");
            let _ = self.st.db.set_status(self.app, "failed");
        }
    }
}

/// Keep only the build trees referenced by the last three deploys.
async fn cleanup_builds(st: &AppState, app: &str) {
    use std::collections::HashSet;
    let keep: HashSet<String> = st
        .db
        .deploys(app, 3)
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

/// Redeploy an earlier build (dashboard rollback).
pub async fn rollback(
    st: &AppState,
    app: &str,
    deploy_id: &str,
) -> Result<DeployOutcome> {
    let dir = st
        .db
        .deploy_dir_for(app, deploy_id)?
        .ok_or_else(|| anyhow::anyhow!("no build for deploy {deploy_id}"))?;
    let path = std::path::PathBuf::from(&dir);
    if !path.is_dir() {
        bail!("build for that deploy is gone (only the last 3 are kept)");
    }
    deploy_dir(st, app, &path, None, None).await
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
}
