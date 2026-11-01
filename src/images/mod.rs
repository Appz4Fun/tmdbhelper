pub mod texture_db;
pub mod texture_hash;

use anyhow::{Context, Result};
use reqwest::StatusCode;
use rusqlite::{Connection, OpenFlags};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Semaphore};
use tracing::{error, info, warn};

struct ArtRow {
    path: String,
    art_type: String,
}

struct SavedImage {
    path: String,
    art_type: String,
    cached_filename: String,
    size: u64,
    width: u32,
    height: u32,
}

struct RegisterOnly {
    path: String,
    art_type: String,
    cached_filename: String,
}

fn parse_jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 2 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 1 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        if marker == 0xD9 {
            break;
        }
        // SOF0 or SOF2 — these contain dimensions
        if marker == 0xC0 || marker == 0xC2 {
            if i + 9 < data.len() {
                let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
                let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
                return Some((w, h));
            }
            return None;
        }
        // Skip segment
        if i + 3 < data.len() {
            let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            i += 2 + len;
        } else {
            break;
        }
    }
    None
}

fn parse_png_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 24 {
        return None;
    }
    let png_sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if data[..8] != png_sig {
        return None;
    }
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    Some((w, h))
}

fn parse_image_dimensions(data: &[u8]) -> (u32, u32) {
    parse_jpeg_dimensions(data)
        .or_else(|| parse_png_dimensions(data))
        .unwrap_or((0, 0))
}

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// Saves an image under its cached name and returns its size, width, and height.
/// Writing to a temporary name means a stopped run can't leave a truncated file
/// that later cycles count as finished. Each save gets its own temporary name,
/// so two rows that share an image can't overwrite each other's partial file.
fn save_image(dir: &Path, cached_filename: &str, bytes: &[u8]) -> std::io::Result<(u64, u32, u32)> {
    let file_path = dir.join(cached_filename);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let temp_path = file_path.with_extension(format!("{id}.part"));
    std::fs::write(&temp_path, bytes)?;
    if let Err(e) = std::fs::rename(&temp_path, &file_path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }
    let (width, height) = parse_image_dimensions(bytes);
    Ok((bytes.len() as u64, width, height))
}

