use crate::db::Db;
use axum::body::Body;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::path::PathBuf;
use std::sync::Arc;

pub type HttpClient = Client<HttpConnector, Body>;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub data: PathBuf,
    pub client: HttpClient,
    /// Optional base domain: `<app>.<base_domain>` resolves to the app.
    pub base_domain: Option<String>,
    /// Public base URL used in CLI output (http://host:port or https://host).
    pub public_url: String,
    /// Passkey dashboard, when configured.
    pub auth: Option<Arc<crate::auth::AuthState>>,
}

impl AppState {
    pub fn new(db: Db, data: PathBuf, base_domain: Option<String>, public_url: String) -> Self {
        let client = Client::builder(TokioExecutor::new()).build_http();
        Self {
            db,
            data,
            client,
            base_domain,
            public_url,
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
