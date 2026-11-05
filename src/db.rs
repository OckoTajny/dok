use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub struct Deploy {
    pub id: String,
    pub created_at: i64,
    pub status: String,
    pub dir: Option<String>,
}

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
    pub updated_at: i64,
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
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS deploys (
  id TEXT PRIMARY KEY,
  app TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  status TEXT NOT NULL,
  log TEXT NOT NULL DEFAULT '',
  dir TEXT
);
CREATE TABLE IF NOT EXISTS sessions (
  hash TEXT PRIMARY KEY,
  csrf TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
"#;

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        // Additive migrations for older databases.
        for sql in [
            "ALTER TABLE apps ADD COLUMN image TEXT",
            "ALTER TABLE apps ADD COLUMN memory_mb INTEGER",
            "ALTER TABLE apps ADD COLUMN cpus REAL",
            "ALTER TABLE deploys ADD COLUMN dir TEXT",
        ] {
            let _ = conn.execute(sql, []);
        }
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.0.lock().expect("db mutex poisoned")
    }

    pub fn list_apps(&self) -> Result<Vec<App>> {
        let c = self.conn();
        let mut stmt = c.prepare(
            "SELECT name, kind, image, memory_mb, cpus, host_port, container_port, domain, status, dir, repo, updated_at
             FROM apps ORDER BY name",
        )?;
        let rows = stmt.query_map([], |r| {
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
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_app(&self, name: &str) -> Result<Option<App>> {
        let c = self.conn();
        let app = c
            .query_row(
                "SELECT name, kind, image, memory_mb, cpus, host_port, container_port, domain, status, dir, repo, updated_at
                 FROM apps WHERE name = ?1",
                params![name],
                |r| {
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
                    })
                },
            )
            .optional()?;
        Ok(app)
    }

    pub fn upsert_app(&self, app: &App) -> Result<()> {
        let c = self.conn();
        c.execute(
            "INSERT INTO apps (name, created_at, kind, image, memory_mb, cpus, host_port, container_port, domain, status, dir, repo, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(name) DO UPDATE SET
               kind=excluded.kind, image=COALESCE(excluded.image, apps.image),
               memory_mb=COALESCE(excluded.memory_mb, apps.memory_mb),
               cpus=COALESCE(excluded.cpus, apps.cpus),
               host_port=excluded.host_port, container_port=excluded.container_port,
               domain=COALESCE(excluded.domain, apps.domain), status=excluded.status,
               dir=excluded.dir, repo=COALESCE(excluded.repo, apps.repo), updated_at=excluded.updated_at",
            params![
                app.name, now(), app.kind, app.image, app.memory_mb, app.cpus,
                app.host_port, app.container_port, app.domain, app.status, app.dir,
                app.repo, app.updated_at
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
        c.execute("DELETE FROM env WHERE app = ?1", params![name])?;
        c.execute("DELETE FROM deploys WHERE app = ?1", params![name])?;
        c.execute("DELETE FROM apps WHERE name = ?1", params![name])?;
        Ok(())
    }

    pub fn env(&self, app: &str) -> Result<Vec<(String, String)>> {
        let c = self.conn();
        let mut stmt = c.prepare("SELECT key, value FROM env WHERE app = ?1 ORDER BY key")?;
        let rows = stmt.query_map(params![app], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn put_env(&self, app: &str, kv: &[(String, String)]) -> Result<()> {
        let mut c = self.conn();
        let tx = c.transaction()?;
        for (k, v) in kv {
            tx.execute(
                "INSERT INTO env (app, key, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT(app, key) DO UPDATE SET value = excluded.value",
                params![app, k, v],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn add_deploy(&self, id: &str, app: &str, status: &str, dir: Option<&str>) -> Result<()> {
        self.conn().execute(
            "INSERT INTO deploys (id, app, created_at, status, dir) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, app, now(), status, dir],
        )?;
        Ok(())
    }

    pub fn deploys(&self, app: &str, limit: usize) -> Result<Vec<Deploy>> {
        let c = self.conn();
        let mut stmt = c.prepare(
            "SELECT id, created_at, status, dir FROM deploys
             WHERE app = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![app, limit as i64], |r| {
            Ok(Deploy {
                id: r.get(0)?,
                created_at: r.get(1)?,
                status: r.get(2)?,
                dir: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn deploy_dir_for(&self, app: &str, deploy: &str) -> Result<Option<String>> {
        let c = self.conn();
        let dir = c
            .query_row(
                "SELECT dir FROM deploys WHERE id = ?1 AND app = ?2",
                params![deploy, app],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(dir.flatten())
    }

    pub fn prune_deploys(&self, app: &str, keep: usize) -> Result<()> {
        self.conn().execute(
            "DELETE FROM deploys WHERE app = ?1 AND id NOT IN (
               SELECT id FROM deploys WHERE app = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2)",
            params![app, keep as i64],
        )?;
        Ok(())
    }

    /// Consistent snapshot via `VACUUM INTO`.
    pub fn backup_to(&self, path: &std::path::Path) -> Result<()> {
        self.conn()
            .execute("VACUUM INTO ?1", params![path.to_string_lossy()])?;
        Ok(())
    }

    pub fn create_session(&self, hash: &str, csrf: &str, ttl: i64) -> Result<()> {
        let c = self.conn();
        let now = now();
        c.execute("DELETE FROM sessions WHERE expires_at <= ?1", params![now])?;
        c.execute(
            "INSERT INTO sessions (hash, csrf, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
            params![hash, csrf, now, now + ttl],
        )?;
        Ok(())
    }

    pub fn session(&self, hash: &str) -> Result<Option<(String, i64)>> {
        let c = self.conn();
        let row = c
            .query_row(
                "SELECT csrf, expires_at FROM sessions WHERE hash = ?1",
                params![hash],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?;
        Ok(row)
    }

    pub fn delete_session(&self, hash: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM sessions WHERE hash = ?1", params![hash])?;
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
        let c = self.conn();
        let log = c
            .query_row(
                "SELECT log FROM deploys WHERE app = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![app],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(log)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let c = self.conn();
        let v = c
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(v)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn count_tokens(&self) -> Result<i64> {
        let c = self.conn();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM tokens", [], |r| r.get(0))?;
        Ok(n)
    }

    pub fn add_token(&self, hash: &str, name: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO tokens (hash, name, created_at) VALUES (?1, ?2, ?3)",
            params![hash, name, now()],
        )?;
        Ok(())
    }

    pub fn token_exists(&self, hash: &str) -> Result<bool> {
        let c = self.conn();
        let n: i64 = c.query_row(
            "SELECT COUNT(*) FROM tokens WHERE hash = ?1",
            params![hash],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn domains(&self) -> Result<Vec<(String, Option<String>)>> {
        let c = self.conn();
        let mut stmt =
            c.prepare("SELECT name, domain FROM apps WHERE domain IS NOT NULL ORDER BY name")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn app_by_domain(&self, host: &str) -> Result<Option<App>> {
        let c = self.conn();
        let app = c
            .query_row(
                "SELECT name, kind, image, memory_mb, cpus, host_port, container_port, domain, status, dir, repo, updated_at
                 FROM apps WHERE domain = ?1",
                params![host],
                |r| {
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
                    })
                },
            )
            .optional()?;
        Ok(app)
    }
}
