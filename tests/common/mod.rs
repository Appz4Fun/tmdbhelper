#![allow(dead_code)]
use rusqlite::Connection;
use std::path::PathBuf;
use warmup_rs::cache::open_writer;

/// Path to a fresh scratch ItemDetails.db.
/// To regenerate the fixture: run `scripts/addon-schema.py`, then
/// `sqlite3 tests/fixtures/ItemDetails-empty.db < schema/ItemDetails.sql`.
pub fn scratch_db() -> (Connection, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("scratch.db");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ItemDetails-empty.db");
    std::fs::copy(&fixture, &path).expect("copy fixture");
    let conn = open_writer(&path).expect("open scratch via cache::open_writer");
    // Foreign keys are checked on the way out, because the writers run without them.
    // Leak the dir so tests can inspect; tempdir auto-cleans on drop.
    std::mem::forget(dir);
    (conn, path)
}
