use crate::api::{TmdbClient, TmdbError};
use crate::id::TmdbType;
use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use tracing::{info, warn};

pub struct SeedItem {
    pub tmdb_id: i64,
    pub tmdb_type: TmdbType,
    pub popularity: f64,
}

struct Endpoint {
    path: &'static str,
    pages: u32,
    default_type: Option<TmdbType>,
}

const ENDPOINTS: &[Endpoint] = &[
    Endpoint {
        path: "trending/all/week",
        pages: 5,
        default_type: None,
    },
    Endpoint {
        path: "movie/popular",
        pages: 5,
        default_type: Some(TmdbType::Movie),
    },
    Endpoint {
        path: "movie/now_playing",
        pages: 3,
        default_type: Some(TmdbType::Movie),
    },
    Endpoint {
        path: "movie/upcoming",
        pages: 3,
        default_type: Some(TmdbType::Movie),
    },
    Endpoint {
        path: "tv/popular",
        pages: 5,
        default_type: Some(TmdbType::Tv),
    },
    Endpoint {
        path: "person/popular",
        pages: 3,
        default_type: Some(TmdbType::Person),
    },
];

fn parse_media_type(s: &str) -> Option<TmdbType> {
    match s {
        "movie" => Some(TmdbType::Movie),
        "tv" => Some(TmdbType::Tv),
        "person" => Some(TmdbType::Person),
        _ => None,
    }
}

pub async fn fetch_seeds(client: &TmdbClient) -> Result<Vec<SeedItem>> {
    let mut seen = HashSet::new();
    let mut items = Vec::new();

    for ep in ENDPOINTS {
        for page in 1..=ep.pages {
            let resp = match client.get_list(ep.path, page).await {
                Ok(r) => r,
                Err(e @ TmdbError::Unauthorized(_)) => return Err(e.into()),
                Err(e) => {
                    warn!("seed endpoint {} page {} failed: {:#}", ep.path, page, e);
                    break;
                }
            };
            for li in &resp.results {
                let tmdb_type = ep
                    .default_type
                    .or_else(|| li.media_type.as_deref().and_then(parse_media_type));
                let Some(tt) = tmdb_type else { continue };
                if seen.insert((li.id, tt)) {
                    items.push(SeedItem {
                        tmdb_id: li.id,
                        tmdb_type: tt,
                        popularity: li.popularity.unwrap_or(0.0),
                    });
                }
            }
            if resp.total_pages.is_some_and(|tp| page >= tp) {
                break;
            }
        }
    }

    info!(
        "seeder fetched {} unique items from TMDB lists",
        items.len()
    );
    Ok(items)
}

const EXPORTS: [(&str, TmdbType); 4] = [
    ("movie_ids", TmdbType::Movie),
    ("tv_series_ids", TmdbType::Tv),
    ("person_ids", TmdbType::Person),
    ("collection_ids", TmdbType::Collection),
];

/// Returns (month, day, year) for a day count since 1970.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (month, day, year)
}

/// TMDB publishes every ID each morning, so yesterday's files always exist.
pub fn export_date(now_unix: i64) -> String {
    let (month, day, year) = civil_from_days(now_unix / 86400 - 1);
    format!("{month:02}_{day:02}_{year}")
}

fn parse_export_line(line: &str, tmdb_type: TmdbType) -> Option<SeedItem> {
    let item: serde_json::Value = serde_json::from_str(line).ok()?;
    if item["adult"] == serde_json::Value::Bool(true) {
        return None;
    }
    Some(SeedItem {
        tmdb_id: item["id"].as_i64()?,
        tmdb_type,
        popularity: item["popularity"].as_f64().unwrap_or(0.0),
    })
}

/// Downloads TMDB's daily ID exports, which list every title, person, and collection.
pub async fn fetch_export_ids(date: &str) -> Result<Vec<SeedItem>> {
    let client = reqwest::Client::new();
    let mut items = Vec::new();
    for (name, tmdb_type) in EXPORTS {
        let url = format!("https://files.tmdb.org/p/exports/{name}_{date}.json.gz");
        let bytes = client
            .get(&url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .with_context(|| format!("download {url}"))?
            .bytes()
            .await
            .with_context(|| format!("read {url}"))?;
        let before = items.len();
        for line in BufReader::new(GzDecoder::new(&bytes[..])).lines() {
            let line = line.with_context(|| format!("decompress {url}"))?;
            items.extend(parse_export_line(&line, tmdb_type));
        }
        info!("TMDB export {name}: {} items", items.len() - before);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_date_is_yesterday_in_utc() {
        // 2026-10-31T12:00:00Z
        assert_eq!(export_date(1_793_448_000), "10_30_2026");
        // 2024-03-01T00:00:00Z, the day after a leap day.
        assert_eq!(export_date(1_709_251_200), "02_29_2024");
    }

    #[test]
    fn export_lines_skip_adult_items() {
        let movie = parse_export_line(
            r#"{"adult":false,"id":550,"original_title":"Fight Club","popularity":40.5,"video":false}"#,
            TmdbType::Movie,
        )
        .unwrap();
        assert_eq!(movie.tmdb_id, 550);
        assert_eq!(movie.popularity, 40.5);
        assert!(parse_export_line(r#"{"adult":true,"id":1}"#, TmdbType::Movie).is_none());
        assert!(parse_export_line("not json", TmdbType::Movie).is_none());
    }
}
