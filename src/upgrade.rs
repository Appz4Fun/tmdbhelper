//! Copies an older `ItemDetails.db` into a new file with the current schema.
//!
//! The add-on never alters a table: when its schema version rises, it drops the
//! tables that changed and recreates them empty. Version 54 drops every item
//! table, so a version 51 file opened by TMDBHelper 6.21 loses its whole cache.
//! This builds a file at the current version instead, so the add-on keeps it.

use crate::cache::{now_unix, DEFAULT_EXPIRY, DELETION_GRACE, SCHEMA_VERSION};
use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const SCHEMA: &str = include_str!("../schema/ItemDetails.sql");

/// Versions whose tables this copy understands. Version 51 kept the version 50
/// tables.
const SOURCE_VERSIONS: &[i64] = &[50, 51];

/// Tables whose layout changed so much that old rows can't be carried over.
/// The add-on drops them on upgrade too, and refills them as it needs to.
const DROPPED_TABLES: &[&str] = &["ratings"];

/// What a finished upgrade copied.
#[derive(Debug, Default)]
pub struct Summary {
    pub tables: usize,
    pub rows: u64,
}

/// Builds `target` from `source`. `source` is only read. The new file is
/// written next to `target` under a temporary name and renamed when it's
/// complete, so a failed run leaves nothing at `target`.
///
/// Every item gets a deletion lease of at least `DEFAULT_EXPIRY` plus the grace
/// period from now. The add-on gives a row the longest lease of any item that
/// relies on it; working that out for 14 million rows isn't worth it when a
/// floor keeps every row alive until warmup-rs or the add-on refreshes it.
pub fn run(source: &Path, target: &Path) -> Result<Summary> {
    if target.exists() {
        bail!("{} already exists; move it away first", target.display());
    }
    let version = source_version(source)?;
    let partial = partial_path(target);
    if partial.exists() {
        std::fs::remove_file(&partial)
            .with_context(|| format!("remove the leftover {}", partial.display()))?;
    }
    info!(
        "upgrading {} (version {version}) into {} (version {SCHEMA_VERSION})",
        source.display(),
        target.display()
    );
    let summary = build(source, &partial).inspect_err(|_| {
        let _ = std::fs::remove_file(&partial);
    })?;
    std::fs::rename(&partial, target)
        .with_context(|| format!("rename {} to {}", partial.display(), target.display()))?;
    info!(
        "upgrade complete: {} tables, {} rows in {}",
        summary.tables,
        summary.rows,
        target.display()
    );
    Ok(summary)
}

fn source_version(source: &Path) -> Result<i64> {
    let conn = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open {}", source.display()))?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version == SCHEMA_VERSION {
        bail!(
            "{} is already at version {SCHEMA_VERSION}",
            source.display()
        );
    }
    if !SOURCE_VERSIONS.contains(&version) {
        bail!(
            "{} has version {version}; this build upgrades versions {SOURCE_VERSIONS:?}",
            source.display()
        );
    }
    Ok(version)
}

fn partial_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    target.with_file_name(name)
}

fn build(source: &Path, path: &Path) -> Result<Summary> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("create {}", path.display()))?;
    // A crash leaves a partial file that the next run deletes, so the copy can
    // skip the journal.
    conn.execute_batch(
        "PRAGMA journal_mode=OFF;
         PRAGMA synchronous=OFF;
         PRAGMA foreign_keys=OFF;
         PRAGMA cache_size=-1048576;",
    )?;
    let (tables, indexes): (Vec<&str>, Vec<&str>) = SCHEMA
        .lines()
        .filter(|l| !l.trim().is_empty())
        .partition(|l| !l.starts_with("CREATE INDEX"));
    // Indexes are built after the rows are in, which is far faster than
    // updating them row by row.
    conn.execute_batch(&tables.join("\n"))
        .context("apply the bundled schema")?;
    conn.execute(
        "ATTACH DATABASE ?1 AS old",
        [format!("file:{}?mode=ro", uri_path(source))],
    )
    .with_context(|| format!("attach {}", source.display()))?;

    let names: Vec<String> = conn
        .prepare("SELECT name FROM main.sqlite_master WHERE type='table' ORDER BY name")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let floor = now_unix() + DEFAULT_EXPIRY;
    let mut summary = Summary::default();
    conn.execute_batch("BEGIN")?;
    for name in &names {
        let old_columns = columns(&conn, "old", name)?;
        if old_columns.is_empty() {
            info!("{name}: new table, left empty");
            continue;
        }
        if DROPPED_TABLES.contains(&name.as_str()) {
            let count: i64 =
                conn.query_row(&format!("SELECT COUNT(*) FROM old.\"{name}\""), [], |r| {
                    r.get(0)
                })?;
            if count > 0 {
                warn!("{name}: the layout changed, so its {count} old rows are dropped");
            }
            continue;
        }
        let new_columns = columns(&conn, "main", name)?;
        let shared: Vec<&String> = new_columns
            .iter()
            .filter(|c| old_columns.contains(c))
            .collect();
        let list = shared
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        // The old file's lookup rows replace the schema's seed rows.
        conn.execute(&format!("DELETE FROM main.\"{name}\""), [])?;
        let rows = if name == "baseitem" && new_columns.iter().any(|c| c == "deletion") {
            conn.execute(
                &format!(
                    "INSERT INTO main.baseitem ({list}, deletion)
                     SELECT {list}, MAX(COALESCE(expiry, 0), ?1) + ?2 FROM old.baseitem"
                ),
                [floor, DELETION_GRACE],
            )?
        } else {
            conn.execute(
                &format!("INSERT INTO main.\"{name}\" ({list}) SELECT {list} FROM old.\"{name}\""),
                [],
            )?
        };
        info!("{name}: copied {rows} rows");
        summary.tables += 1;
        summary.rows += rows as u64;
    }
    conn.execute_batch("COMMIT")?;
    conn.execute_batch("DETACH DATABASE old")?;

    for (i, index) in indexes.iter().enumerate() {
        info!("index {}/{}: {index}", i + 1, indexes.len());
        conn.execute_batch(index)?;
    }
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != SCHEMA_VERSION {
        bail!("the bundled schema set version {version}, expected {SCHEMA_VERSION}");
    }
    let check: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if check != "ok" {
        bail!("the integrity check on the new file failed: {check}");
    }
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    Ok(summary)
}

