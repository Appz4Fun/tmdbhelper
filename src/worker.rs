use crate::api::{TmdbClient, TmdbError};
use crate::cache::{mapper, now_unix, open_writer, WriteContext};
use crate::id::{pack_episode_key, unpack_episode_key, TmdbType};
use crate::seed;
use crate::state::{Child, QueueItem, StateDb};
use anyhow::Result;
use rusqlite::Connection;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Semaphore};
use tracing::{error, info, warn};

const MAX_DEPTH: i64 = 2;
const MEGA_TX_SIZE: usize = 200;
const EXPORT_DATE_KEY: &str = "export_date";

pub struct Options {
    pub state_path: PathBuf,
    pub item_details_path: PathBuf,
    pub api_key: String,
    pub language: String,
    pub concurrency: usize,
    pub batch_size: usize,
    pub all_ids: bool,
    pub min_free_bytes: u64,
}

/// One response, with the episode groups a show fetches alongside it.
pub struct WriteJob {
    pub item: QueueItem,
    pub payload: Value,
    pub groups: Vec<Value>,
}

/// What the writer thread does next. One channel keeps results in order, and it
/// keeps failures moving when no fetch succeeds.
enum WriterMessage {
    Write(Box<WriteJob>),
    NotFound(QueueItem),
    Failed(QueueItem),
    Unavailable(QueueItem),
}

fn credit_children(children: &mut Vec<Child>, items: &Value, keys: &[&str], depth: i64) {
    for key in keys {
        for credit in items[key].as_array().into_iter().flatten() {
            if let Some(id) = credit["id"].as_i64() {
                let popularity = credit["popularity"].as_f64().unwrap_or(0.0);
                children.push((id, TmdbType::Person, popularity, depth));
            }
        }
    }
}

fn related_children(
    children: &mut Vec<Child>,
    value: &Value,
    keys: &[&str],
    tmdb_type: TmdbType,
    depth: i64,
) {
    for key in keys {
        let items = value[key]["results"]
            .as_array()
            .or_else(|| value[key].as_array());
        for related in items.into_iter().flatten() {
            if let Some(id) = related["id"].as_i64() {
                let popularity = related["popularity"].as_f64().unwrap_or(0.0);
                children.push((id, tmdb_type, popularity, depth));
            }
        }
    }
}

/// Extract the items a response leads to. ALL persons from credits are enqueued
/// (user requirement: "every IMDb-grade page warm"). Credits and similar titles
/// are one hop further away. Seasons and episodes are part of their show, so they
/// stay at its depth and popularity however far the crawl has gone.
pub fn extract_children(item: &QueueItem, payload: &Value) -> Vec<Child> {
    let mut children = Vec::new();
    let next = item.depth + 1;
    match item.tmdb_type {
        TmdbType::Movie => {
            credit_children(&mut children, &payload["credits"], &["cast", "crew"], next);
            related_children(
                &mut children,
                payload,
                &["similar", "recommendations"],
                TmdbType::Movie,
                next,
            );
            if let Some(id) = payload["belongs_to_collection"]["id"].as_i64() {
                children.push((id, TmdbType::Collection, item.popularity, next));
            }
        }
        TmdbType::Tv => {
            credit_children(
                &mut children,
                &payload["aggregate_credits"],
                &["cast", "crew"],
                next,
            );
            credit_children(&mut children, payload, &["created_by"], next);
            related_children(
                &mut children,
                payload,
                &["similar", "recommendations"],
                TmdbType::Tv,
                next,
            );
        }
        TmdbType::Episode => credit_children(
            &mut children,
            &payload["credits"],
            &["guest_stars", "crew"],
            next,
        ),
        TmdbType::Person => {
            for (key, tmdb_type) in [
                ("movie_credits", TmdbType::Movie),
                ("tv_credits", TmdbType::Tv),
            ] {
                related_children(
                    &mut children,
                    &payload[key],
                    &["cast", "crew"],
                    tmdb_type,
                    next,
                );
            }
        }
        TmdbType::Collection => {
            related_children(&mut children, payload, &["parts"], TmdbType::Movie, next)
        }
        TmdbType::Season => {}
    }
    if item.depth >= MAX_DEPTH {
        children.clear();
    }
    match item.tmdb_type {
        TmdbType::Tv => {
            let popularity = payload["popularity"].as_f64().unwrap_or(item.popularity);
            for season in payload["seasons"].as_array().into_iter().flatten() {
                if let Some(number) = season["season_number"].as_i64() {
                    let key = pack_episode_key(item.tmdb_id, number, 0);
                    children.push((key, TmdbType::Season, popularity, item.depth));
                }
            }
        }
        TmdbType::Season => {
            let (tv_id, season, _) = unpack_episode_key(item.tmdb_id);
            for episode in payload["episodes"].as_array().into_iter().flatten() {
                if let Some(number) = episode["episode_number"].as_i64() {
                    let key = pack_episode_key(tv_id, season, number);
                    children.push((key, TmdbType::Episode, item.popularity, item.depth));
                }
            }
        }
        _ => {}
    }
    children
}

