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
    pub container: Option<String>,
    pub host_port: Option<u16>,
    pub commit_sha: Option<String>,
    pub commit_message: Option<String>,
    pub commit_author: Option<String>,
    pub branch: Option<String>,
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

/// An admin may invite members/viewers; only an owner may grant admin/owner.
pub fn can_grant(actor: Option<&str>, target: &str) -> bool {
    match actor {
        Some("owner") => matches!(target, "owner" | "admin" | "member" | "viewer"),
        Some("admin") => matches!(target, "member" | "viewer"),
        _ => false,
    }
}

pub fn hash_password(password: &str) -> String {
    use argon2::password_hash::SaltString;
    use argon2::{Argon2, PasswordHasher};
    use rand_core::OsRng;
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .unwrap_or_default()
}

pub fn verify_password(stored: &str, password: &str) -> bool {
    use argon2::{Argon2, PasswordHash, PasswordVerifier};
    if let Some(rest) = stored.strip_prefix("sha256$") {
        // Legacy hashes from before argon2; verify then the next save upgrades.
        let Some((salt, want)) = rest.split_once('$') else {
            return false;
        };
        let got = format!(
            "{:x}",
            sha2::Sha256::digest(format!("{salt}:{password}").as_bytes())
        );
        return got.len() == want.len()
            && got
                .bytes()
                .zip(want.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0;
    }
    PasswordHash::new(stored)
        .map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
        .unwrap_or(false)
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
CREATE TABLE IF NOT EXISTS token_blacklist (
  hash TEXT PRIMARY KEY,
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
  used_at INTEGER,
  invited_by TEXT
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

const DB_VERSION: i64 = 4;

/// One transactional step per schema version. New databases start at 0 and
/// run the same steps as old ones so migrations are exercised everywhere.
fn migrate(conn: &mut Connection) -> Result<()> {
    loop {
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version >= DB_VERSION {
            return Ok(());
        }
        let tx = conn.transaction()?;
        match version {
            0 => {
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
                    let _ = tx.execute(sql, []);
                }
                // Keep one owner per legacy domain before the unique index.
                let _ = tx.execute(
                    "UPDATE apps SET domain = NULL WHERE domain IS NOT NULL AND rowid NOT IN (
                       SELECT MAX(rowid) FROM apps WHERE domain IS NOT NULL GROUP BY domain)",
                    [],
                );
                let _ = tx.execute(
                    "CREATE UNIQUE INDEX IF NOT EXISTS apps_domain_unique
                     ON apps(domain) WHERE domain IS NOT NULL",
                    [],
                );
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO env_vars (app, env, key, value, secret)
                     SELECT app, 'production', key, value, 0 FROM env",
                    [],
                );
            }
            1 => {
                let _ = tx.execute_batch(
                    "CREATE TABLE IF NOT EXISTS access_log (
                       id INTEGER PRIMARY KEY AUTOINCREMENT, app TEXT NOT NULL,
                       ts INTEGER NOT NULL, method TEXT NOT NULL, path TEXT NOT NULL,
                       status INTEGER NOT NULL, latency_ms INTEGER NOT NULL, ip TEXT);
                     CREATE INDEX IF NOT EXISTS access_log_app ON access_log (app, id);",
                );
            }
            2 => {
                for sql in [
                    "ALTER TABLE deploys ADD COLUMN container TEXT",
                    "ALTER TABLE deploys ADD COLUMN host_port INTEGER",
                    "ALTER TABLE deploys ADD COLUMN commit_sha TEXT",
                    "ALTER TABLE deploys ADD COLUMN commit_message TEXT",
                    "ALTER TABLE deploys ADD COLUMN commit_author TEXT",
                    "ALTER TABLE deploys ADD COLUMN branch TEXT",
                ] {
                    let _ = tx.execute(sql, []);
                }
            }
            3 => {
                let _ = tx.execute("ALTER TABLE invites ADD COLUMN invited_by TEXT", []);
            }
            _ => break,
        }
        tx.pragma_update(None, "user_version", version + 1)?;
        tx.commit()?;
    }
    Ok(())
}

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
        let mut conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        migrate(&mut conn)?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
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

    pub fn single_user(&self) -> Result<Option<User>> {
        let c = self.conn();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
        if n != 1 {
            return Ok(None);
        }
        Ok(c.query_row(
            "SELECT id, email, name FROM users LIMIT 1",
            [],
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

    pub fn update_member_role(&self, team_id: &str, user_id: &str, role: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "UPDATE members SET role = ?3 WHERE team_id = ?1 AND user_id = ?2",
            params![team_id, user_id, role],
        )?)
    }

    pub fn remove_member(&self, team_id: &str, user_id: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM members WHERE team_id = ?1 AND user_id = ?2",
            params![team_id, user_id],
        )?)
    }

    pub fn owner_count(&self, team_id: &str) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM members WHERE team_id = ?1 AND role = 'owner'",
            params![team_id],
            |r| r.get(0),
        )?)
    }

    pub fn invites_for(
        &self,
        team_id: &str,
    ) -> Result<Vec<(String, String, i64, i64, Option<i64>, Option<String>)>> {
        let c = self.conn();
        let mut st = c.prepare(
            "SELECT token, role, created_at, expires_at, used_at, invited_by FROM invites
             WHERE team_id = ?1 ORDER BY created_at DESC, rowid DESC",
        )?;
        let rows = st.query_map(params![team_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Team name, role and inviter for the join page. Expired/used invites
    /// return `None` so the page can fail immediately instead of after the
    /// passkey ceremony.
    pub fn invite_details(&self, token: &str) -> Result<Option<(String, String, Option<String>)>> {
        let c = self.conn();
        Ok(c.query_row(
            "SELECT t.name, i.role,
                    COALESCE(u.name, u.email)
             FROM invites i
             JOIN teams t ON t.id = i.team_id
             LEFT JOIN users u ON u.id = i.invited_by
             WHERE i.token = ?1 AND i.used_at IS NULL AND i.expires_at > ?2",
            params![token, now()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?)
    }

    pub fn revoke_invite(&self, team_id: &str, token: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM invites WHERE team_id = ?1 AND token = ?2",
            params![team_id, token],
        )?)
    }

    pub fn create_invite(
        &self,
        team_id: &str,
        role: &str,
        ttl: i64,
        invited_by: Option<&str>,
    ) -> Result<String> {
        let token = uuid::Uuid::new_v4().simple().to_string();
        self.conn().execute(
            "INSERT INTO invites (token, team_id, role, created_at, expires_at, invited_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![token, team_id, role, now(), now() + ttl, invited_by],
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
        let changed = c.execute(
            "UPDATE invites SET used_at = ?2 WHERE token = ?1 AND used_at IS NULL AND expires_at > ?2",
            params![token, now()],
        )?;
        if changed != 1 {
            return Ok(None);
        }
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

    pub fn clear_domain(&self, name: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "UPDATE apps SET domain = NULL, updated_at = ?2 WHERE name = ?1",
            params![name, now()],
        )?)
    }

    pub fn set_repo(&self, name: &str, repo: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE apps SET repo = ?2, updated_at = ?3 WHERE name = ?1",
            params![name, repo, now()],
        )?;
        Ok(())
    }

    pub fn delete_app(&self, name: &str) -> Result<()> {
        let hosts: Vec<String> = self.domains_for(name).unwrap_or_default();
        let app_domain = self
            .get_app(name)
            .ok()
            .flatten()
            .and_then(|a| a.domain)
            .into_iter();
        let c = self.conn();
        for table in ["env", "env_vars", "domains", "deploys", "access_log"] {
            c.execute(&format!("DELETE FROM {table} WHERE app = ?1"), params![name])?;
        }
        c.execute(
            "DELETE FROM settings WHERE key = ?1",
            params![format!("proxy:{name}")],
        )?;
        for host in hosts.into_iter().chain(app_domain) {
            c.execute(
                "DELETE FROM settings WHERE key = ?1",
                params![format!("cert:{host}")],
            )?;
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
            "SELECT id, created_at, status, COALESCE(env,'production'), dir, image, url, author,
                    container, host_port, commit_sha, commit_message, commit_author, branch
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
                container: r.get(8)?,
                host_port: r.get::<_, Option<i64>>(9)?.map(|p| p as u16),
                commit_sha: r.get(10)?,
                commit_message: r.get(11)?,
                commit_author: r.get(12)?,
                branch: r.get(13)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn deploy(&self, app: &str, id: &str) -> Result<Option<Deploy>> {
        let c = self.conn();
        Ok(c.query_row(
            "SELECT id, created_at, status, COALESCE(env,'production'), dir, image, url, author,
                    container, host_port, commit_sha, commit_message, commit_author, branch
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
                    container: r.get(8)?,
                    host_port: r.get::<_, Option<i64>>(9)?.map(|p| p as u16),
                    commit_sha: r.get(10)?,
                    commit_message: r.get(11)?,
                    commit_author: r.get(12)?,
                    branch: r.get(13)?,
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

    pub fn deploy_log(&self, app: &str, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT log FROM deploys WHERE app = ?1 AND id = ?2",
                params![app, id],
                |r| r.get(0),
            )
            .optional()?)
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

    /// Global retention (both envs). Never removes the production pointer.
    #[allow(dead_code)]
    pub fn prune_deploys(&self, app: &str, keep: usize) -> Result<Vec<Deploy>> {
        self.prune_inner(app, None, keep)
    }

    /// Per-environment retention. Pruning previews never sees production rows.
    pub fn prune_deploys_env(&self, app: &str, env: &str, keep: usize) -> Result<Vec<Deploy>> {
        self.prune_inner(app, Some(env), keep)
    }

    fn prune_inner(&self, app: &str, env: Option<&str>, keep: usize) -> Result<Vec<Deploy>> {
        use std::collections::HashSet;
        let all = self.deploys_env(app, env, 100_000)?;
        let production = if env == Some("production") {
            self.get_app(app)?
                .and_then(|a| a.production_deploy)
                .unwrap_or_default()
        } else if env.is_none() {
            self.get_app(app)?
                .and_then(|a| a.production_deploy)
                .unwrap_or_default()
        } else {
            String::new()
        };
        let mut keep_ids: HashSet<String> = HashSet::new();
        if env.is_none() {
            for e in ["production", "preview"] {
                for d in all.iter().filter(|d| d.env == e).take(keep) {
                    keep_ids.insert(d.id.clone());
                }
            }
            if !production.is_empty() {
                keep_ids.insert(production);
            }
        } else {
            for d in all.iter().take(keep) {
                keep_ids.insert(d.id.clone());
            }
            if !production.is_empty() {
                keep_ids.insert(production);
            }
        }
        let mut removed = Vec::new();
        let c = self.conn();
        for d in all {
            if !keep_ids.contains(&d.id) {
                if c.execute("DELETE FROM deploys WHERE id = ?1", params![d.id])
                    .is_ok()
                {
                    removed.push(d);
                }
            }
        }
        Ok(removed)
    }

    pub fn set_deploy_routing(
        &self,
        id: &str,
        container: Option<&str>,
        host_port: Option<u16>,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE deploys SET container = ?2, host_port = ?3 WHERE id = ?1",
            params![id, container, host_port.map(|p| p as i64)],
        )?;
        Ok(())
    }

    pub fn set_deploy_commit(
        &self,
        id: &str,
        sha: &str,
        message: &str,
        author: &str,
        branch: &str,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE deploys SET commit_sha = ?2, commit_message = ?3, commit_author = ?4, branch = ?5 WHERE id = ?1",
            params![id, sha, message, author, branch],
        )?;
        Ok(())
    }

    pub fn deploy_by_prefix(&self, app: &str, prefix: &str) -> Result<Option<Deploy>> {
        let c = self.conn();
        Ok(c.query_row(
            "SELECT id, created_at, status, COALESCE(env,'production'), dir, image, url, author,
                    container, host_port, commit_sha, commit_message, commit_author, branch
             FROM deploys WHERE app = ?1 AND id LIKE ?2 || '%' ORDER BY created_at DESC LIMIT 1",
            params![app, prefix],
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
                    container: r.get(8)?,
                    host_port: r.get::<_, Option<i64>>(9)?.map(|p| p as u16),
                    commit_sha: r.get(10)?,
                    commit_message: r.get(11)?,
                    commit_author: r.get(12)?,
                    branch: r.get(13)?,
                })
            },
        )
        .optional()?)
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

    pub fn deploy_counts(&self, app: &str) -> Result<(i64, i64, i64)> {
        let c = self.conn();
        let total: i64 = c.query_row(
            "SELECT COUNT(*) FROM deploys WHERE app = ?1",
            params![app],
            |r| r.get(0),
        )?;
        let production: i64 = c.query_row(
            "SELECT COUNT(*) FROM deploys WHERE app = ?1 AND env = 'production'",
            params![app],
            |r| r.get(0),
        )?;
        let preview: i64 = c.query_row(
            "SELECT COUNT(*) FROM deploys WHERE app = ?1 AND env = 'preview'",
            params![app],
            |r| r.get(0),
        )?;
        Ok((total, production, preview))
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

    pub fn remove_domain(&self, app: &str, host: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM domains WHERE host = ?1 AND app = ?2",
            params![host, app],
        )?)
    }

    /// Owner of an `apps.domain` value (legacy domain column).
    pub fn app_domain_owner(&self, host: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT name FROM apps WHERE domain = ?1 LIMIT 1",
                params![host],
                |r| r.get(0),
            )
            .optional()?)
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
                "SELECT user_id FROM tokens WHERE hash = ?1
                 AND hash NOT IN (SELECT hash FROM token_blacklist)",
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

    /// Claim pre-multi-user projects for the first team.
    pub fn assign_orphan_apps(&self, team_id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE apps SET team_id = ?1, updated_at = ?2 WHERE team_id IS NULL",
            params![team_id, now()],
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

    #[allow(dead_code)]
    pub fn revoke_token(&self, hash: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO token_blacklist (hash, created_at) VALUES (?1, ?2)",
            params![hash, now()],
        )?;
        Ok(())
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

    /// Atomically claim the single first-run slot. Exactly one caller wins.
    pub fn claim_first_user(&self) -> Result<bool> {
        Ok(self.conn().execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES ('first_user_claimed', '1')",
            [],
        )? == 1)
    }

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
    fn invites_are_single_use_and_expire() {
        let (db, path) = tmp_db();
        let u = db.create_user("i@example.com", "Invitee").unwrap();
        let t = db.create_team("Team", "team", &u.id).unwrap();
        let token = db.create_invite(&t.id, "member", 60, None).unwrap();
        assert!(db.use_invite(&token, &u.id).unwrap().is_some());
        assert!(db.use_invite(&token, &u.id).unwrap().is_none());
        let expired = db.create_invite(&t.id, "member", -1, None).unwrap();
        assert!(db.use_invite(&expired, &u.id).unwrap().is_none());
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn preview_retention_keeps_production() {
        let (db, path) = tmp_db();
        let t = db.create_user("r@example.com", "R").unwrap();
        let _ = t;
        for i in 0..12 {
            let id = format!("preview-{i:02}");
            db.add_deploy(&id, "app", "ok", Some("/tmp/p"), "preview", None)
                .unwrap();
        }
        for i in 0..3 {
            let id = format!("production-deploy-{i}");
            db.add_deploy(&id, "app", "ok", Some("/tmp/d"), "production", None)
                .unwrap();
            db.set_production("app", &id).unwrap();
        }
        let removed = db.prune_deploys_env("app", "preview", 10).unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(db.deploys_env("app", Some("production"), 100).unwrap().len(), 3);
        let all = db.deploys("app", 100).unwrap();
        assert!(all.iter().any(|d| d.id == "production-deploy-2"));
        assert!(db.deploy("app", "production-deploy-2").unwrap().is_some());
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn first_user_claim_is_atomic() {
        let (db, path) = tmp_db();
        assert!(db.claim_first_user().unwrap());
        assert!(!db.claim_first_user().unwrap());
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn migrates_legacy_database() {
        let path = std::env::temp_dir().join(format!("dok-legacy-{}.db", uuid::Uuid::new_v4()));
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE apps (name TEXT PRIMARY KEY, created_at INTEGER NOT NULL,
                   kind TEXT NOT NULL DEFAULT 'docker', host_port INTEGER, container_port INTEGER,
                   domain TEXT, status TEXT NOT NULL DEFAULT 'new', dir TEXT, repo TEXT,
                   updated_at INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE env (app TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
                   PRIMARY KEY (app, key));
                 CREATE TABLE deploys (id TEXT PRIMARY KEY, app TEXT NOT NULL,
                   created_at INTEGER NOT NULL, status TEXT NOT NULL, log TEXT NOT NULL DEFAULT '');
                 CREATE TABLE tokens (hash TEXT PRIMARY KEY, name TEXT NOT NULL, created_at INTEGER NOT NULL);
                 CREATE TABLE sessions (hash TEXT PRIMARY KEY, csrf TEXT NOT NULL,
                   created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);
                 INSERT INTO apps (name, created_at, domain) VALUES ('old', 1, 'same.test');
                 INSERT INTO apps (name, created_at, domain) VALUES ('old2', 2, 'same.test');
                 INSERT INTO env (app, key, value) VALUES ('old', 'A', '1');",
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        let app = db.get_app("old").unwrap().unwrap();
        assert_eq!(app.team_id, None);
        assert_eq!(
            db.env_resolved("old", "production").unwrap(),
            vec![("A".into(), "1".into())]
        );
        let version: i64 = db
            .conn()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 4);
        let dupes: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM apps WHERE domain = 'same.test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dupes, 1);
        drop(db);
        let _ = std::fs::remove_file(path);
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