fn columns(conn: &Connection, schema: &str, table: &str) -> Result<Vec<String>> {
    Ok(conn
        .prepare("SELECT name FROM pragma_table_info(?1, ?2)")?
        .query_map([table, schema], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// Escapes a path for a `file:` URI.
fn uri_path(path: &Path) -> String {
    let mut out = String::new();
    for c in path.to_string_lossy().chars() {
        match c {
            '%' => out.push_str("%25"),
            '?' => out.push_str("%3f"),
            '#' => out.push_str("%23"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A version 51 file: the bundled schema with the version 51 tables put back.
    fn v51(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(
            "DROP TABLE baseitem;
             CREATE TABLE baseitem(id TEXT PRIMARY KEY, mediatype TEXT, expiry INTEGER, datalevel INTEGER DEFAULT 0 NOT NULL, fanart_tv INTEGER DEFAULT 0 NOT NULL, translation INTEGER DEFAULT 0 NOT NULL, language TEXT);
             DROP TABLE ratings; DROP TABLE ratings_provider;
             DROP TABLE stored_metadata; DROP TABLE stored_metadata_state;
             CREATE TABLE ratings(id TEXT PRIMARY KEY, tmdb_rating INTEGER, expiry INTEGER);
             INSERT INTO baseitem VALUES ('movie.550', 'movie', 4102444800, 5, 0, 1, 'en-US');
             INSERT INTO baseitem VALUES ('person.287', 'person', 0, 0, 0, 0, 'en-US');
             INSERT INTO movie (id, tmdb_id, title) VALUES ('movie.550', 550, 'Fight Club');
             INSERT INTO ratings VALUES ('movie.550', 8, 0);
             INSERT INTO countries VALUES ('Atlantis', 'XA');
             PRAGMA user_version=51;",
        )
        .unwrap();
    }

    #[test]
    fn copies_a_version_51_file_into_the_current_schema() {
        let dir = tempfile::tempdir().unwrap();
        let (source, target) = (dir.path().join("old.db"), dir.path().join("new.db"));
        v51(&source);
        let summary = run(&source, &target).unwrap();
        assert!(summary.rows >= 3);
        assert!(!partial_path(&target).exists());

        let conn = Connection::open(&target).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let deletion = |id: &str| -> i64 {
            conn.query_row("SELECT deletion FROM baseitem WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .unwrap()
        };
        assert_eq!(deletion("movie.550"), 4102444800 + DELETION_GRACE);
        assert!(deletion("person.287") >= now_unix() + DEFAULT_EXPIRY);
        let title: String = conn
            .query_row("SELECT title FROM movie WHERE id='movie.550'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(title, "Fight Club");
        let ratings: i64 = conn
            .query_row("SELECT COUNT(*) FROM ratings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ratings, 0);
        let atlantis: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM countries WHERE iso_country='XA'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(atlantis, 1);
        // The schema must match what a fresh file gets, index for index.
        let fresh = Connection::open_in_memory().unwrap();
        fresh.execute_batch(SCHEMA).unwrap();
        let shape = |c: &Connection| -> Vec<(String, String)> {
            c.prepare("SELECT name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert_eq!(shape(&conn), shape(&fresh));
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn refuses_to_overwrite_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let (source, target) = (dir.path().join("old.db"), dir.path().join("new.db"));
        v51(&source);
        std::fs::write(&target, b"keep me").unwrap();
        assert!(run(&source, &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep me");
    }

    #[test]
    fn refuses_a_file_already_at_the_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("old.db");
        Connection::open(&source)
            .unwrap()
            .execute_batch(SCHEMA)
            .unwrap();
        assert!(run(&source, &dir.path().join("new.db")).is_err());
    }
}