/// Writes one response in the shape its queue type calls for.
pub fn write_job(conn: &Connection, ctx: &WriteContext, job: &WriteJob) -> Result<()> {
    let payload = &job.payload;
    match job.item.tmdb_type {
        TmdbType::Movie => mapper::write_movie(conn, ctx, payload),
        TmdbType::Tv => mapper::write_tv(conn, ctx, payload, &job.groups),
        TmdbType::Season => {
            let (tv_id, season, _) = unpack_episode_key(job.item.tmdb_id);
            mapper::write_season(conn, ctx, tv_id, season, payload)
        }
        TmdbType::Episode => {
            let (tv_id, season, episode) = unpack_episode_key(job.item.tmdb_id);
            mapper::write_episode(conn, ctx, tv_id, season, episode, payload)
        }
        TmdbType::Person => mapper::write_person(conn, ctx, payload),
        TmdbType::Collection => mapper::write_collection(conn, ctx, payload),
    }
}

async fn fetch(client: &TmdbClient, item: &QueueItem) -> Result<(Value, Vec<Value>), TmdbError> {
    let id = item.tmdb_id;
    let payload = match item.tmdb_type {
        TmdbType::Movie => client.get_movie(id).await?,
        TmdbType::Tv => client.get_tv(id).await?,
        TmdbType::Season => {
            let (tv_id, season, _) = unpack_episode_key(id);
            client.get_season(tv_id, season).await?
        }
        TmdbType::Episode => {
            let (tv_id, season, episode) = unpack_episode_key(id);
            client.get_episode(tv_id, season, episode).await?
        }
        TmdbType::Person => client.get_person(id).await?,
        TmdbType::Collection => client.get_collection(id).await?,
    };
    let mut groups = Vec::new();
    if item.tmdb_type == TmdbType::Tv {
        for group in payload["episode_groups"]["results"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(group_id) = group["id"].as_str() {
                match client.get_episode_group(group_id).await {
                    Ok(g) => groups.push(g),
                    Err(e @ TmdbError::Unauthorized(_)) => return Err(e),
                    Err(e) => warn!("episode group {group_id} failed: {e}"),
                }
            }
        }
    }
    Ok((payload, groups))
}

fn flush_batch(
    writer: &mut Connection,
    state: &mut StateDb,
    ctx: &WriteContext,
    batch: &[WriteJob],
) -> Result<(u64, u64)> {
    let t0 = std::time::Instant::now();
    let tx = writer.transaction()?;
    for job in batch {
        // A savepoint keeps one bad item from leaving half its rows behind.
        tx.execute_batch("SAVEPOINT item")?;
        if let Err(e) = write_job(&tx, ctx, job) {
            error!(
                "write failed for {:?} {}: {:?}",
                job.item.tmdb_type, job.item.tmdb_id, e
            );
            tx.execute_batch("ROLLBACK TO item")?;
        }
        tx.execute_batch("RELEASE item")?;
    }
    tx.commit()?;
    crate::cache::checkpoint_passive(writer);
    let write_ms = t0.elapsed().as_millis() as u64;

    let t1 = std::time::Instant::now();
    let children: Vec<Vec<Child>> = batch
        .iter()
        .map(|job| extract_children(&job.item, &job.payload))
        .collect();
    let state_items: Vec<crate::state::ChildBatch<'_>> = batch
        .iter()
        .zip(&children)
        .map(|(job, c)| (job.item.tmdb_id, job.item.tmdb_type, c.as_slice()))
        .collect();
    if let Err(e) = state.visit_and_enqueue_multi(&state_items) {
        error!("state mega-batch failed: {:?}", e);
    }
    Ok((write_ms, t1.elapsed().as_millis() as u64))
}

