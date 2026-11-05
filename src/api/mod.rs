use anyhow::{Context, Result};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use tracing::warn;

#[derive(Deserialize)]
pub struct ListResponse {
    pub results: Vec<ListItem>,
    pub total_pages: Option<u32>,
}

#[derive(Deserialize)]
pub struct ListItem {
    pub id: i64,
    pub media_type: Option<String>,
    pub popularity: Option<f64>,
}

/// Why a TMDB request failed, which decides what the crawl does with the item.
#[derive(Debug, thiserror::Error)]
pub enum TmdbError {
    #[error("TMDB rejected the API key for {0}")]
    Unauthorized(String),
    #[error("TMDB has no item at {0}")]
    NotFound(String),
    /// A network failure or rate limiting, which says nothing about the item.
    #[error("TMDB is unavailable for {0}: {1}")]
    Unavailable(String, String),
    /// TMDB answered, but the item keeps failing.
    #[error("TMDB failed for {0}: {1}")]
    Failed(String, String),
}

const TMDB_BASE: &str = "https://api.themoviedb.org/3";
// These are the add-on's own endpoints, plus `similar` and `recommendations` for the crawl.
const APPEND_MOVIE: &str = "images,release_dates,external_ids,credits,videos,watch/providers,translations,similar,recommendations";
const APPEND_TV: &str = "images,external_ids,content_ratings,episode_groups,videos,watch/providers,aggregate_credits,translations,similar,recommendations";
const APPEND_SEASON: &str = "images,external_ids,watch/providers,aggregate_credits,translations";
const APPEND_EPISODE: &str = "images,external_ids,credits,translations";
const APPEND_PERSON: &str = "images,external_ids,movie_credits,tv_credits";
const APPEND_COLLECTION: &str = "images";
const RETRIES: u32 = 3;

#[derive(Clone)]
pub struct TmdbClient {
    client: Client,
    api_key: String,
    language: String,
}

impl TmdbClient {
    pub fn new(api_key: String, language: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(50)
            .build()
            .context("build HTTP client")?;
        Ok(Self {
            client,
            api_key,
            language,
        })
    }

    pub async fn get_movie(&self, id: i64) -> Result<Value, TmdbError> {
        self.get_item(&format!("movie/{id}"), APPEND_MOVIE).await
    }

    pub async fn get_tv(&self, id: i64) -> Result<Value, TmdbError> {
        self.get_item(&format!("tv/{id}"), APPEND_TV).await
    }

    pub async fn get_season(&self, tv_id: i64, season: i64) -> Result<Value, TmdbError> {
        self.get_item(&format!("tv/{tv_id}/season/{season}"), APPEND_SEASON)
            .await
    }

    pub async fn get_episode(
        &self,
        tv_id: i64,
        season: i64,
        episode: i64,
    ) -> Result<Value, TmdbError> {
        self.get_item(
            &format!("tv/{tv_id}/season/{season}/episode/{episode}"),
            APPEND_EPISODE,
        )
        .await
    }

    pub async fn get_person(&self, id: i64) -> Result<Value, TmdbError> {
        self.get_item(&format!("person/{id}"), APPEND_PERSON).await
    }

    pub async fn get_collection(&self, id: i64) -> Result<Value, TmdbError> {
        self.get_item(&format!("collection/{id}"), APPEND_COLLECTION)
            .await
    }

    pub async fn get_episode_group(&self, id: &str) -> Result<Value, TmdbError> {
        self.get(
            &format!("tv/episode_group/{id}"),
            &format!("language={}", self.language),
        )
        .await
    }

    pub async fn get_list(&self, path: &str, page: u32) -> Result<ListResponse, TmdbError> {
        let value = self.get(path, &format!("page={page}")).await?;
        serde_json::from_value(value).map_err(|e| TmdbError::Failed(path.into(), e.to_string()))
    }

    /// Loads TMDB's genre names, which credit and collection lists give only as IDs.
    pub async fn get_genres(&self) -> Result<HashMap<i64, String>, TmdbError> {
        let mut genres = HashMap::new();
        for path in ["genre/movie/list", "genre/tv/list"] {
            let value = self
                .get(path, &format!("language={}", self.language))
                .await?;
            for genre in value["genres"].as_array().into_iter().flatten() {
                if let (Some(id), Some(name)) = (genre["id"].as_i64(), genre["name"].as_str()) {
                    genres.insert(id, name.to_owned());
                }
            }
        }
        Ok(genres)
    }

    /// Makes one request, so a bad key stops the crawl at startup.
    pub async fn check_key(&self) -> Result<(), TmdbError> {
        self.get("configuration", "").await.map(|_| ())
    }

    async fn get_item(&self, path: &str, append: &str) -> Result<Value, TmdbError> {
        // Asks for the images the add-on asks for: the UI language, no language, and English.
        let iso = self.language.get(..2).unwrap_or("en");
        let query = format!(
            "language={}&append_to_response={append}&include_image_language={iso},null,en&include_video_language={iso},null,en",
            self.language
        );
        self.get(path, &query).await
    }

    async fn get(&self, path: &str, query: &str) -> Result<Value, TmdbError> {
        let url = format!("{TMDB_BASE}/{path}?api_key={}&{query}", self.api_key);
        let mut last = String::new();
        for attempt in 0..RETRIES {
            let base_backoff = Duration::from_millis(500 * (1 << attempt));
            let resp = match self.client.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    // The URL holds the API key, so it must never reach a log.
                    last = e.without_url().to_string();
                    warn!(
                        "TMDB request for {path} failed, retry {}: {last}",
                        attempt + 1
                    );
                    tokio::time::sleep(base_backoff).await;
                    continue;
                }
            };
            let status = resp.status();
            if status.is_success() {
                return resp
                    .json::<Value>()
                    .await
                    .map_err(|e| TmdbError::Failed(path.into(), e.without_url().to_string()));
            }
            match status {
                StatusCode::UNAUTHORIZED => return Err(TmdbError::Unauthorized(path.into())),
                StatusCode::NOT_FOUND => return Err(TmdbError::NotFound(path.into())),
                StatusCode::TOO_MANY_REQUESTS => {
                    let retry_after = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(Duration::from_secs);
                    last = status.to_string();
                    tokio::time::sleep(retry_after.map_or(base_backoff, |r| r.max(base_backoff)))
                        .await;
                }
                s if s.is_server_error() => {
                    warn!("TMDB {s} for {path}, retry {}", attempt + 1);
                    tokio::time::sleep(base_backoff).await;
                    // An outage, not a bad item, so the item goes back in the queue.
                    if attempt + 1 == RETRIES {
                        return Err(TmdbError::Unavailable(path.into(), s.to_string()));
                    }
                }
                s => return Err(TmdbError::Failed(path.into(), s.to_string())),
            }
        }
        Err(TmdbError::Unavailable(path.into(), last))
    }
}
