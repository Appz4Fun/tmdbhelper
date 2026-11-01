//! Fills `fanart_tv` for movies and shows the metadata crawl has written.
//! With its fanart.tv lookup on, TMDBHelper only reads a movie or show once
//! `baseitem.fanart_tv` is set, whether or not fanart.tv has art for it.

use crate::cache::{mapper, open_writer, WriteContext};
use anyhow::{Context, Result};
use reqwest::{Client, StatusCode};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{error, info, warn};

const API_URL: &str = "https://webservice.fanart.tv/v3";
const BATCH_SIZE: i64 = 500;
const RETRIES: u32 = 3;

pub struct Options {
    pub item_details_path: PathBuf,
    pub api_key: String,
    pub client_key: Option<String>,
    pub language: String,
    pub concurrency: usize,
    pub min_free_bytes: u64,
}

struct Item {
    id: String,
    /// The fanart.tv path, or `None` for a show with no TVDB ID, which it can't look up.
    path: Option<String>,
}

enum Outcome {
    Found(Value),
    NotListed,
    Unavailable(String),
}

async fn fetch(client: &Client, query: &str, path: &str) -> Result<Outcome> {
    let url = format!("{API_URL}/{path}?{query}");
    let mut last = String::new();
    for attempt in 0..RETRIES {
        let backoff = Duration::from_millis(500 * (1 << attempt));
        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                // The URL holds the API keys, so it must never reach a log.
                last = e.without_url().to_string();
                tokio::time::sleep(backoff).await;
                continue;
            }
        };
        let status = resp.status();
        if status.is_success() {
            return match resp.json::<Value>().await {
                Ok(data) => Ok(Outcome::Found(data)),
                Err(e) => Ok(Outcome::Unavailable(e.without_url().to_string())),
            };
        }
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                anyhow::bail!("fanart.tv rejected the API key ({status})")
            }
            StatusCode::NOT_FOUND => return Ok(Outcome::NotListed),
            _ => {
                last = status.to_string();
                tokio::time::sleep(backoff * 4).await;
            }
        }
    }
    Ok(Outcome::Unavailable(last))
}

/// The next movies and shows without fanart, most popular first.
fn next_items(conn: &Connection, skip: &HashSet<String>) -> Result<Vec<Item>> {
    let limit = BATCH_SIZE + skip.len() as i64;
    let mut stmt = conn.prepare_cached(
        "SELECT b.id, b.mediatype, COALESCE(m.tmdb_id, t.tmdb_id),
                (SELECT value FROM unique_id u WHERE u.parent_id = b.id AND u.key = 'tvdb')
         FROM baseitem b
         LEFT JOIN movie m ON m.id = b.id
         LEFT JOIN tvshow t ON t.id = b.id
         WHERE b.fanart_tv = 0 AND b.datalevel >= 5 AND b.mediatype IN ('movie', 'tvshow')
         ORDER BY COALESCE(m.popularity, t.popularity, 0) DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut items = Vec::new();
    for row in rows {
        let (id, mediatype, tmdb_id, tvdb_id) = row?;
        if skip.contains(&id) {
            continue;
        }
        let path = if mediatype == "movie" {
            tmdb_id.map(|i| format!("movies/{i}"))
        } else {
            tvdb_id
                .filter(|v| v.parse::<i64>().is_ok())
                .map(|v| format!("tv/{v}"))
        };
        items.push(Item { id, path });
    }
    Ok(items)
}

pub async fn run(options: Options) -> Result<()> {
    let Options {
        item_details_path,
        api_key,
        client_key,
        language,
        concurrency,
        min_free_bytes,
    } = options;
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(50)
        .build()
        .context("build fanart.tv HTTP client")?;
    let query = match &client_key {
        Some(key) => format!("api_key={api_key}&client_key={key}"),
        None => format!("api_key={api_key}"),
    };
    let query = Arc::new(query);
    let sem = Arc::new(Semaphore::new(concurrency));
    let mut conn = open_writer(&item_details_path)?;
    let ctx = WriteContext {
        language,
        genres: Default::default(),
    };
    // Items that keep failing wait for the next run, so they don't block the rest.
    let mut skip = HashSet::new();
    let mut done = 0u64;
    let start = std::time::Instant::now();

    loop {
        let free = crate::paths::available_space(crate::paths::parent_dir(&item_details_path))
            .unwrap_or(u64::MAX);
        if free < min_free_bytes {
            warn!(
                "fanart crawl paused: {}MB free is under the {}MB minimum, sleeping 5 minutes",
                free / (1024 * 1024),
                min_free_bytes / (1024 * 1024)
            );
            tokio::time::sleep(Duration::from_secs(300)).await;
            continue;
        }
        let items = next_items(&conn, &skip)?;
        if items.is_empty() {
            info!("no movies or shows need fanart, sleeping 5 minutes");
            tokio::time::sleep(Duration::from_secs(300)).await;
            continue;
        }

        let mut tasks = JoinSet::new();
        for item in items {
            let permit = sem.clone().acquire_owned().await?;
            let client = client.clone();
            let query = query.clone();
            tasks.spawn(async move {
                let _permit = permit;
                let outcome = match &item.path {
                    Some(path) => fetch(&client, &query, path).await,
                    None => Ok(Outcome::NotListed),
                };
                (item.id, outcome)
            });
        }
        let results = tasks.join_all().await;

        let tx = conn.transaction()?;
        for (id, outcome) in results {
            match outcome? {
                Outcome::Found(data) => {
                    if let Err(e) = mapper::write_fanart(&tx, &ctx, &id, &data) {
                        error!("fanart write failed for {id}: {e:?}");
                        skip.insert(id);
                        continue;
                    }
                }
                Outcome::NotListed => mapper::mark_fanart_checked(&tx, &id)?,
                Outcome::Unavailable(reason) => {
                    warn!("fanart.tv is unavailable for {id}: {reason}");
                    skip.insert(id);
                    continue;
                }
            }
            done += 1;
            if done.is_multiple_of(1000) {
                let rate = done as f64 / start.elapsed().as_secs_f64().max(1.0);
                info!(
                    "fanart: done={done} skipped={} rate={rate:.1}/s",
                    skip.len()
                );
            }
        }
        tx.commit()?;
    }
}
