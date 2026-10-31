use crate::images::texture_hash::cached_filename;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags};
use std::collections::{HashMap, HashSet};
use std::path::Path;

// Kodi's `idxTexture` and `idxSize` indexes aren't unique, so `INSERT OR IGNORE`
// and `INSERT OR REPLACE` never dedupe. These statements check for an existing row.
const INSERT_TEXTURE: &str = "INSERT INTO texture (url, cachedurl, imagehash, lasthashcheck)
     SELECT ?1, ?2, '', ?3 WHERE NOT EXISTS (SELECT 1 FROM texture WHERE url=?1)";
const UPDATE_SIZE: &str =
    "UPDATE sizes SET width=?2, height=?3, lastusetime=?4 WHERE idtexture=?1 AND size=1";
const INSERT_SIZE: &str =
    "INSERT INTO sizes (idtexture, size, width, height, usecount, lastusetime)
     SELECT ?1, 1, ?2, ?3, 1, ?4 WHERE NOT EXISTS (SELECT 1 FROM sizes WHERE idtexture=?1 AND size=1)";

pub struct TextureDb {
    conn: Connection,
}

impl TextureDb {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .with_context(|| format!("open textures db {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=OFF;
             PRAGMA busy_timeout=30000;",
        )?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS version (
                idVersion INTEGER,
                iCompressCount INTEGER
             );
             INSERT OR IGNORE INTO version (idVersion, iCompressCount)
                SELECT 13, 0 WHERE NOT EXISTS (SELECT 1 FROM version);
             CREATE TABLE IF NOT EXISTS texture (
                id INTEGER PRIMARY KEY,
                url TEXT,
                cachedurl TEXT,
                imagehash TEXT,
                lasthashcheck TEXT
             );
             CREATE TABLE IF NOT EXISTS sizes (
                idtexture INTEGER,
                size INTEGER,
                width INTEGER,
                height INTEGER,
                usecount INTEGER,
                lastusetime TEXT
             );
             CREATE INDEX IF NOT EXISTS idxTexture ON texture(url);
             CREATE INDEX IF NOT EXISTS idxSize ON sizes(idtexture, size);",
        )?;
        Ok(Self { conn })
    }

    pub fn is_cached(&self, url: &str) -> bool {
        self.conn
            .prepare_cached("SELECT 1 FROM texture WHERE url=?1 LIMIT 1")
            .and_then(|mut s| s.query_row(params![url], |_| Ok(())))
            .is_ok()
    }

    /// Removes images older versions downloaded below full quality, and SVG logos,
    /// which TMDBHelper never shows. Returns the number of files it deleted. The
    /// next image cycle downloads their originals.
    pub fn purge_lower_quality(&self, thumbnails_dir: &Path) -> Result<usize> {
        let mut by_file: HashMap<String, Vec<i64>> = HashMap::new();
        let mut stale = HashSet::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT id, url, cachedurl FROM texture")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (id, url, cached) = row?;
                let ext = url.rsplit('.').next().unwrap_or("jpg");
                // A file is named for the URL it came from, so this finds the
                // files downloaded at this size, not the ones merely registered.
                let downloaded_small =
                    url.contains("/w1280/") && cached_filename(&url, ext) == cached;
                if downloaded_small || ext.eq_ignore_ascii_case("svg") {
                    stale.insert(cached.clone());
                }
                by_file.entry(cached).or_default().push(id);
            }
        }
        let mut deleted = 0;
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut del_sizes = tx.prepare("DELETE FROM sizes WHERE idtexture=?1")?;
            let mut del_texture = tx.prepare("DELETE FROM texture WHERE id=?1")?;
            for cached in &stale {
                for id in by_file.get(cached).into_iter().flatten() {
                    del_sizes.execute(params![id])?;
                    del_texture.execute(params![id])?;
                }
                match std::fs::remove_file(thumbnails_dir.join(cached)) {
                    Ok(()) => deleted += 1,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e).with_context(|| format!("delete {cached}")),
                }
            }
        }
        tx.commit()?;
        Ok(deleted)
    }

    pub fn load_all_cached_urls(&self) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT url FROM texture")?;
        let urls = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(urls)
    }

    pub fn insert_texture(
        &self,
        url: &str,
        cached_url: &str,
        width: u32,
        height: u32,
    ) -> Result<i64> {
        let now = format_utc_now();
        self.conn
            .prepare_cached(INSERT_TEXTURE)?
            .execute(params![url, cached_url, &now])?;

        let id: i64 = self
            .conn
            .prepare_cached("SELECT id FROM texture WHERE url=?1")?
            .query_row(params![url], |r| r.get(0))?;

        if width > 0 || height > 0 {
            self.conn
                .prepare_cached(UPDATE_SIZE)?
                .execute(params![id, width, height, &now])?;
            self.conn
                .prepare_cached(INSERT_SIZE)?
                .execute(params![id, width, height, &now])?;
        }

        Ok(id)
    }

    pub fn insert_texture_batch(&self, items: &[(String, String, u32, u32)]) -> Result<usize> {
        let now = format_utc_now();
        let tx = self.conn.unchecked_transaction()?;
        let mut count = 0usize;

        {
            let mut ins_tex = tx.prepare_cached(INSERT_TEXTURE)?;
            let mut sel_id = tx.prepare_cached("SELECT id FROM texture WHERE url=?1")?;
            let mut upd_size = tx.prepare_cached(UPDATE_SIZE)?;
            let mut ins_size = tx.prepare_cached(INSERT_SIZE)?;

            for (url, cached_url, w, h) in items {
                if ins_tex.execute(params![url, cached_url, &now])? > 0 {
                    count += 1;
                }
                if *w > 0 || *h > 0 {
                    let id: i64 = sel_id.query_row(params![url], |r| r.get(0))?;
                    upd_size.execute(params![id, w, h, &now])?;
                    ins_size.execute(params![id, w, h, &now])?;
                }
            }
        }

        tx.commit()?;
        Ok(count)
    }
}

