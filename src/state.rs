use crate::db::Db;
use axum::body::Body;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

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
    /// Hosts allowed to call the management API.
    pub api_hosts: Vec<String>,
    /// Seconds to wait for upstream response headers.
    pub upstream_timeout: u64,
    /// Passkey dashboard, when configured.
    pub auth: Option<Arc<crate::auth::AuthState>>,
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
        let mut api_hosts = vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
            "[::1]".to_string(),
        ];
        if let Some(h) = parsed.as_ref().and_then(|u| u.host_str()) {
            api_hosts.push(h.to_string());
        }

        Self {
            db,
            data,
            client,
            base_domain,
            public_url,
            scheme,
            api_hosts,
            upstream_timeout: 60,
            auth: None,
        }
    }

    pub fn with_auth(mut self, auth: Option<Arc<crate::auth::AuthState>>) -> Self {
        self.auth = auth;
        self
    }

    pub fn builds_dir(&self) -> PathBuf {
        self.data.join("builds")
    }
}