fn apply_failure(state: &mut StateDb, message: WriterMessage) {
    let result = match message {
        WriterMessage::NotFound(item) => state.finish(item.tmdb_id, item.tmdb_type, "not_found"),
        WriterMessage::Failed(item) => state.record_failure(item.tmdb_id, item.tmdb_type),
        WriterMessage::Unavailable(item) => state.release(item.tmdb_id, item.tmdb_type),
        WriterMessage::Write(_) => Ok(()),
    };
    if let Err(e) = result {
        error!("state update failed: {:?}", e);
    }
}

async fn seed_exports(state: &mut StateDb) -> Result<()> {
    let date = seed::export_date(now_unix());
    if state.meta(EXPORT_DATE_KEY)?.as_deref() == Some(date.as_str()) {
        return Ok(());
    }
    info!("loading TMDB's full ID lists for {date}");
    let ids: Vec<Child> = seed::fetch_export_ids(&date)
        .await?
        .into_iter()
        // Depth 2 runs after the popular titles and adds no hops, since these
        // lists already name everything.
        .map(|s| (s.tmdb_id, s.tmdb_type, s.popularity, MAX_DEPTH))
        .collect();
    for chunk in ids.chunks(100_000) {
        state.enqueue_many(chunk)?;
    }
    state.set_meta(EXPORT_DATE_KEY, &date)?;
    info!("queued {} items from TMDB's ID lists", ids.len());
    Ok(())
}

fn exit_on_bad_key(e: &TmdbError) {
    if matches!(e, TmdbError::Unauthorized(_)) {
        error!("{e}. Check --tmdb-api-key or WARMUP_TMDB_API_KEY");
        std::process::exit(2);
    }
}

