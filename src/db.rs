use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

#[derive(Debug, Clone, Serialize)]
pub struct App {
    pub name: String,
    pub team_id: Option<String>,
    pub kind: String,
    pub image: Option<String>,
    pub memory_mb: Option<i64>,
    pub cpus: Option<f64>,
    pub host_port: Option<u16>,
    pub container_port: Option<u16>,
    pub domain: Option<String>,
    pub status: String,
    pub dir: Option<String>,
    pub repo: Option<String>,
    pub production_deploy: Option<String>,
    pub framework: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Deploy {
    pub id: String,
    pub created_at: i64,
    pub status: String,
    pub env: String,
    pub dir: Option<String>,
    pub image: Option<String>,
    pub url: Option<String>,
    pub author: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccessLog {
    pub ts: i64,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub latency_ms: i64,
    pub ip: Option<String>,
}

/// Per-project edge behaviour, stored as JSON in the settings table so no
/// extra migration is needed for older databases.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxySettings {
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub basic_auth_user: Option<String>,
    #[serde(default)]
    pub basic_auth_hash: Option<String>,
    /// "off", "www-to-apex" or "apex-to-www".
    #[serde(default)]
    pub redirect: Option<String>,
    #[serde(default)]
    pub compression: bool,
    #[serde(default)]
    pub max_body_mb: Option<u64>,
    #[serde(default)]
    pub force_https: bool,
}

pub fn can_read(role: Option<&str>) -> bool {
    matches!(role, Some("owner" | "admin" | "member" | "viewer"))
}

pub fn can_write(role: Option<&str>) -> bool {
    matches!(role, Some("owner" | "admin"))
}

pub fn hash_password(password: &str) -> String {
    let salt = uuid::Uuid::new_v4().simple().to_string();
    let digest = sha2::Sha256::digest(format!("{salt}:{password}").as_bytes());
    format!("sha256${salt}${digest:x}")
}

pub fn verify_password(stored: &str, password: &str) -> bool {
    let Some(rest) = stored.strip_prefix("sha256$") else {
        return false;
    };
    let Some((salt, want)) = rest.split_once('$') else {
        return false;
    };
    let got = format!(
        "{:x}",
        sha2::Sha256::digest(format!("{salt}:{password}").as_bytes())
    );
    got.len() == want.len()
        && got
            .bytes()
            .zip(want.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

#[derive(Debug, Clone, Serialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub slug: String,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS apps (
  name TEXT PRIMARY KEY,
  created_at INTEGER NOT NULL,
  kind TEXT NOT NULL DEFAULT 'docker',
  host_port INTEGER,
  container_port INTEGER,
  domain TEXT,
  status TEXT NOT NULL DEFAULT 'new',
  dir TEXT,
  repo TEXT,
  updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS env (
  app TEXT NOT NULL,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  PRIMARY KEY (app, key)
);
CREATE TABLE IF NOT EXISTS tokens (
  hash TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS deploys (
  id TEXT PRIMARY KEY,
  app TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  status TEXT NOT NULL,
  log TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
  hash TEXT PRIMARY KEY,
  csrf TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS passkeys (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL,
  data TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS teams (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  slug TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS members (
  team_id TEXT NOT NULL,
  user_id TEXT NOT NULL,
  role TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (team_id, user_id)
);
CREATE TABLE IF NOT EXISTS invites (
  token TEXT PRIMARY KEY,
  team_id TEXT NOT NULL,
  role TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  used_at INTEGER
);
CREATE TABLE IF NOT EXISTS env_vars (
  app TEXT NOT NULL,
  env TEXT NOT NULL,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  secret INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (app, env, key)
);
CREATE TABLE IF NOT EXISTS domains (
  host TEXT PRIMARY KEY,
  app TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS access_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  app TEXT NOT NULL,
  ts INTEGER NOT NULL,
  method TEXT NOT NULL,
  path TEXT NOT NULL,
  status INTEGER NOT NULL,
  latency_ms INTEGER NOT NULL,
  ip TEXT
);
CREATE INDEX IF NOT EXISTS access_log_app ON access_log (app, id);
"#;

fn app_from_row(r: &rusqlite::Row) -> rusqlite::Result<App> {
    Ok(App {
        name: r.get(0)?,
        kind: r.get(1)?,
        image: r.get(2)?,
        memory_mb: r.get(3)?,
        cpus: r.get(4)?,
        host_port: r.get(5)?,
        container_port: r.get(6)?,
        domain: r.get(7)?,
        status: r.get(8)?,
        dir: r.get(9)?,
        repo: r.get(10)?,
        updated_at: r.get(11)?,
        team_id: r.get(12)?,
        production_deploy: r.get(13)?,
        framework: r.get(14)?,
    })
}

const APP_COLS: &str = "name, kind, image, memory_mb, cpus, host_port, container_port, domain, status, dir, repo, updated_at, team_id, production_deploy, framework";

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        for sql in [
            "ALTER TABLE apps ADD COLUMN image TEXT",
            "ALTER TABLE apps ADD COLUMN memory_mb INTEGER",
            "ALTER TABLE apps ADD COLUMN cpus REAL",
            "ALTER TABLE apps ADD COLUMN team_id TEXT",
            "ALTER TABLE apps ADD COLUMN production_deploy TEXT",
            "ALTER TABLE apps ADD COLUMN framework TEXT",
            "ALTER TABLE deploys ADD COLUMN dir TEXT",
            "ALTER TABLE deploys ADD COLUMN env TEXT",
            "ALTER TABLE deploys ADD COLUMN image TEXT",
            "ALTER TABLE deploys ADD COLUMN url TEXT",
            "ALTER TABLE deploys ADD COLUMN author TEXT",
            "ALTER TABLE tokens ADD COLUMN user_id TEXT",
            "ALTER TABLE sessions ADD COLUMN user_id TEXT",
        ] {
            let _ = conn.execute(sql, []);
        }
        // Copy legacy env into env_vars once.
        let _ = conn.execute(
            "INSERT OR IGNORE INTO env_vars (app, env, key, value, secret)
             SELECT app, 'production', key, value, 0 FROM env",
            [],
        );
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.0.lock().expect("db mutex poisoned")
    }

    // ---------- users & passkeys ----------

    pub fn user_count(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?)
    }

    #[allow(dead_code)]
    pub fn create_user(&self, email: &str, name: &str) -> Result<User> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.conn().execute(
            "INSERT INTO users (id, email, name, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, email.to_ascii_lowercase(), name, now()],
        )?;
        Ok(User {
            id,
            email: email.to_ascii_lowercase(),
            name: name.to_string(),
        })
    }

    pub fn user(&self, id: &str) -> Result<Option<User>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, email, name FROM users WHERE id = ?1",
                params![id],
                |r| {
                    Ok(User {
                        id: r.get(0)?,
                        email: r.get(1)?,
                        name: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn user_by_email(&self, email: &str) -> Result<Option<User>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, email, name FROM users WHERE email = ?1",
                params![email.to_ascii_lowercase()],
                |r| {
                    Ok(User {
                        id: r.get(0)?,
                        email: r.get(1)?,
                        name: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn create_user_with_id(&self, id: &str, email: &str, name: &str) -> Result<User> {
        self.conn().execute(
            "INSERT INTO users (id, email, name, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, email.to_ascii_lowercase(), name, now()],
        )?;
        Ok(User {
            id: id.to_string(),
            email: email.to_ascii_lowercase(),
            name: name.to_string(),
        })
    }

    pub fn replace_passkeys(&self, user_id: &str, keys: &[String]) -> Result<()> {
        let mut c = self.conn();
        let tx = c.transaction()?;
        tx.execute("DELETE FROM passkeys WHERE user_id = ?1", params![user_id])?;
        for k in keys {
            tx.execute(
                "INSERT INTO passkeys (id, user_id, data, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![
                    uuid::Uuid::new_v4().simple().to_string(),
                    user_id,
                    k,
                    now()
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn add_passkey(&self, user_id: &str, data: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO passkeys (id, user_id, data, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                uuid::Uuid::new_v4().simple().to_string(),
                user_id,
                data,
                now()
            ],
        )?;
        Ok(())
    }

    pub fn passkeys_for(&self, user_id: &str) -> Result<Vec<String>> {
        let c = self.conn();
        let mut st = c.prepare("SELECT data FROM passkeys WHERE user_id = ?1")?;
        let rows = st.query_map(params![user_id], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn all_passkeys(&self) -> Result<Vec<(String, String)>> {
        let c = self.conn();
        let mut st = c.prepare("SELECT user_id, data FROM passkeys")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    #[allow(dead_code)]
    pub fn user_for_credential(&self, cred_id: &str) -> Result<Option<String>> {
        let c = self.conn();
        let mut st = c.prepare("SELECT user_id, data FROM passkeys")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (user_id, data) = row?;
            if data.contains(cred_id) {
                return Ok(Some(user_id));
            }
        }
        Ok(None)
    }

    // ---------- teams ----------

    pub fn create_team(&self, name: &str, slug: &str, owner: &str) -> Result<Team> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let c = self.conn();
        c.execute(
            "INSERT INTO teams (id, name, slug, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, name, slug, now()],
        )?;
        c.execute(
            "INSERT INTO members (team_id, user_id, role, created_at) VALUES (?1, ?2, 'owner', ?3)",
            params![id, owner, now()],
        )?;
        Ok(Team {
            id,
            name: name.to_string(),
            slug: slug.to_string(),
        })
    }

    pub fn teams_for(&self, user_id: &str) -> Result<Vec<Team>> {
        let c = self.conn();
        let mut st = c.prepare(
            "SELECT t.id, t.name, t.slug FROM teams t
             JOIN members m ON m.team_id = t.id WHERE m.user_id = ?1 ORDER BY t.name",
        )?;
        let rows = st.query_map(params![user_id], |r| {
            Ok(Team {
                id: r.get(0)?,
                name: r.get(1)?,
                slug: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn team(&self, id: &str) -> Result<Option<Team>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, name, slug FROM teams WHERE id = ?1",
                params![id],
                |r| {
                    Ok(Team {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        slug: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn team_by_slug(&self, slug: &str) -> Result<Option<Team>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, name, slug FROM teams WHERE slug = ?1",
                params![slug],
                |r| {
                    Ok(Team {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        slug: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn role(&self, team_id: &str, user_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT role FROM members WHERE team_id = ?1 AND user_id = ?2",
                params![team_id, user_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn members(&self, team_id: &str) -> Result<Vec<(User, String)>> {
        let c = self.conn();
        let mut st = c.prepare(
            "SELECT u.id, u.email, u.name, m.role FROM members m
             JOIN users u ON u.id = m.user_id WHERE m.team_id = ?1 ORDER BY u.name",
        )?;
        let rows = st.query_map(params![team_id], |r| {
            Ok((
                User {
                    id: r.get(0)?,
                    email: r.get(1)?,
                    name: r.get(2)?,
                },
                r.get(3)?,
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    #[allow(dead_code)]
    pub fn add_member(&self, team_id: &str, user_id: &str, role: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO members (team_id, user_id, role, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![team_id, user_id, role, now()],
        )?;
        Ok(())
    }

    pub fn create_invite(&self, team_id: &str, role: &str, ttl: i64) -> Result<String> {
        let token = uuid::Uuid::new_v4().simple().to_string();
        self.conn().execute(
            "INSERT INTO invites (token, team_id, role, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![token, team_id, role, now(), now() + ttl],
        )?;
        Ok(token)
    }

    pub fn invite(&self, token: &str) -> Result<Option<(String, String)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT team_id, role FROM invites WHERE token = ?1 AND used_at IS NULL AND expires_at > ?2",
                params![token, now()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?)
    }

    pub fn use_invite(&self, token: &str, user_id: &str) -> Result<Option<String>> {
        let Some((team_id, role)) = self.invite(token)? else {
            return Ok(None);
        };
        let c = self.conn();
        c.execute(
            "UPDATE invites SET used_at = ?2 WHERE token = ?1",
            params![token, now()],
        )?;
        c.execute(
            "INSERT OR IGNORE INTO members (team_id, user_id, role, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![team_id, user_id, role, now()],
        )?;
        Ok(Some(team_id))
    }

    // ---------- projects ----------

    pub fn list_apps(&self) -> Result<Vec<App>> {
        let c = self.conn();
        let mut st = c.prepare(&format!("SELECT {APP_COLS} FROM apps ORDER BY name"))?;
        let rows = st.query_map([], app_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn apps_for_team(&self, team_id: &str) -> Result<Vec<App>> {
        let c = self.conn();
        let mut st = c.prepare(&format!(
            "SELECT {APP_COLS} FROM apps WHERE team_id = ?1 ORDER BY name"
        ))?;
        let rows = st.query_map(params![team_id], app_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_app(&self, name: &str) -> Result<Option<App>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {APP_COLS} FROM apps WHERE name = ?1"),
                params![name],
                app_from_row,
            )
            .optional()?)
    }

    pub fn upsert_app(&self, app: &App) -> Result<()> {
        self.conn().execute(
            "INSERT INTO apps (name, created_at, kind, image, memory_mb, cpus, host_port, container_port, domain, status, dir, repo, updated_at, team_id, production_deploy, framework)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT(name) DO UPDATE SET
               kind=excluded.kind, image=COALESCE(excluded.image, apps.image),
               memory_mb=COALESCE(excluded.memory_mb, apps.memory_mb),
               cpus=COALESCE(excluded.cpus, apps.cpus),
               host_port=excluded.host_port, container_port=excluded.container_port,
               domain=COALESCE(excluded.domain, apps.domain), status=excluded.status,
               dir=excluded.dir, repo=COALESCE(excluded.repo, apps.repo),
               updated_at=excluded.updated_at,
               team_id=COALESCE(excluded.team_id, apps.team_id),
               production_deploy=COALESCE(excluded.production_deploy, apps.production_deploy),
               framework=COALESCE(excluded.framework, apps.framework)",
            params![
                app.name, now(), app.kind, app.image, app.memory_mb, app.cpus,
                app.host_port, app.container_port, app.domain, app.status, app.dir,
                app.repo, app.updated_at, app.team_id, app.production_deploy, app.framework
            ],
        )?;
        Ok(())
    }

    pub fn set_status(&self, name: &str, status: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE apps SET status = ?2, updated_at = ?3 WHERE name = ?1",
            params![name, status, now()],
        )?;
        Ok(())
    }

    pub fn set_domain(&self, name: &str, domain: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE apps SET domain = ?2, updated_at = ?3 WHERE name = ?1",
            params![name, domain, now()],
        )?;
        Ok(())
    }

    pub fn set_repo(&self, name: &str, repo: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE apps SET repo = ?2, updated_at = ?3 WHERE name = ?1",
            params![name, repo, now()],
        )?;
        Ok(())
    }

    pub fn delete_app(&self, name: &str) -> Result<()> {
        let c = self.conn();
        for table in ["env", "env_vars", "domains", "deploys"] {
            let col = if table == "domains" { "app" } else { "app" };
            c.execute(&format!("DELETE FROM {table} WHERE {col} = ?1"), params![name])?;
        }
        c.execute("DELETE FROM apps WHERE name = ?1", params![name])?;
        Ok(())
    }

    // ---------- env ----------

    pub fn env(&self, app: &str, env: &str) -> Result<Vec<(String, String, bool)>> {
        let c = self.conn();
        let mut st =
            c.prepare("SELECT key, value, secret FROM env_vars WHERE app = ?1 AND env = ?2 ORDER BY key")?;
        let rows = st.query_map(params![app, env], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Production first, then preview, then development.
    pub fn env_resolved(&self, app: &str, env: &str) -> Result<Vec<(String, String)>> {
        let mut out = std::collections::HashMap::new();
        for layer in ["production", "preview", "development"] {
            for (k, v, _) in self.env(app, layer)? {
                out.insert(k, v);
            }
            if layer == env {
                break;
            }
        }
        let mut v: Vec<_> = out.into_iter().collect();
        v.sort();
        Ok(v)
    }

    pub fn put_env(&self, app: &str, env: &str, kv: &[(String, String, bool)]) -> Result<()> {
        let mut c = self.conn();
        let tx = c.transaction()?;
        for (k, v, secret) in kv {
            tx.execute(
                "INSERT INTO env_vars (app, env, key, value, secret) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(app, env, key) DO UPDATE SET value = excluded.value, secret = excluded.secret",
                params![app, env, k, v, *secret as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_env(&self, app: &str, env: &str, key: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM env_vars WHERE app = ?1 AND env = ?2 AND key = ?3",
            params![app, env, key],
        )?;
        Ok(())
    }

    // ---------- deployments ----------

    pub fn add_deploy(
        &self,
        id: &str,
        app: &str,
        status: &str,
        dir: Option<&str>,
        env: &str,
        author: Option<&str>,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO deploys (id, app, created_at, status, dir, env, author) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, app, now(), status, dir, env, author],
        )?;
        Ok(())
    }

    pub fn deploys(&self, app: &str, limit: usize) -> Result<Vec<Deploy>> {
        self.deploys_env(app, None, limit)
    }

    pub fn deploys_env(&self, app: &str, env: Option<&str>, limit: usize) -> Result<Vec<Deploy>> {
        let c = self.conn();
        let mut st = c.prepare(
            "SELECT id, created_at, status, COALESCE(env,'production'), dir, image, url, author
             FROM deploys WHERE app = ?1 AND (?2 IS NULL OR COALESCE(env,'production') = ?2)
             ORDER BY created_at DESC, rowid DESC LIMIT ?3",
        )?;
        let rows = st.query_map(params![app, env, limit as i64], |r| {
            Ok(Deploy {
                id: r.get(0)?,
                created_at: r.get(1)?,
                status: r.get(2)?,
                env: r.get(3)?,
                dir: r.get(4)?,
                image: r.get(5)?,
                url: r.get(6)?,
                author: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn deploy(&self, app: &str, id: &str) -> Result<Option<Deploy>> {
        let c = self.conn();
        Ok(c.query_row(
            "SELECT id, created_at, status, COALESCE(env,'production'), dir, image, url, author
             FROM deploys WHERE id = ?1 AND app = ?2",
            params![id, app],
            |r| {
                Ok(Deploy {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    status: r.get(2)?,
                    env: r.get(3)?,
                    dir: r.get(4)?,
                    image: r.get(5)?,
                    url: r.get(6)?,
                    author: r.get(7)?,
                })
            },
        )
        .optional()?)
    }

    pub fn finish_deploy_meta(
        &self,
        id: &str,
        status: &str,
        log: &str,
        image: Option<&str>,
        url: Option<&str>,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE deploys SET status = ?2, log = ?3, image = COALESCE(?4, image), url = COALESCE(?5, url) WHERE id = ?1",
            params![id, status, log, image, url],
        )?;
        Ok(())
    }

    pub fn finish_deploy(&self, id: &str, status: &str, log: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE deploys SET status = ?2, log = ?3 WHERE id = ?1",
            params![id, status, log],
        )?;
        Ok(())
    }

    pub fn last_deploy_log(&self, app: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT log FROM deploys WHERE app = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![app],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    pub fn prune_deploys(&self, app: &str, keep: usize) -> Result<()> {
        self.conn().execute(
            "DELETE FROM deploys WHERE app = ?1 AND id NOT IN (
               SELECT id FROM deploys WHERE app = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2)",
            params![app, keep as i64],
        )?;
        Ok(())
    }

    pub fn set_production(&self, app: &str, deploy: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE apps SET production_deploy = ?2, updated_at = ?3 WHERE name = ?1",
            params![app, deploy, now()],
        )?;
        Ok(())
    }

    // ---------- domains ----------

    pub fn add_domain(&self, app: &str, host: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO domains (host, app, created_at) VALUES (?1, ?2, ?3)",
            params![host, app, now()],
        )?;
        Ok(())
    }

    pub fn domains_for(&self, app: &str) -> Result<Vec<String>> {
        let c = self.conn();
        let mut st = c.prepare("SELECT host FROM domains WHERE app = ?1 ORDER BY host")?;
        let rows = st.query_map(params![app], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn domain_owner(&self, host: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT app FROM domains WHERE host = ?1",
                params![host],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn remove_domain(&self, host: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM domains WHERE host = ?1", params![host])?;
        Ok(())
    }

    // ---------- tokens & sessions ----------

    pub fn add_token(&self, hash: &str, name: &str, user_id: Option<&str>) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO tokens (hash, name, created_at, user_id) VALUES (?1, ?2, ?3, ?4)",
            params![hash, name, now(), user_id],
        )?;
        Ok(())
    }

    pub fn token_user(&self, hash: &str) -> Result<Option<Option<String>>> {
        let row = self
            .conn()
            .query_row(
                "SELECT user_id FROM tokens WHERE hash = ?1",
                params![hash],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(row)
    }

    pub fn assign_orphan_tokens(&self, user_id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE tokens SET user_id = ?1 WHERE user_id IS NULL",
            params![user_id],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn token_exists(&self, hash: &str) -> Result<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM tokens WHERE hash = ?1",
                params![hash],
                |r| r.get::<_, i64>(0),
            )?
            > 0)
    }

    pub fn count_tokens(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM tokens", [], |r| r.get(0))?)
    }

    pub fn backup_to(&self, path: &std::path::Path) -> Result<()> {
        self.conn()
            .execute("VACUUM INTO ?1", params![path.to_string_lossy()])?;
        Ok(())
    }

    pub fn create_session(&self, hash: &str, user_id: &str, csrf: &str, ttl: i64) -> Result<()> {
        let c = self.conn();
        let now = now();
        c.execute("DELETE FROM sessions WHERE expires_at <= ?1", params![now])?;
        c.execute(
            "INSERT INTO sessions (hash, user_id, csrf, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![hash, user_id, csrf, now, now + ttl],
        )?;
        Ok(())
    }

    pub fn session(&self, hash: &str) -> Result<Option<(String, String, i64)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT user_id, csrf, expires_at FROM sessions WHERE hash = ?1",
                params![hash],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?)
    }

    pub fn delete_session(&self, hash: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM sessions WHERE hash = ?1", params![hash])?;
        Ok(())
    }

    // ---------- settings ----------

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn app_by_domain(&self, host: &str) -> Result<Option<App>> {
        let app: Option<String> = self
            .conn()
            .query_row(
                "SELECT app FROM domains WHERE host = ?1",
                params![host],
                |r| r.get(0),
            )
            .optional()?;
        match app {
            Some(a) => self.get_app(&a),
            None => Ok(self
                .conn()
                .query_row(
                    &format!("SELECT {APP_COLS} FROM apps WHERE domain = ?1"),
                    params![host],
                    app_from_row,
                )
                .optional()?),
        }
    }

    pub fn domains(&self) -> Result<Vec<(String, Option<String>)>> {
        let c = self.conn();
        let mut st = c.prepare("SELECT name, domain FROM apps WHERE domain IS NOT NULL")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Every hostname registered in the `domains` table, with its project.
    pub fn domain_hosts(&self) -> Result<Vec<(String, String)>> {
        let c = self.conn();
        let mut st = c.prepare("SELECT host, app FROM domains ORDER BY host")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // ---------- per-project proxy settings ----------

    pub fn proxy_settings(&self, app: &str) -> ProxySettings {
        self.get_setting(&format!("proxy:{app}"))
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn set_proxy_settings(&self, app: &str, settings: &ProxySettings) -> Result<()> {
        self.set_setting(&format!("proxy:{app}"), &serde_json::to_string(settings)?)
    }

    // ---------- access log ----------

    pub fn log_access(
        &self,
        app: &str,
        method: &str,
        path: &str,
        status: u16,
        latency_ms: i64,
        ip: Option<&str>,
    ) {
        let c = self.conn();
        let _ = c.execute(
            "INSERT INTO access_log (app, ts, method, path, status, latency_ms, ip)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![app, now(), method, path, status as i64, latency_ms, ip],
        );
        // Keep the last 200 requests per project; cheap bounded retention.
        let _ = c.execute(
            "DELETE FROM access_log WHERE app = ?1 AND id NOT IN (
               SELECT id FROM access_log WHERE app = ?1 ORDER BY id DESC LIMIT 200)",
            params![app],
        );
    }

    pub fn access_logs(&self, app: &str, limit: usize) -> Result<Vec<AccessLog>> {
        let c = self.conn();
        let mut st = c.prepare(
            "SELECT ts, method, path, status, latency_ms, ip FROM access_log
             WHERE app = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![app, limit as i64], |r| {
            Ok(AccessLog {
                ts: r.get(0)?,
                method: r.get(1)?,
                path: r.get(2)?,
                status: r.get::<_, i64>(3)? as u16,
                latency_ms: r.get(4)?,
                ip: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::{can_read, can_write, hash_password, verify_password, Db};
    use std::path::PathBuf;

    fn tmp_db() -> (Db, PathBuf) {
        let path = std::env::temp_dir().join(format!("dok-db-{}.db", uuid::Uuid::new_v4()));
        let db = Db::open(&path).unwrap();
        (db, path)
    }

    #[test]
    fn authz_roles() {
        assert!(can_read(Some("owner")));
        assert!(can_read(Some("viewer")));
        assert!(!can_read(Some("nobody")));
        assert!(!can_read(None));
        assert!(can_write(Some("owner")));
        assert!(can_write(Some("admin")));
        assert!(!can_write(Some("member")));
        assert!(!can_write(Some("viewer")));
        assert!(!can_write(None));
    }

    #[test]
    fn passwords_roundtrip() {
        let h = hash_password("hunter2");
        assert!(verify_password(&h, "hunter2"));
        assert!(!verify_password(&h, "wrong"));
        assert!(!verify_password("plain", "hunter2"));
    }

    #[test]
    fn env_inherits_production() {
        let (db, path) = tmp_db();
        db.put_env("app", "production", &[("A".into(), "1".into(), false)])
            .unwrap();
        db.put_env(
            "app",
            "preview",
            &[("A".into(), "2".into(), false), ("B".into(), "3".into(), false)],
        )
        .unwrap();
        let prod = db.env_resolved("app", "production").unwrap();
        assert_eq!(prod, vec![("A".into(), "1".into())]);
        let preview = db.env_resolved("app", "preview").unwrap();
        assert_eq!(
            preview,
            vec![("A".into(), "2".into()), ("B".into(), "3".into())]
        );
        let dev = db.env_resolved("app", "development").unwrap();
        assert_eq!(
            dev,
            vec![("A".into(), "2".into()), ("B".into(), "3".into())]
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
