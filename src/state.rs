use crate::db::Db;
use axum::body::Body;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Tiny in-memory fixed-window limiter (per key, e.g. IP + bucket).
#[derive(Clone, Default)]
pub struct RateLimiter(Arc<Mutex<HashMap<String, Vec<i64>>>>);

impl RateLimiter {
    pub fn allow(&self, key: &str, limit: usize, window_secs: i64) -> bool {
        let now = crate::db::now();
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let hits = map.entry(key.to_string()).or_default();
        hits.retain(|t| *t > now - window_secs);
        if hits.len() >= limit {
            return false;
        }
        hits.push(now);
        true
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
        }
    }

    pub fn with_auth(mut self, auth: Option<Arc<crate::auth::AuthState>>) -> Self {
        self.auth = auth;
        self
    }

    pub fn with_dashboard_origin(mut self, origin: Option<String>) -> Self {
        self.dashboard_origin_host = origin.map(|o| {
            url::Url::parse(&o)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.trim_end_matches('.').to_ascii_lowercase()))
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
        let builtin = match addr {
            std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
            std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local(),
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
        url::Url::parse(&self.public_url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.trim_end_matches('.').to_ascii_lowercase()))
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
