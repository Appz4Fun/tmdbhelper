pub mod mapper;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const DATALEVEL_FULL: i64 = 5;
pub const DAY: i64 = 86400;
pub const DEFAULT_EXPIRY: i64 = 30 * DAY;
pub const SHORTER_EXPIRY: i64 = 7 * DAY;
/// The `ItemDetails.db` version from the add-on's `database.py`. The add-on drops
/// every table in a database with an older non-zero version, so this must match.
pub const SCHEMA_VERSION: i64 = 51;
/// Older versions this build upgrades in place. Version 51 kept the version 50
/// tables, so only the version number changes.
const UPGRADABLE_VERSIONS: &[i64] = &[50];

/// What every writer needs besides the TMDB response.
pub struct WriteContext {
    /// The add-on's language setting. It only reads rows with the same language.
    pub language: String,
    pub genres: HashMap<i64, String>,
}

pub fn open_writer(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .with_context(|| format!("open {}", path.display()))?;
    // busy_timeout=30s tolerates Kodi's periodic database vacuum cycles
    // (observed up to 60s on the live box). The writers create parents before
    // children, and leaving foreign keys off stops a refresh cascading into them.
    durable_wal(&conn)?;
    conn.execute_batch(
        "PRAGMA busy_timeout=120000;
         PRAGMA foreign_keys=OFF;
         PRAGMA cache_size=-262144;
         PRAGMA mmap_size=268435456;
         PRAGMA wal_autocheckpoint=1000;",
    )?;
    // The writers use about 60 statements per item, and SQLite recompiles any
    // that fall out of the cache, which is the slowest part of a write.
    conn.set_prepared_statement_cache_capacity(256);
    let mut version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if UPGRADABLE_VERSIONS.contains(&version) {
        conn.execute_batch(&format!("PRAGMA user_version={SCHEMA_VERSION};"))?;
        version = SCHEMA_VERSION;
    }
    if version != SCHEMA_VERSION {
        anyhow::bail!(
            "{} has schema version {version}, but this build writes version {SCHEMA_VERSION}. Delete it and state.db to rebuild them",
            path.display()
        );
    }
    Ok(conn)
}

pub fn checkpoint_passive(conn: &Connection) {
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// WAL with `synchronous=NORMAL`: a crash or forced restart can lose the last
/// few transactions, but it can't corrupt the database, which
/// `synchronous=OFF` could. On macOS a plain fsync leaves data in the drive's
/// cache, so every sync uses `F_FULLFSYNC` there. Other systems ignore both
/// `fullfsync` settings.
pub fn durable_wal(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA fullfsync=ON;
         PRAGMA checkpoint_fullfsync=ON;",
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn open_writer_upgrades_a_version_50_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ItemDetails.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../../schema/ItemDetails.sql"))
            .unwrap();
        conn.execute_batch("PRAGMA user_version=50;").unwrap();
        drop(conn);
        let conn = super::open_writer(&path).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, super::SCHEMA_VERSION);
    }

    #[test]
    fn open_writer_refuses_an_unknown_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ItemDetails.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA user_version=49;").unwrap();
        drop(conn);
        assert!(super::open_writer(&path).is_err());
    }

    #[test]
    fn durable_wal_never_runs_with_synchronous_off() {
        let dir = tempfile::tempdir().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("t.db")).unwrap();
        super::durable_wal(&conn).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        let sync: i64 = conn
            .query_row("PRAGMA synchronous", [], |r| r.get(0))
            .unwrap();
        let checkpoint_fullfsync: i64 = conn
            .query_row("PRAGMA checkpoint_fullfsync", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(sync, 1, "synchronous should be NORMAL");
        assert_eq!(checkpoint_fullfsync, 1);
    }
}
