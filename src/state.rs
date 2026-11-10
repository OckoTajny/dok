use crate::db::Db;
use axum::body::Body;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One point-in-time runtime sample. Kept in memory; restarts start a new
/// series. CPU is a percentage, memory is MiB (both parsed from `docker stats`).
#[derive(Debug, Clone, Serialize)]
pub struct MetricSample {
    pub ts: i64,
    pub cpu: Option<f64>,
    pub memory_mb: Option<f64>,
}

fn parse_cpu(s: &str) -> Option<f64> {
    s.trim().trim_end_matches('%').trim().parse().ok()
}

fn parse_memory_mb(s: &str) -> Option<f64> {
    let used = s.split('/').next()?.trim();
    let lower = used.to_ascii_lowercase();
    let (num, factor) = if let Some(v) = lower
        .strip_suffix("gib")
        .or_else(|| lower.strip_suffix("gb"))
    {
        (v, 1024.0)
    } else if let Some(v) = lower
        .strip_suffix("mib")
        .or_else(|| lower.strip_suffix("mb"))
    {
        (v, 1.0)
    } else if let Some(v) = lower
        .strip_suffix("kib")
        .or_else(|| lower.strip_suffix("kb"))
    {
        (v, 1.0 / 1024.0)
    } else {
        (lower.as_str(), 1.0)
    };
    let value: f64 = num.trim().parse().ok()?;
    Some(value * factor)
}

/// Tiny in-memory fixed-window limiter (per key, e.g. IP + bucket).
#[derive(Clone, Default)]
pub struct RateLimiter(Arc<Mutex<HashMap<String, Vec<i64>>>>);

impl RateLimiter {
    pub fn allow(&self, key: &str, limit: usize, window_secs: i64) -> bool {
        let now = crate::db::now();
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let allowed = {
            let hits = map.entry(key.to_string()).or_default();
            hits.retain(|t| *t > now - window_secs);
            if hits.len() >= limit {
                false
            } else {
                hits.push(now);
                true
            }
        };
        // Evict expired buckets and cap the map so a spray of client IPs or
        // basic-auth usernames cannot grow it without bound.
        if map.get(key).map(|h| h.is_empty()).unwrap_or(false) {
            map.remove(key);
        }
        if map.len() > 1024 {
            map.retain(|_, hits| {
                hits.retain(|t| *t > now - 3600);
                !hits.is_empty()
            });
        }
        allowed
    }

    /// Check the window without recording a hit. Lets callers reject before
    /// doing expensive work such as Argon2 password verification.
    pub fn blocked(&self, key: &str, limit: usize, window_secs: i64) -> bool {
        let now = crate::db::now();
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let hits = map.entry(key.to_string()).or_default();
        hits.retain(|t| *t > now - window_secs);
        let blocked = hits.len() >= limit;
        if map.len() > 1024 {
            map.retain(|_, hits| {
                hits.retain(|t| *t > now - 3600);
                !hits.is_empty()
            });
        }
        blocked
    }

    /// Record one event (for example a failed basic-auth attempt).
    pub fn hit(&self, key: &str) {
        let now = crate::db::now();
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let hits = map.entry(key.to_string()).or_default();
        hits.push(now);
        if hits.len() > 256 {
            hits.drain(..hits.len() - 256);
        }
    }
}

pub type HttpClient = Client<HttpConnector, Body>;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub data: PathBuf,
    pub client: HttpClient,
    /// Base domain routed through this instance: `<app>.<base_domain>`.
    pub base_domain: Option<String>,
    /// Public base URL used in CLI output (http://host:port or https://host).
    pub public_url: String,
    /// Scheme clients use to reach us.
    pub scheme: String,
    /// Seconds to wait for upstream response headers.
    pub upstream_timeout: u64,
    /// Passkey dashboard, when configured.
    pub auth: Option<Arc<crate::auth::AuthState>>,
    /// Host of the canonical dashboard origin (may differ from `public_url`
    /// when `public_url` is an IP address and the passkey origin is localhost).
    pub dashboard_origin_host: Option<String>,
    /// Optional Traefik file-provider directory for per-app routers.
    pub traefik_dir: Option<PathBuf>,
    pub rate: RateLimiter,
    pub trusted_proxies: Vec<ipnet::IpNet>,
    /// Bound concurrent builds/deploys and log streams so one token cannot
    /// exhaust the host.
    pub deploy_gate: Arc<tokio::sync::Semaphore>,
    pub stream_gate: Arc<tokio::sync::Semaphore>,
    /// Recent CPU/memory samples per project, newest last (bounded ring).
    pub metrics: Arc<Mutex<HashMap<String, VecDeque<MetricSample>>>>,
}

