//! SQLite storage. One writer connection behind a mutex; a small read pool for
//! the web layer. WAL mode, `synchronous=NORMAL`, a busy timeout, and a tuned
//! page cache.

pub mod hands;
pub mod queries;
pub mod schema;

use crate::domain::{HeroProfile, Player};
use crate::error::StorageError;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub const SETTING_ACTIVE_PROFILE: &str = "active_hero_profile";
pub const SETTING_SESSION_GAP_SECS: &str = "session_gap_secs";
pub const SETTING_CURRENCY_CONFIRMED: &str = "currency_confirmed";
pub const SETTING_HERO_NAME: &str = "hero_name";

/// The process-wide database handle.
#[derive(Clone)]
pub struct Db {
    /// Single writer. All mutations take this lock.
    writer: Arc<Mutex<Connection>>,
    /// Read pool for the web layer.
    pool: r2d2::Pool<SqliteConnectionManager>,
}

impl Db {
    /// Open (creating if needed) the database at `path`, apply migrations, and
    /// set pragmas.
    pub fn open(path: &Path) -> Result<Db, StorageError> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI,
        )?;
        apply_pragmas(&conn)?;
        migrate(&mut conn)?;

        let manager = SqliteConnectionManager::file(path).with_init(|c| {
            c.execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=10000;\n\
                 PRAGMA query_only=ON; PRAGMA cache_size=-65536;",
            )
        });
        let pool = r2d2::Pool::builder().max_size(8).build(manager)?;
        Ok(Db { writer: Arc::new(Mutex::new(conn)), pool })
    }

    /// Open a fresh temporary file-backed database (tests only). A file is used
    /// rather than `:memory:` so the writer and the read pool see the same data.
    pub fn open_temp() -> Result<(Db, tempdir::TempDir), StorageError> {
        let dir = tempdir::TempDir::new("texture-test").map_err(|e| {
            StorageError::NotFound(format!("could not create temp dir: {e}"))
        })?;
        let db = Db::open(&dir.path().join("test.sqlite"))?;
        Ok((db, dir))
    }

    /// Run `f` with the writer connection held.
    pub fn with_writer<T>(&self, f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T, StorageError> {
        let mut guard = self.writer.lock().expect("writer mutex poisoned");
        Ok(f(&mut guard)?)
    }

    /// Run `f` with a pooled read connection.
    pub fn with_reader<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, StorageError> {
        let conn = self.pool.get()?;
        Ok(f(&conn)?)
    }

    // ---- settings -------------------------------------------------------

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        self.with_reader(|c| {
            c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()
        })
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StorageError> {
        self.with_writer(|c| {
            c.execute(
                "INSERT INTO settings(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![key, value],
            )
        })?;
        Ok(())
    }

    pub fn session_gap_secs(&self) -> Result<i64, StorageError> {
        Ok(self
            .get_setting(SETTING_SESSION_GAP_SECS)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(30 * 60))
    }

    // ---- identity -------------------------------------------------------

    pub fn upsert_player(&self, screen_name: &str) -> Result<i64, StorageError> {
        self.with_writer(|c| {
            c.execute("INSERT INTO players(screen_name) VALUES (?1) ON CONFLICT(screen_name) DO NOTHING", [screen_name])?;
            c.query_row("SELECT id FROM players WHERE screen_name = ?1", [screen_name], |r| r.get(0))
        })
    }

    pub fn list_players(&self) -> Result<Vec<Player>, StorageError> {
        self.with_reader(|c| {
            let mut stmt = c.prepare("SELECT id, screen_name FROM players ORDER BY screen_name")?;
            let rows = stmt
                .query_map([], |r| Ok(Player { id: r.get(0)?, screen_name: r.get(1)? }))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn create_profile(&self, name: &str) -> Result<i64, StorageError> {
        self.with_writer(|c| {
            c.execute("INSERT INTO hero_profiles(name) VALUES (?1) ON CONFLICT(name) DO NOTHING", [name])?;
            c.query_row("SELECT id FROM hero_profiles WHERE name = ?1", [name], |r| r.get(0))
        })
    }

    pub fn add_profile_player(&self, profile_id: i64, player_id: i64) -> Result<(), StorageError> {
        self.with_writer(|c| {
            c.execute(
                "INSERT INTO hero_profile_players(profile_id, player_id) VALUES (?1, ?2)
                 ON CONFLICT DO NOTHING",
                rusqlite::params![profile_id, player_id],
            )
        })?;
        Ok(())
    }

    pub fn set_active_profile(&self, profile_id: i64) -> Result<(), StorageError> {
        self.with_writer(|c| {
            c.execute("UPDATE hero_profiles SET active = 0", [])?;
            c.execute("UPDATE hero_profiles SET active = 1 WHERE id = ?1", [profile_id])
        })?;
        self.set_setting(SETTING_ACTIVE_PROFILE, &profile_id.to_string())
    }

    pub fn active_profile(&self) -> Result<Option<HeroProfile>, StorageError> {
        let id: Option<i64> = self
            .get_setting(SETTING_ACTIVE_PROFILE)?
            .and_then(|v| v.parse().ok());
        let Some(id) = id else { return Ok(None) };
        self.profile(id).map(Some)
    }

    pub fn profile(&self, id: i64) -> Result<HeroProfile, StorageError> {
        self.with_reader(|c| {
            let name: String = c.query_row("SELECT name FROM hero_profiles WHERE id = ?1", [id], |r| r.get(0))?;
            let mut stmt = c.prepare("SELECT player_id FROM hero_profile_players WHERE profile_id = ?1")?;
            let ids = stmt.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
            Ok(HeroProfile { id, name, player_ids: ids })
        })
    }

    pub fn list_profiles(&self) -> Result<Vec<HeroProfile>, StorageError> {
        let ids: Vec<i64> = self.with_reader(|c| {
            let mut stmt = c.prepare("SELECT id FROM hero_profiles ORDER BY name")?;
            let ids = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
            Ok(ids)
        })?;
        ids.into_iter().map(|id| self.profile(id)).collect()
    }

    /// Resolve the hero profile for `hero_name`, creating the player, the profile
    /// and the alias, and making it active if no profile is active yet. Called on
    /// first run so the operator never has to type the identity their stats
    /// depend on.
    pub fn ensure_hero_profile(&self, hero_name: &str) -> Result<(i64, i64), StorageError> {
        let player_id = self.upsert_player(hero_name)?;
        let profile_id = match self.active_profile()? {
            Some(p) => p.id,
            None => self.create_profile("default")?,
        };
        self.add_profile_player(profile_id, player_id)?;
        if self.active_profile()?.is_none() {
            self.set_active_profile(profile_id)?;
        }
        Ok((profile_id, player_id))
    }

    /// The set of player ids that constitute the active hero profile.
    pub fn active_player_ids(&self) -> Result<(i64, Vec<i64>), StorageError> {
        match self.active_profile()? {
            Some(p) => Ok((p.id, p.player_ids)),
            None => Err(StorageError::NotFound("no active hero profile".into())),
        }
    }

    /// `(screen_name, player_id)` for every player in a hero profile. The parser
    /// deliberately does not know identities, so the import pipeline resolves a
    /// hand's hero seat through this mapping.
    pub fn players_for_profile(&self, profile_id: i64) -> Result<Vec<(String, i64)>, StorageError> {
        self.with_reader(|c| {
            let mut stmt = c.prepare(
                "SELECT p.screen_name, p.id
                 FROM players p
                 JOIN hero_profile_players hpp ON hpp.player_id = p.id
                 WHERE hpp.profile_id = ?1
                 ORDER BY p.screen_name",
            )?;
            let rows = stmt
                .query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }
}

/// Apply the SQLite pragmas every connection needs.
pub fn apply_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;
         PRAGMA busy_timeout=10000;
         PRAGMA cache_size=-65536;
         PRAGMA temp_store=MEMORY;
         PRAGMA mmap_size=268435456;",
    )
}

/// Apply all migrations not yet recorded in `meta`.
pub fn migrate(conn: &mut Connection) -> Result<(), StorageError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);")?;
    let current: u32 = conn
        .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get::<_, String>(0))
        .optional()?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    for (i, sql) in schema::MIGRATIONS.iter().enumerate() {
        let version = (i + 1) as u32;
        if version <= current {
            continue;
        }
        let tx = conn.transaction().map_err(|e| StorageError::Migration { version, source: e })?;
        tx.execute_batch(sql).map_err(|e| StorageError::Migration { version, source: e })?;
        tx.execute(
            "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [version.to_string()],
        )
        .map_err(|e| StorageError::Migration { version, source: e })?;
        tx.commit().map_err(|e| StorageError::Migration { version, source: e })?;
    }
    Ok(())
}

fn uuid_like() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

/// A minimal temp-dir helper so tests do not need an extra dependency at the
/// library boundary.
pub mod tempdir {
    use std::path::{Path, PathBuf};

    pub struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        pub fn new(prefix: &str) -> std::io::Result<TempDir> {
            let path = std::env::temp_dir().join(format!("{prefix}-{}-{}", std::process::id(), super::uuid_like()));
            std::fs::create_dir_all(&path)?;
            Ok(TempDir { path })
        }
        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