fn format_utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    format_utc(secs)
}

fn format_utc(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let time_of_day = secs % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;

    // Civil date from days since 1970-01-01 (algorithm from Howard Hinnant)
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let yr = if mo <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", yr, mo, d, h, m, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn format_utc_known_dates() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00");
        assert_eq!(format_utc(1745570400), "2025-04-25 08:40:00");
        assert_eq!(format_utc(951782400), "2000-02-29 00:00:00");
        assert_eq!(format_utc(1609459199), "2020-12-31 23:59:59");
    }

    #[test]
    fn open_creates_tables() {
        let f = NamedTempFile::new().unwrap();
        let db = TextureDb::open(f.path()).unwrap();
        assert!(!db.is_cached("https://example.com/test.jpg"));
    }

    #[test]
    fn insert_and_query() {
        let f = NamedTempFile::new().unwrap();
        let db = TextureDb::open(f.path()).unwrap();
        let id = db
            .insert_texture(
                "https://image.tmdb.org/t/p/w780/test.jpg",
                "f/fa000000.jpg",
                780,
                1170,
            )
            .unwrap();
        assert!(id > 0);
        assert!(db.is_cached("https://image.tmdb.org/t/p/w780/test.jpg"));
        assert!(!db.is_cached("https://image.tmdb.org/t/p/w500/test.jpg"));
    }

    #[test]
    fn insert_batch() {
        let f = NamedTempFile::new().unwrap();
        let db = TextureDb::open(f.path()).unwrap();
        let items = vec![
            (
                "https://a.com/1.jpg".into(),
                "a/a0000001.jpg".into(),
                100,
                200,
            ),
            (
                "https://a.com/2.jpg".into(),
                "b/b0000002.jpg".into(),
                300,
                400,
            ),
        ];
        let count = db.insert_texture_batch(&items).unwrap();
        assert_eq!(count, 2);
        assert!(db.is_cached("https://a.com/1.jpg"));
        assert!(db.is_cached("https://a.com/2.jpg"));
    }

    #[test]
    fn insert_ignore_duplicate() {
        let f = NamedTempFile::new().unwrap();
        let db = TextureDb::open(f.path()).unwrap();
        db.insert_texture("https://a.com/x.jpg", "a/a0000001.jpg", 100, 200)
            .unwrap();
        db.insert_texture("https://a.com/x.jpg", "a/a0000001.jpg", 100, 200)
            .unwrap();
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM texture WHERE url='https://a.com/x.jpg'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let sizes: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM sizes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sizes, 1);
    }

    #[test]
    fn purge_removes_only_lower_quality_and_svg_files() {
        let dir = tempfile::tempdir().unwrap();
        let f = NamedTempFile::new().unwrap();
        let db = TextureDb::open(f.path()).unwrap();
        let small_url = "https://image.tmdb.org/t/p/w1280/poster.jpg";
        let small = cached_filename(small_url, "jpg");
        // A backdrop's file came from `original`, and w1280 only points at it.
        let full_url = "https://image.tmdb.org/t/p/original/backdrop.jpg";
        let full = cached_filename(full_url, "jpg");
        let svg_url = "https://image.tmdb.org/t/p/original/logo.svg";
        let svg = cached_filename(svg_url, "svg");
        for (url, cached) in [
            (small_url, &small),
            ("https://image.tmdb.org/t/p/original/poster.jpg", &small),
            (full_url, &full),
            ("https://image.tmdb.org/t/p/w1280/backdrop.jpg", &full),
            (svg_url, &svg),
        ] {
            db.insert_texture(url, cached, 10, 10).unwrap();
            let path = dir.path().join(cached);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        }

        assert_eq!(db.purge_lower_quality(dir.path()).unwrap(), 2);
        assert!(!dir.path().join(&small).exists());
        assert!(!dir.path().join(&svg).exists());
        assert!(dir.path().join(&full).exists());
        assert!(!db.is_cached(small_url));
        assert!(!db.is_cached("https://image.tmdb.org/t/p/original/poster.jpg"));
        assert!(db.is_cached(full_url));
        assert!(db.is_cached("https://image.tmdb.org/t/p/w1280/backdrop.jpg"));
    }

    #[test]
    fn multiple_variants_share_cached_url() {
        let f = NamedTempFile::new().unwrap();
        let db = TextureDb::open(f.path()).unwrap();
        let cached = "f/fa000000.jpg";
        db.insert_texture(
            "https://image.tmdb.org/t/p/original/test.jpg",
            cached,
            1920,
            1080,
        )
        .unwrap();
        db.insert_texture(
            "https://image.tmdb.org/t/p/w1280/test.jpg",
            cached,
            1920,
            1080,
        )
        .unwrap();
        db.insert_texture(
            "https://image.tmdb.org/t/p/w780/test.jpg",
            cached,
            1920,
            1080,
        )
        .unwrap();

        let mut stmt = db
            .conn
            .prepare("SELECT cachedurl FROM texture WHERE cachedurl=?1")
            .unwrap();
        let rows: Vec<String> = stmt
            .query_map(params![cached], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(rows.len(), 3);
    }
}