pub async fn run(
    item_details_path: PathBuf,
    textures_db_path: PathBuf,
    thumbnails_dir: PathBuf,
    concurrency: usize,
    min_free_bytes: u64,
) -> Result<()> {
    tokio::task::spawn_blocking({
        let textures_db_path = textures_db_path.clone();
        let thumbnails_dir = thumbnails_dir.clone();
        move || -> Result<()> {
            let db = texture_db::TextureDb::open(&textures_db_path)?;
            let deleted = db.purge_lower_quality(&thumbnails_dir)?;
            if deleted > 0 {
                info!("deleted {deleted} images below full quality");
            }
            Ok(())
        }
    })
    .await??;

    let sem = Arc::new(Semaphore::new(concurrency));
    let (tx, mut rx) = mpsc::channel::<SavedImage>(200);
    let (reg_tx, mut reg_rx) = mpsc::channel::<RegisterOnly>(200);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .pool_max_idle_per_host(50)
        .build()
        .context("build image HTTP client")?;

    #[allow(unused_variables)]
    let writer_handle = {
        let textures_db_path = textures_db_path.clone();
        let thumbnails_dir = thumbnails_dir.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let db = texture_db::TextureDb::open(&textures_db_path)?;
            let mut batch: Vec<(String, String, u32, u32)> = Vec::with_capacity(200);
            let mut written = 0u64;
            let mut bytes_total = 0u64;
            let mut registered = 0u64;
            let start = std::time::Instant::now();

            const BATCH_SIZE: usize = 200;

            #[allow(clippy::while_let_loop)]
            loop {
                while let Ok(reg) = reg_rx.try_recv() {
                    let file_path = thumbnails_dir.join(&reg.cached_filename);
                    let data = match std::fs::read(&file_path) {
                        Ok(d) => d,
                        Err(_) => continue,
                    };
                    let (w, h) = parse_image_dimensions(&data);
                    let variants = texture_hash::url_variants(&reg.path, &reg.art_type);
                    for url in &variants {
                        batch.push((url.clone(), reg.cached_filename.clone(), w, h));
                    }
                    if batch.len() >= BATCH_SIZE {
                        if let Err(e) = db.insert_texture_batch(&batch) {
                            error!("texture reg batch insert failed: {:?}", e);
                        }
                        batch.clear();
                    }
                    registered += 1;
                }

                // With nothing waiting, save what's done so far: a batch can sit
                // unwritten for the whole 5-minute wait between cycles otherwise.
                if rx.is_empty() && !batch.is_empty() {
                    if let Err(e) = db.insert_texture_batch(&batch) {
                        error!("texture batch insert failed: {:?}", e);
                    }
                    batch.clear();
                }

                let dl = match rx.blocking_recv() {
                    Some(dl) => dl,
                    None => break,
                };

                bytes_total += dl.size;
                let (w, h) = (dl.width, dl.height);

                let variants = texture_hash::url_variants(&dl.path, &dl.art_type);
                for url in &variants {
                    batch.push((url.clone(), dl.cached_filename.clone(), w, h));
                }

                if batch.len() >= BATCH_SIZE {
                    if let Err(e) = db.insert_texture_batch(&batch) {
                        error!("texture batch insert failed: {:?}", e);
                    }
                    batch.clear();
                }

                written += 1;
                if written.is_multiple_of(1000) {
                    let elapsed = start.elapsed().as_secs_f64().max(1.0);
                    let rate = written as f64 / elapsed;
                    let mb = bytes_total as f64 / (1024.0 * 1024.0);
                    info!(
                        "images: written={} registered={} rate={:.1}/s disk={:.0}MB",
                        written, registered, rate, mb
                    );
                }
            }

            if !batch.is_empty() {
                if let Err(e) = db.insert_texture_batch(&batch) {
                    error!("texture final batch insert failed: {:?}", e);
                }
            }
            info!(
                "image writer exiting, total written={} registered={}",
                written, registered
            );
            Ok(())
        })
    };

    loop {
        let art_rows = tokio::task::spawn_blocking({
            let p = item_details_path.clone();
            move || -> Result<Vec<ArtRow>> {
                let conn = Connection::open_with_flags(&p, OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .with_context(|| format!("open ItemDetails.db {}", p.display()))?;
                // Most popular first, so a disk limit keeps the art people will see.
                // SVG logos are skipped because TMDBHelper only shows PNG logos.
                let mut stmt = conn.prepare(
                    "SELECT x.icon, x.type FROM (
                         SELECT icon, type, parent_id FROM art
                         WHERE icon IS NOT NULL AND extension != 'svg'
                         UNION ALL
                         SELECT icon, type, parent_id FROM fanart_tv WHERE icon IS NOT NULL
                     ) x
                     LEFT JOIN movie m ON m.id = x.parent_id
                     LEFT JOIN tvshow t ON t.id = x.parent_id
                     LEFT JOIN person p ON p.id = x.parent_id
                     LEFT JOIN season s ON s.id = x.parent_id
                     LEFT JOIN episode e ON e.id = x.parent_id
                     LEFT JOIN tvshow st ON st.id = COALESCE(s.tvshow_id, e.tvshow_id)
                     GROUP BY x.icon, x.type
                     ORDER BY MAX(COALESCE(m.popularity, t.popularity, p.popularity, st.popularity, 0)) DESC",
                )?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok(ArtRow {
                            path: r.get(0)?,
                            art_type: r.get(1)?,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(rows)
            }
        })
        .await??;

        info!(
            "image cycle: {} art rows from ItemDetails.db",
            art_rows.len()
        );
        if art_rows.is_empty() {
            info!("no art rows, sleeping 5 minutes");
            tokio::time::sleep(Duration::from_secs(300)).await;
            continue;
        }

        let has_room = || {
            let free = crate::paths::available_space(&thumbnails_dir).unwrap_or(u64::MAX);
            if free < min_free_bytes {
                warn!(
                    "image downloads paused: {}MB free is under the {}MB minimum, sleeping 5 minutes",
                    free / (1024 * 1024),
                    min_free_bytes / (1024 * 1024)
                );
            }
            free >= min_free_bytes
        };
        if !has_room() {
            tokio::time::sleep(Duration::from_secs(300)).await;
            continue;
        }

        let cached_urls = tokio::task::spawn_blocking({
            let p = textures_db_path.clone();
            move || -> Result<HashSet<String>> {
                let db = texture_db::TextureDb::open(&p)?;
                db.load_all_cached_urls()
            }
        })
        .await??;
        info!("image skip cache: {} cached URLs", cached_urls.len());

        let existing_files = tokio::task::spawn_blocking({
            let dir = thumbnails_dir.clone();
            move || -> HashSet<String> {
                let mut set = HashSet::new();
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    return set;
                };
                for subdir in entries.filter_map(|e| e.ok()) {
                    if !subdir.file_type().is_ok_and(|t| t.is_dir()) {
                        continue;
                    }
                    let prefix = subdir.file_name().to_string_lossy().to_string();
                    let Ok(files) = std::fs::read_dir(subdir.path()) else {
                        continue;
                    };
                    for file in files.filter_map(|e| e.ok()) {
                        let name = file.file_name().to_string_lossy().to_string();
                        set.insert(format!("{}/{}", prefix, name));
                    }
                }
                set
            }
        })
        .await?;
        info!("image skip cache: {} files on disk", existing_files.len());

        let mut dispatched = 0u64;
        let mut skipped = 0u64;

        for row in &art_rows {
            let ext = row.path.rsplit('.').next().unwrap_or("jpg");
            let download_url = texture_hash::download_url(&row.path);
            let cached_filename = texture_hash::cached_filename(&download_url, ext);

            if existing_files.contains(&cached_filename) {
                let variants = texture_hash::url_variants(&row.path, &row.art_type);
                let any_registered = variants.iter().any(|u| cached_urls.contains(u));
                if any_registered {
                    skipped += 1;
                    continue;
                }
                let _ = reg_tx.try_send(RegisterOnly {
                    path: row.path.clone(),
                    art_type: row.art_type.clone(),
                    cached_filename: cached_filename.clone(),
                });
                skipped += 1;
                continue;
            }

            let permit = sem.clone().acquire_owned().await?;
            let client = client.clone();
            let tx = tx.clone();
            let path = row.path.clone();
            let art_type = row.art_type.clone();
            let url = download_url.clone();
            let cf = cached_filename.clone();
            let thumbnails_dir = thumbnails_dir.clone();
            tokio::spawn(async move {
                let _permit = permit;
                for attempt in 0u32..3 {
                    let resp = match client.get(&url).send().await {
                        Ok(r) => r,
                        Err(e) if e.is_timeout() || e.is_connect() || e.is_request() => {
                            let backoff = Duration::from_millis(500 * (1 << attempt));
                            warn!(
                                "image transport error {}, retry {} in {:?}: {}",
                                url,
                                attempt + 1,
                                backoff,
                                e
                            );
                            tokio::time::sleep(backoff).await;
                            continue;
                        }
                        Err(e) => {
                            warn!("image fetch {}: {:?}", url, e);
                            return;
                        }
                    };
                    let status = resp.status();
                    if status.is_success() {
                        match resp.bytes().await {
                            Ok(bytes) => {
                                // Saving here, not in the database writer, lets a slow
                                // network share take many files at once.
                                let (dir, name) = (thumbnails_dir, cf.clone());
                                let saved = tokio::task::spawn_blocking(move || {
                                    save_image(&dir, &name, &bytes)
                                })
                                .await;
                                match saved {
                                    Ok(Ok((size, width, height))) => {
                                        let _ = tx
                                            .send(SavedImage {
                                                path,
                                                art_type,
                                                cached_filename: cf,
                                                size,
                                                width,
                                                height,
                                            })
                                            .await;
                                    }
                                    Ok(Err(e)) => error!("write {} failed: {:?}", cf, e),
                                    Err(e) => error!("write {} failed: {:?}", cf, e),
                                }
                            }
                            Err(e) => warn!("image read body {}: {:?}", url, e),
                        }
                        return;
                    }
                    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                        let base_backoff = Duration::from_millis(500 * (1 << attempt));
                        let retry_after = resp
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok())
                            .map(Duration::from_secs);
                        let backoff = retry_after.map_or(base_backoff, |ra| ra.max(base_backoff));
                        warn!(
                            "image {} for {}, retry {} in {:?}",
                            status,
                            url,
                            attempt + 1,
                            backoff
                        );
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    warn!("image fetch {} -> {}", url, status);
                    return;
                }
                warn!("image retries exhausted for {}", url);
            });

            dispatched += 1;
            if dispatched.is_multiple_of(1000) && !has_room() {
                break;
            }
            if dispatched.is_multiple_of(10000) {
                info!(
                    "image dispatch progress: dispatched={} skipped={}",
                    dispatched, skipped
                );
            }
        }

        // Wait for all in-flight downloads to complete
        let drain = sem.acquire_many(concurrency as u32).await?;
        drop(drain);

        info!(
            "image cycle complete: dispatched={} skipped={}, sleeping 5 minutes",
            dispatched, skipped
        );
        tokio::time::sleep(Duration::from_secs(300)).await;
    }

    #[allow(unreachable_code)]
    {
        drop(tx);
        drop(reg_tx);
        writer_handle.await??;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_image_writes_the_file_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut png = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D,
        ];
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&800u32.to_be_bytes());
        png.extend_from_slice(&600u32.to_be_bytes());
        let saved = save_image(dir.path(), "a/a0000001.png", &png).unwrap();
        assert_eq!(saved, (png.len() as u64, 800, 600));
        assert_eq!(
            std::fs::read(dir.path().join("a/a0000001.png")).unwrap(),
            png
        );
        let names: Vec<_> = std::fs::read_dir(dir.path().join("a")).unwrap().collect();
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn concurrent_saves_of_one_image_all_succeed() {
        let dir = tempfile::tempdir().unwrap();
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let path = dir.path().to_path_buf();
                std::thread::spawn(move || save_image(&path, "b/b0000002.jpg", &[7u8; 4096]))
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        assert_eq!(std::fs::read_dir(dir.path().join("b")).unwrap().count(), 1);
        assert_eq!(
            std::fs::read(dir.path().join("b/b0000002.jpg"))
                .unwrap()
                .len(),
            4096
        );
    }

    #[test]
    fn jpeg_dimensions() {
        // Minimal JPEG: SOI + SOF0 with 100x200
        let mut data = vec![0xFF, 0xD8]; // SOI
        data.extend_from_slice(&[0xFF, 0xC0]); // SOF0
        data.extend_from_slice(&[0x00, 0x0B]); // length=11
        data.push(8); // precision
        data.extend_from_slice(&[0x00, 0xC8]); // height=200
        data.extend_from_slice(&[0x00, 0x64]); // width=100
        data.extend_from_slice(&[0x03, 0x01, 0x22, 0x00]); // components
        assert_eq!(parse_jpeg_dimensions(&data), Some((100, 200)));
    }

    #[test]
    fn jpeg_sof2_progressive() {
        let mut data = vec![0xFF, 0xD8]; // SOI
        data.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]); // APP0 len=16
        data.extend_from_slice(&[0; 14]); // APP0 payload
        data.extend_from_slice(&[0xFF, 0xC2]); // SOF2 (progressive)
        data.extend_from_slice(&[0x00, 0x0B]); // length
        data.push(8);
        data.extend_from_slice(&[0x04, 0x38]); // height=1080
        data.extend_from_slice(&[0x07, 0x80]); // width=1920
        data.extend_from_slice(&[0x03, 0x01, 0x22, 0x00]);
        assert_eq!(parse_jpeg_dimensions(&data), Some((1920, 1080)));
    }

    #[test]
    fn png_dimensions() {
        let mut data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]; // PNG sig
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]); // IHDR length
        data.extend_from_slice(b"IHDR");
        data.extend_from_slice(&800u32.to_be_bytes()); // width=800
        data.extend_from_slice(&600u32.to_be_bytes()); // height=600
        assert_eq!(parse_png_dimensions(&data), Some((800, 600)));
    }

    #[test]
    fn unknown_format_returns_zero() {
        assert_eq!(parse_image_dimensions(&[0x00, 0x01, 0x02]), (0, 0));
        assert_eq!(parse_image_dimensions(&[]), (0, 0));
    }
}