pub async fn run(options: Options) -> Result<()> {
    let Options {
        state_path,
        item_details_path,
        api_key,
        language,
        concurrency,
        batch_size,
        all_ids,
        min_free_bytes,
    } = options;
    let client = TmdbClient::new(api_key, language.clone())?;
    if let Err(e) = client.check_key().await {
        exit_on_bad_key(&e);
        warn!("couldn't check the API key: {e}");
    }
    let genres = match client.get_genres().await {
        Ok(g) => g,
        Err(e) => {
            exit_on_bad_key(&e);
            warn!("genre names unavailable: {e}");
            Default::default()
        }
    };
    let ctx = WriteContext { language, genres };

    let sem = Arc::new(Semaphore::new(concurrency));
    let (tx, mut rx) = mpsc::channel::<WriterMessage>(200);

    let mut state = StateDb::open(&state_path)?;
    let released = state.release_claims()?;
    if released > 0 {
        info!("returned {released} items from the last run to the queue");
    }

    let free_space_dir = item_details_path.clone();
    let writer_handle = {
        let state_path = state_path.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut writer = open_writer(&item_details_path)?;
            let mut state = StateDb::open(&state_path)?;
            let mut batch: Vec<WriteJob> = Vec::with_capacity(MEGA_TX_SIZE);
            let mut written = 0u64;
            let start = std::time::Instant::now();
            let mut total_write_ms = 0u64;
            let mut total_state_ms = 0u64;

            while let Some(message) = rx.blocking_recv() {
                let mut pending = Some(message);
                while let Some(message) = pending.take() {
                    match message {
                        WriterMessage::Write(job) => batch.push(*job),
                        other => apply_failure(&mut state, other),
                    }
                    if batch.len() < MEGA_TX_SIZE {
                        pending = rx.try_recv().ok();
                    }
                }
                if batch.is_empty() {
                    continue;
                }
                let (write_ms, state_ms) = flush_batch(&mut writer, &mut state, &ctx, &batch)?;
                total_write_ms += write_ms;
                total_state_ms += state_ms;

                written += batch.len() as u64;
                if written % 100 < batch.len() as u64 || written == batch.len() as u64 {
                    let elapsed = start.elapsed().as_secs_f64().max(1.0);
                    let rate = written as f64 / elapsed;
                    let qsize = state.queue_size().unwrap_or(0);
                    let vcount = state.visited_count().unwrap_or(0);
                    let avg_write = total_write_ms.checked_div(written).unwrap_or(0);
                    let avg_state = total_state_ms.checked_div(written).unwrap_or(0);
                    info!("written={} visited={} queue={} rate={:.1}/s batch={} avg_write={}ms avg_state={}ms",
                          written, vcount, qsize, rate, batch.len(), avg_write, avg_state);
                }
                batch.clear();
            }
            info!("writer task exiting, total written={}", written);
            Ok(())
        })
    };

    if all_ids {
        if let Err(e) = seed_exports(&mut state).await {
            warn!("TMDB's ID lists are unavailable: {e:#}");
        }
    }

    loop {
        if writer_handle.is_finished() {
            writer_handle.await??;
            anyhow::bail!("the database writer stopped");
        }
        let free = free_space_dir
            .parent()
            .map_or(u64::MAX, |p| fs2::available_space(p).unwrap_or(u64::MAX));
        if free < min_free_bytes {
            warn!(
                "metadata crawl paused: {}MB free is under the {}MB minimum, sleeping 5 minutes",
                free / (1024 * 1024),
                min_free_bytes / (1024 * 1024)
            );
            tokio::time::sleep(Duration::from_secs(300)).await;
            continue;
        }
        let batch = state.pop_batch(batch_size)?;
        if batch.is_empty() {
            // Claimed items can still add to the queue, so wait for them.
            if state.claimed_count()? > 0 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            info!("queue empty, running seeder");
            let seeds = match seed::fetch_seeds(&client).await {
                Ok(seeds) => seeds,
                Err(e) => {
                    if let Some(e) = e.downcast_ref::<TmdbError>() {
                        exit_on_bad_key(e);
                    }
                    warn!("seeder failed: {e:#}");
                    Vec::new()
                }
            };
            info!("seeder returned {} items", seeds.len());

            let requeued = state.requeue_expired(30)?;
            info!("requeued {} expired items", requeued);

            for s in &seeds {
                state.enqueue_child(s.tmdb_id, s.tmdb_type, 0, s.popularity)?;
            }
            if all_ids {
                if let Err(e) = seed_exports(&mut state).await {
                    warn!("TMDB's ID lists are unavailable: {e:#}");
                }
            }

            let new_size = state.queue_size()?;
            if new_size == 0 {
                info!("no new items after seeding, sleeping 1 hour");
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
            continue;
        }
        for item in batch {
            let permit = sem.clone().acquire_owned().await?;
            let client = client.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let message = match fetch(&client, &item).await {
                    Ok((payload, groups)) => WriterMessage::Write(Box::new(WriteJob {
                        item,
                        payload,
                        groups,
                    })),
                    Err(TmdbError::NotFound(path)) => {
                        info!("no item at {path}, skipping it");
                        WriterMessage::NotFound(item)
                    }
                    Err(e @ TmdbError::Unavailable(..)) => {
                        warn!("{e}");
                        // Give a struggling service room before this item returns.
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        WriterMessage::Unavailable(item)
                    }
                    Err(e) => {
                        exit_on_bad_key(&e);
                        warn!("{e}");
                        WriterMessage::Failed(item)
                    }
                };
                let _ = tx.send(message).await;
            });
        }
    }
}
