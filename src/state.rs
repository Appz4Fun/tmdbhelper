use crate::id::TmdbType;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags};
use std::path::Path;

/// One child of an item: (TMDB ID, type, popularity, queue depth).
pub type Child = (i64, TmdbType, f64, i64);
pub type ChildBatch<'a> = (i64, TmdbType, &'a [Child]);

/// A failing item is dropped after this many attempts, until its refresh comes round.
pub const MAX_ATTEMPTS: i64 = 5;

pub struct StateDb {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct QueueItem {
    pub tmdb_id: i64,
    pub tmdb_type: TmdbType,
    pub depth: i64,
    pub popularity: f64,
}

fn type_to_str(t: TmdbType) -> &'static str {
    match t {
        TmdbType::Movie => "movie",
        TmdbType::Tv => "tv",
        TmdbType::Season => "season",
        TmdbType::Episode => "episode",
        TmdbType::Person => "person",
        TmdbType::Collection => "collection",
    }
}

fn type_from_str(s: &str) -> Option<TmdbType> {
    Some(match s {
        "movie" => TmdbType::Movie,
        "tv" => TmdbType::Tv,
        "season" => TmdbType::Season,
        "episode" => TmdbType::Episode,
        "person" => TmdbType::Person,
        "collection" | "set" => TmdbType::Collection,
        _ => return None,
    })
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

const ENQUEUE: &str = "INSERT INTO queue (tmdb_id, tmdb_type, depth, popularity, enqueued_at)
     SELECT ?1, ?2, ?3, ?4, ?5
     WHERE NOT EXISTS (SELECT 1 FROM visited WHERE tmdb_id=?1 AND tmdb_type=?2)
     ON CONFLICT(tmdb_id, tmdb_type) DO UPDATE SET
         depth=MIN(depth, excluded.depth), popularity=MAX(popularity, excluded.popularity)";

impl StateDb {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .with_context(|| format!("open state {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=OFF;
             PRAGMA busy_timeout=120000;
             PRAGMA cache_size=-65536;",
        )?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS queue (
                 tmdb_id INTEGER, tmdb_type TEXT, depth INTEGER,
                 popularity REAL DEFAULT 0.0, enqueued_at REAL,
                 PRIMARY KEY (tmdb_id, tmdb_type));
             CREATE TABLE IF NOT EXISTS visited (
                 tmdb_id INTEGER, tmdb_type TEXT, visited_at REAL,
                 result TEXT NOT NULL DEFAULT 'ok',
                 PRIMARY KEY (tmdb_id, tmdb_type));
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);",
        )?;
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('queue')")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if !columns.iter().any(|c| c == "claimed") {
            conn.execute_batch(
                "ALTER TABLE queue ADD COLUMN claimed INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE queue ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_queue_next
                 ON queue(claimed, depth ASC, popularity DESC, enqueued_at ASC);",
        )?;
        Ok(Self { conn })
    }

    /// Returns the items a stopped run had in flight to the queue.
    pub fn release_claims(&self) -> Result<usize> {
        Ok(self
            .conn
            .execute("UPDATE queue SET claimed=0 WHERE claimed=1", [])?)
    }

    /// Claims a batch of pending items, ordered by (depth ASC, popularity DESC).
    /// They stay in the queue until they're visited, so stopping the crawl loses
    /// nothing. Call `release_claims` at startup to pick them up again.
    pub fn pop_batch(&mut self, limit: usize) -> Result<Vec<QueueItem>> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut items = Vec::new();
        let mut raw_types = Vec::new();
        {
            let mut stmt = tx.prepare_cached(
                "SELECT tmdb_id, tmdb_type, depth, popularity FROM queue WHERE claimed=0
                 ORDER BY depth ASC, popularity DESC, enqueued_at ASC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], |r| {
                let type_s: String = r.get(1)?;
                Ok((
                    r.get::<_, i64>(0)?,
                    type_s,
                    r.get::<_, i64>(2)?,
                    r.get::<_, f64>(3).unwrap_or(0.0),
                ))
            })?;
            for row in rows {
                let (id, type_s, depth, pop) = row?;
                if let Some(t) = type_from_str(&type_s) {
                    items.push(QueueItem {
                        tmdb_id: id,
                        tmdb_type: t,
                        depth,
                        popularity: pop,
                    });
                    raw_types.push(type_s);
                }
            }
        }
        for (i, item) in items.iter().enumerate() {
            tx.prepare_cached("UPDATE queue SET claimed=1 WHERE tmdb_id=?1 AND tmdb_type=?2")?
                .execute(params![item.tmdb_id, &raw_types[i]])?;
        }
        tx.commit()?;
        Ok(items)
    }

    pub fn enqueue_child(
        &mut self,
        tmdb_id: i64,
        tmdb_type: TmdbType,
        depth: i64,
        popularity: f64,
    ) -> Result<()> {
        self.conn.prepare_cached(ENQUEUE)?.execute(params![
            tmdb_id,
            type_to_str(tmdb_type),
            depth,
            popularity,
            now()
        ])?;
        Ok(())
    }

    /// Enqueues many items in one transaction.
    pub fn enqueue_many(&mut self, items: &[Child]) -> Result<()> {
        let now = now();
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(ENQUEUE)?;
            for (id, t, pop, depth) in items {
                stmt.execute(params![*id, type_to_str(*t), *depth, *pop, now])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Mega-batch: visit + enqueue children for MULTIPLE items in ONE transaction.
    /// At 7/s with 20 items/batch, the old per-item tx pattern still costs 20 WAL
    /// commits per batch. This collapses them to 1.
    pub fn visit_and_enqueue_multi(&mut self, items: &[ChildBatch<'_>]) -> Result<()> {
        let now = now();
        let tx = self.conn.transaction()?;
        // Phase 1: mark ALL parents visited first, so children that overlap with
        // other parents in this batch won't be re-enqueued
        for (tmdb_id, tmdb_type, _children) in items {
            tx.prepare_cached(
                "INSERT OR REPLACE INTO visited (tmdb_id, tmdb_type, visited_at, result) VALUES (?1, ?2, ?3, 'ok')",
            )?
            .execute(params![*tmdb_id, type_to_str(*tmdb_type), now])?;
            tx.prepare_cached("DELETE FROM queue WHERE tmdb_id=?1 AND tmdb_type=?2")?
                .execute(params![*tmdb_id, type_to_str(*tmdb_type)])?;
        }
        // Phase 2: enqueue children, skipping those already visited
        for (_tmdb_id, _tmdb_type, children) in items {
            let mut stmt = tx.prepare_cached(ENQUEUE)?;
            for (cid, ctype, pop, depth) in *children {
                stmt.execute(params![*cid, type_to_str(*ctype), *depth, *pop, now])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Finishes an item that won't be fetched again until its refresh comes round.
    pub fn finish(&mut self, tmdb_id: i64, tmdb_type: TmdbType, result: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.prepare_cached(
            "INSERT OR REPLACE INTO visited (tmdb_id, tmdb_type, visited_at, result) VALUES (?1, ?2, ?3, ?4)",
        )?
        .execute(params![tmdb_id, type_to_str(tmdb_type), now(), result])?;
        tx.prepare_cached("DELETE FROM queue WHERE tmdb_id=?1 AND tmdb_type=?2")?
            .execute(params![tmdb_id, type_to_str(tmdb_type)])?;
        tx.commit()?;
        Ok(())
    }

    /// Puts an item back in the queue. An unavailable service doesn't count against it.
    pub fn release(&mut self, tmdb_id: i64, tmdb_type: TmdbType) -> Result<()> {
        self.conn
            .prepare_cached("UPDATE queue SET claimed=0 WHERE tmdb_id=?1 AND tmdb_type=?2")?
            .execute(params![tmdb_id, type_to_str(tmdb_type)])?;
        Ok(())
    }

    /// Counts a failed attempt, and finishes the item as failed at `MAX_ATTEMPTS`.
    pub fn record_failure(&mut self, tmdb_id: i64, tmdb_type: TmdbType) -> Result<()> {
        let attempts: i64 = self
            .conn
            .prepare_cached(
                "UPDATE queue SET claimed=0, attempts=attempts+1
                 WHERE tmdb_id=?1 AND tmdb_type=?2 RETURNING attempts",
            )?
            .query_row(params![tmdb_id, type_to_str(tmdb_type)], |r| r.get(0))?;
        if attempts >= MAX_ATTEMPTS {
            self.finish(tmdb_id, tmdb_type, "failed")?;
        }
        Ok(())
    }

    pub fn requeue_expired(&mut self, max_age_days: i64) -> Result<i64> {
        let now = now();
        let cutoff = now - (max_age_days as f64 * 86400.0);
        let tx = self.conn.transaction()?;
        let count = tx.execute(
            "INSERT OR IGNORE INTO queue (tmdb_id, tmdb_type, depth, popularity, enqueued_at)
             SELECT tmdb_id, tmdb_type, 2, 0.0, ?1 FROM visited WHERE visited_at < ?2",
            params![now, cutoff],
        )?;
        tx.execute("DELETE FROM visited WHERE visited_at < ?1", params![cutoff])?;
        tx.commit()?;
        Ok(count as i64)
    }

    pub fn queue_size(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM queue", [], |r| r.get(0))?)
    }

    pub fn claimed_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM queue WHERE claimed=1", [], |r| {
                r.get(0)
            })?)
    }

    pub fn visited_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM visited", [], |r| r.get(0))?)
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        match self
            .conn
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
        {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            [key, value],
        )?;
        Ok(())
    }
}
