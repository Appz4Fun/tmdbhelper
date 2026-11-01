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
pub const SCHEMA_VERSION: i64 = 50;

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
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=OFF;
         PRAGMA busy_timeout=120000;
         PRAGMA foreign_keys=OFF;
         PRAGMA cache_size=-262144;
         PRAGMA mmap_size=268435456;
         PRAGMA wal_autocheckpoint=1000;",
    )?;
    // The writers use about 60 statements per item, and SQLite recompiles any
    // that fall out of the cache, which is the slowest part of a write.
    conn.set_prepared_statement_cache_capacity(256);
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
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