impl AppState {
    pub fn new(db: Db, data: PathBuf, base_domain: Option<String>, public_url: String) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(Duration::from_secs(10)));
        connector.set_nodelay(true);
        let client = Client::builder(TokioExecutor::new()).build(connector);

        let parsed = url::Url::parse(&public_url).ok();
        let scheme = parsed
            .as_ref()
            .map(|u| u.scheme().to_string())
            .unwrap_or_else(|| "http".into());
        Self {
            db,
            data,
            client,
            base_domain,
            public_url,
            scheme,
            upstream_timeout: 60,
            auth: None,
            dashboard_origin_host: None,
            traefik_dir: None,
            rate: RateLimiter::default(),
            trusted_proxies: Vec::new(),
            deploy_gate: Arc::new(tokio::sync::Semaphore::new(4)),
            stream_gate: Arc::new(tokio::sync::Semaphore::new(16)),
            metrics: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Record a `docker stats` snapshot. Keeps roughly one hour at the default
    /// 30s sampling interval.
    pub fn record_metrics(&self, app: &str, stats: &serde_json::Value) {
        let cpu = stats["cpu"].as_str().and_then(parse_cpu);
        let memory_mb = stats["memory"].as_str().and_then(parse_memory_mb);
        if cpu.is_none() && memory_mb.is_none() {
            return;
        }
        let mut map = self.metrics.lock().unwrap_or_else(|e| e.into_inner());
        let series = map.entry(app.to_string()).or_default();
        series.push_back(MetricSample {
            ts: crate::db::now(),
            cpu,
            memory_mb,
        });
        while series.len() > 120 {
            series.pop_front();
        }
    }

    pub fn metric_history(&self, app: &str, limit: usize) -> Vec<MetricSample> {
        let map = self.metrics.lock().unwrap_or_else(|e| e.into_inner());
        map.get(app)
            .map(|s| {
                let skip = s.len().saturating_sub(limit);
                s.iter().skip(skip).cloned().collect()
            })
            .unwrap_or_default()
    }

    pub fn with_auth(mut self, auth: Option<Arc<crate::auth::AuthState>>) -> Self {
        self.auth = auth;
        self
    }

    pub fn with_dashboard_origin(mut self, origin: Option<String>) -> Self {
        self.dashboard_origin_host = origin.map(|o| {
            url::Url::parse(&o)
                .ok()
                .and_then(|u| {
                    u.host_str()
                        .map(|h| h.trim_end_matches('.').to_ascii_lowercase())
                })
                .unwrap_or_default()
        });
        self
    }

    pub fn with_traefik_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.traefik_dir = dir;
        self
    }

    pub fn with_trusted_proxies(mut self, nets: Vec<ipnet::IpNet>) -> Self {
        self.trusted_proxies = nets;
        self
    }

    /// Only loopback/private peers (or explicitly configured CIDRs) may set
    /// X-Forwarded-Proto.
    pub fn trusted_peer(&self, ip: &str) -> bool {
        let Ok(addr) = ip.parse::<std::net::IpAddr>() else {
            return false;
        };
        // Only loopback is trusted by default. Docker/Traefik private ranges
        // must be granted explicitly with --trusted-proxy.
        let builtin = match addr {
            std::net::IpAddr::V4(v4) => v4.is_loopback(),
            std::net::IpAddr::V6(v6) => v6.is_loopback(),
        };
        builtin || self.trusted_proxies.iter().any(|n| n.contains(&addr))
    }

    /// Rewrite the Traefik routers for every app that has a hostname.
    /// Traefik only requests a certificate when it can see the exact Host().
    pub fn sync_traefik(&self) {
        let Some(dir) = &self.traefik_dir else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let mut hosts = std::collections::HashSet::new();
        for a in self.db.list_apps().unwrap_or_default() {
            if let Some(d) = &a.domain {
                hosts.insert(d.clone());
            }
            if let Some(b) = &self.base_domain {
                hosts.insert(format!("{}.{}", a.name, b));
            }
            for h in self.db.domains_for(&a.name).unwrap_or_default() {
                hosts.insert(h);
            }
        }
        for (h, _) in self.db.domain_hosts().unwrap_or_default() {
            hosts.insert(h);
        }
        // Preview deployments are served on `<app>-<deploy8>.<base>`; without
        // an explicit router Traefik never forwards those hosts to dok.
        for h in self.db.preview_hosts() {
            hosts.insert(h);
        }
        let mut routers = String::new();
        let mut hosts: Vec<_> = hosts.into_iter().collect();
        hosts.sort();
        for host in hosts {
            let id: String = host
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect();
            routers.push_str(&format!(
                "    dok-app-{id}-https:\n      entryPoints: [https]\n      rule: Host(`{host}`)\n      service: dok\n      tls:\n        certResolver: letsencrypt\n    dok-app-{id}-http:\n      entryPoints: [http]\n      rule: Host(`{host}`)\n      middlewares: [dok-redirect-https]\n      service: dok\n"
            ));
        }
        let body = format!("# generated by dok; do not edit\nhttp:\n  routers:\n{routers}");
        let path = dir.join("dok-apps.yaml");
        let tmp = dir.join(".dok-apps.yaml.tmp");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    /// Cross-process per-app deploy lock (flock on `<data>/locks/<app>.lock`).
    pub async fn lock_deploy(&self, app: &str) -> anyhow::Result<std::fs::File> {
        use fs2::FileExt;
        let path = self.data.join("locks").join(format!("{app}.lock"));
        tokio::task::spawn_blocking(move || -> anyhow::Result<std::fs::File> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&path)?;
            file.lock_exclusive()?;
            Ok(file)
        })
        .await?
    }

    /// Hostname that serves the dashboard/API; never routed to an app.
    pub fn dashboard_host(&self) -> Option<String> {
        url::Url::parse(&self.public_url).ok().and_then(|u| {
            u.host_str()
                .map(|h| h.trim_end_matches('.').to_ascii_lowercase())
        })
    }

    /// True when `host` belongs to the control plane, not to user projects.
    pub fn reserved_host(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if host.is_empty() {
            return false;
        }
        if Some(host.as_str()) == self.dashboard_host().as_deref() {
            return true;
        }
        if let Some(base) = &self.base_domain {
            let base = base.trim_end_matches('.').to_ascii_lowercase();
            if host == base {
                return true;
            }
        }
        false
    }

    /// The dashboard, passkey auth and token API are only served on the
    /// dashboard host(s), never on project or unknown hosts.
    pub fn is_control_host(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if host.is_empty() {
            // HTTP/1.0 and local `tower` tests have no Host header.
            return true;
        }
        let dash = self.dashboard_host();
        let origin = self.dashboard_origin_host.clone();
        if dash.as_deref() == Some(host.as_str()) || origin.as_deref() == Some(host.as_str()) {
            return true;
        }
        let loopback = |h: &str| h == "localhost" || h == "127.0.0.1" || h == "[::1]" || h == "::1";
        (loopback(&host) && dash.as_deref().map(loopback).unwrap_or(false))
            || (loopback(&host) && origin.as_deref().map(loopback).unwrap_or(false))
    }

    pub fn is_base_subdomain(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        match &self.base_domain {
            Some(base) => {
                let base = base.trim_end_matches('.').to_ascii_lowercase();
                host.ends_with(&format!(".{base}"))
            }
            None => false,
        }
    }

    pub fn builds_dir(&self) -> PathBuf {
        self.data.join("builds")
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_cpu, parse_memory_mb};

    #[test]
    fn parses_docker_stats_units() {
        assert_eq!(parse_cpu("12.50%"), Some(12.5));
        assert_eq!(parse_cpu("0.00%"), Some(0.0));
        assert_eq!(parse_memory_mb("12.5MiB / 512MiB"), Some(12.5));
        assert_eq!(parse_memory_mb("1.5GiB / 2GiB"), Some(1536.0));
        assert_eq!(parse_memory_mb("512KiB / 1MiB"), Some(0.5));
    }
}
