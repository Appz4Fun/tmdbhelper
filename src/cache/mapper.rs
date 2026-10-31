//! Writes TMDB responses the way TMDBHelper's `mappings.py` and its factories do.

use crate::cache::{now_unix, WriteContext, DATALEVEL_FULL, DAY, DEFAULT_EXPIRY, SHORTER_EXPIRY};
use crate::id::{build_episode_id, build_season_id};
use anyhow::Result;
use rusqlite::types::Value as Sql;
use rusqlite::{params_from_iter, Connection};
use serde_json::Value;

const ART_TYPES: [&str; 5] = ["backdrops", "posters", "stills", "logos", "profiles"];
const RELEASE_TYPES: [&str; 6] = [
    "Premiere",
    "Limited",
    "Theatrical",
    "Digital",
    "Physical",
    "TV",
];

/// `get_blanks_none`: an empty string or a missing value becomes NULL.
fn sql(v: &Value) -> Sql {
    match v {
        Value::String(s) if !s.is_empty() => Sql::Text(s.clone()),
        Value::Number(n) => n
            .as_i64()
            .map(Sql::Integer)
            .or_else(|| n.as_f64().map(Sql::Real))
            .unwrap_or(Sql::Null),
        Value::Bool(b) => Sql::Integer(*b as i64),
        _ => Sql::Null,
    }
}

fn text(s: &str) -> Sql {
    Sql::Text(s.to_owned())
}

fn array(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

fn year(v: &Value) -> Sql {
    v.as_str()
        .and_then(|d| d.get(0..4))
        .and_then(|y| y.parse().ok())
        .map_or(Sql::Null, Sql::Integer)
}

fn runtime_seconds(v: &Value) -> i64 {
    let minutes = match v {
        Value::Array(a) => a.first().and_then(Value::as_i64),
        other => other.as_i64(),
    };
    minutes.unwrap_or(0) * 60
}

fn extension(path: &str) -> Sql {
    path.rsplit('.').next().map_or(Sql::Null, text)
}

fn insert_sql(table: &str, conflict: &str, keys: &[&str], assign: &str) -> String {
    let marks = vec!["?"; keys.len()].join(", ");
    let conflict_keys: Vec<&str> = conflict.split(", ").collect();
    let updates: Vec<String> = keys
        .iter()
        .filter(|k| !conflict_keys.contains(k))
        .map(|k| assign.replace("{t}", table).replace("{k}", k))
        .collect();
    let action = if updates.is_empty() {
        "NOTHING".to_owned()
    } else {
        format!("UPDATE SET {}", updates.join(", "))
    };
    format!(
        "INSERT INTO {table}({}) VALUES ({marks}) ON CONFLICT({conflict}) DO {action}",
        keys.join(", ")
    )
}

/// `set_or_update_null_list_values`: keeps existing values and fills in NULLs.
fn fill(conn: &Connection, table: &str, conflict: &str, row: &[(&str, Sql)]) -> Result<()> {
    let keys: Vec<&str> = row.iter().map(|(k, _)| *k).collect();
    let statement = insert_sql(
        table,
        conflict,
        &keys,
        "{k}=COALESCE({t}.{k}, excluded.{k})",
    );
    conn.prepare_cached(&statement)?
        .execute(params_from_iter(row.iter().map(|(_, v)| v)))?;
    Ok(())
}

/// Writes an item's own row from a full response. New values win, so a refresh
/// picks up changes, and a value only a stub had survives.
fn store(conn: &Connection, table: &str, row: &[(&str, Sql)]) -> Result<()> {
    let keys: Vec<&str> = row.iter().map(|(k, _)| *k).collect();
    let statement = insert_sql(table, "id", &keys, "{k}=COALESCE(excluded.{k}, {t}.{k})");
    conn.prepare_cached(&statement)?
        .execute(params_from_iter(row.iter().map(|(_, v)| v)))?;
    Ok(())
}

/// Civil date to days since 1970, for comparing air dates.
fn days_from_date(date: &str) -> Option<i64> {
    let y: i64 = date.get(0..4)?.parse().ok()?;
    let m: i64 = date.get(5..7)?.parse().ok()?;
    let d: i64 = date.get(8..10)?.parse().ok()?;
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

/// `set_unaired_expiry`: unaired items refresh sooner, closer to their premiere.
fn expiry_seconds(default: i64, premiered: &Value, next_episode: &Value) -> i64 {
    let Some(air_day) = premiered.as_str().and_then(days_from_date) else {
        return SHORTER_EXPIRY;
    };
    let days_to_air = air_day - now_unix() / DAY;
    if days_to_air > 0 {
        return (days_to_air / 2 + 1) * DAY;
    }
    if next_episode.is_object() {
        return SHORTER_EXPIRY;
    }
    default
}

fn base_full(
    conn: &Connection,
    ctx: &WriteContext,
    id: &str,
    mediatype: &str,
    expiry_in: i64,
) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO baseitem (id, mediatype, expiry, datalevel, fanart_tv, translation, language)
         VALUES (?1, ?2, ?3, ?4, 0, 1, ?5)
         ON CONFLICT(id) DO UPDATE SET mediatype=excluded.mediatype, expiry=excluded.expiry,
             datalevel=excluded.datalevel, fanart_tv=excluded.fanart_tv,
             translation=excluded.translation, language=excluded.language",
    )?
    .execute(rusqlite::params![
        id,
        mediatype,
        now_unix() + expiry_in,
        DATALEVEL_FULL,
        &ctx.language
    ])?;
    Ok(())
}

/// A stub has an expiry of 0, so the add-on fetches it in full if a crawl hasn't.
fn base_stub(conn: &Connection, ctx: &WriteContext, id: &str, mediatype: &str) -> Result<()> {
    conn.prepare_cached(
        "INSERT OR IGNORE INTO baseitem (id, mediatype, expiry, language) VALUES (?1, ?2, 0, ?3)",
    )?
    .execute(rusqlite::params![id, mediatype, &ctx.language])?;
    Ok(())
}

fn default_art(conn: &Connection, parent: &str, path: &str, art_type: &str) -> Result<()> {
    fill(
        conn,
        "default_art",
        "type, parent_id",
        &[
            ("icon", text(path)),
            ("type", text(art_type)),
            ("parent_id", text(parent)),
        ],
    )?;
    fill(
        conn,
        "art",
        "icon, type, parent_id",
        &[
            ("icon", text(path)),
            ("type", text(art_type)),
            ("extension", extension(path)),
            ("parent_id", text(parent)),
        ],
    )
}

fn default_art_of(conn: &Connection, parent: &str, v: &Value) -> Result<()> {
    for (key, art_type) in [
        ("poster_path", "posters"),
        ("backdrop_path", "backdrops"),
        ("profile_path", "profiles"),
    ] {
        if let Some(path) = v[key].as_str().filter(|p| !p.is_empty()) {
            default_art(conn, parent, path, art_type)?;
        }
    }
    Ok(())
}

fn aspect_ratio_bucket(ratio: f64) -> i64 {
    if ratio < 1.0 {
        1
    } else if ratio == 1.0 {
        2
    } else if (1.7..=1.8).contains(&ratio) {
        4
    } else if ratio < 1.7 {
        3
    } else {
        5
    }
}

fn images(conn: &Connection, parent: &str, v: &Value) -> Result<()> {
    for art_type in ART_TYPES {
        for art in array(&v["images"][art_type]) {
            let Some(path) = art["file_path"].as_str().filter(|p| !p.is_empty()) else {
                continue;
            };
            let quality =
                art["width"].as_i64().unwrap_or(0) * art["height"].as_i64().unwrap_or(0) / 200_000;
            fill(
                conn,
                "art",
                "icon, type, parent_id",
                &[
                    (
                        "aspect_ratio",
                        art["aspect_ratio"]
                            .as_f64()
                            .map_or(Sql::Null, |r| Sql::Integer(aspect_ratio_bucket(r))),
                    ),
                    ("quality", Sql::Integer(quality)),
                    ("iso_language", sql(&art["iso_639_1"])),
                    ("iso_country", sql(&art["iso_3166_1"])),
                    ("icon", text(path)),
                    ("type", text(art_type)),
                    ("extension", extension(path)),
                    (
                        "rating",
                        Sql::Integer((art["vote_average"].as_f64().unwrap_or(0.0) * 100.0) as i64),
                    ),
                    ("votes", sql(&art["vote_count"])),
                    ("parent_id", text(parent)),
                ],
            )?;
        }
    }
    Ok(())
}

fn custom(conn: &Connection, parent: &str, key: &str, value: Sql) -> Result<()> {
    fill(
        conn,
        "custom",
        "key, parent_id",
        &[
            ("key", text(key)),
            ("value", value),
            ("parent_id", text(parent)),
        ],
    )
}

fn money(v: &Value) -> Option<Sql> {
    let amount = v.as_i64()?;
    let digits = amount.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    Some(Sql::Text(format!("${grouped}")))
}

fn customs(conn: &Connection, parent: &str, v: &Value) -> Result<()> {
    for key in ["budget", "revenue"] {
        if let Some(value) = money(&v[key]) {
            custom(conn, parent, key, value)?;
        }
    }
    for key in ["original_language", "homepage"] {
        if v[key].as_str().is_some_and(|s| !s.is_empty()) {
            custom(conn, parent, key, sql(&v[key]))?;
        }
    }
    Ok(())
}

/// `get_unique_ids`: the add-on keeps every external ID, with "None" for a null one.
fn unique_ids(conn: &Connection, parent: &str, v: &Value) -> Result<()> {
    let Some(ids) = v["external_ids"].as_object() else {
        return Ok(());
    };
    for (key, value) in ids {
        let key = if key == "id" {
            "tmdb".to_owned()
        } else {
            key.replace("_id", "")
        };
        let value = match value {
            Value::Null => text("None"),
            Value::String(s) if s.is_empty() => Sql::Null,
            Value::String(s) => text(s),
            other => Sql::Text(other.to_string()),
        };
        fill(
            conn,
            "unique_id",
            "key, parent_id",
            &[
                ("key", text(&key)),
                ("value", value),
                ("parent_id", text(parent)),
            ],
        )?;
    }
    Ok(())
}

fn genres(conn: &Connection, parent: &str, v: &Value) -> Result<()> {
    for g in array(&v["genres"]) {
        fill(
            conn,
            "genre",
            "tmdb_id, parent_id",
            &[
                ("name", sql(&g["name"])),
                ("tmdb_id", sql(&g["id"])),
                ("parent_id", text(parent)),
            ],
        )?;
    }
    Ok(())
}

fn languages(conn: &Connection, parent: &str, v: &Value) -> Result<()> {
    for l in array(&v["spoken_languages"]) {
        fill(
            conn,
            "languages",
            "iso_language",
            &[
                ("name", sql(&l["name"])),
                ("english_name", sql(&l["english_name"])),
                ("iso_language", sql(&l["iso_639_1"])),
            ],
        )?;
        fill(
            conn,
            "language",
            "iso_language, parent_id",
            &[
                ("iso_language", sql(&l["iso_639_1"])),
                ("parent_id", text(parent)),
            ],
        )?;
    }
    Ok(())
}

fn countries(conn: &Connection, parent: &str, v: &Value) -> Result<()> {
    for c in array(&v["production_countries"]) {
        fill(
            conn,
            "countries",
            "iso_country",
            &[
                ("name", sql(&c["name"])),
                ("iso_country", sql(&c["iso_3166_1"])),
            ],
        )?;
        fill(
            conn,
            "country",
            "iso_country, parent_id",
            &[
                ("iso_country", sql(&c["iso_3166_1"])),
                ("parent_id", text(parent)),
            ],
        )?;
    }
    Ok(())
}

fn organizations(
    conn: &Connection,
    parent: &str,
    items: &Value,
    owner: &str,
    link: &str,
) -> Result<()> {
    for c in array(items) {
        fill(
            conn,
            owner,
            "tmdb_id",
            &[
                ("tmdb_id", sql(&c["id"])),
                ("name", sql(&c["name"])),
                ("logo", sql(&c["logo_path"])),
                ("country", sql(&c["origin_country"])),
            ],
        )?;
        fill(
            conn,
            link,
            "tmdb_id, parent_id",
            &[("tmdb_id", sql(&c["id"])), ("parent_id", text(parent))],
        )?;
    }
    Ok(())
}

fn providers(conn: &Connection, parent: &str, v: &Value) -> Result<()> {
    let Some(results) = v["watch/providers"]["results"].as_object() else {
        return Ok(());
    };
    for (iso, availabilities) in results {
        let Some(availabilities) = availabilities.as_object() else {
            continue;
        };
        for (availability, list) in availabilities {
            if availability == "link" {
                continue;
            }
            for p in array(list) {
                fill(
                    conn,
                    "service",
                    "tmdb_id",
                    &[
                        ("tmdb_id", sql(&p["provider_id"])),
                        ("display_priority", sql(&p["display_priority"])),
                        ("name", sql(&p["provider_name"])),
                        ("logo", sql(&p["logo_path"])),
                    ],
                )?;
                fill(
                    conn,
                    "provider",
                    "tmdb_id, iso_country, parent_id",
                    &[
                        ("tmdb_id", sql(&p["provider_id"])),
                        ("availability", text(availability)),
                        ("iso_country", text(iso)),
                        ("parent_id", text(parent)),
                    ],
                )?;
            }
        }
    }
    Ok(())
}

fn translations(conn: &Connection, parent: &str, v: &Value) -> Result<()> {
    // NULLs never conflict in a UNIQUE constraint, so clear the rows a refresh replaces.
    conn.prepare_cached("DELETE FROM translation WHERE parent_id=?1")?
        .execute([parent])?;
    for t in array(&v["translations"]["translations"]) {
        let data = &t["data"];
        let title = if data["title"].as_str().is_some_and(|s| !s.is_empty()) {
            &data["title"]
        } else {
            &data["name"]
        };
        fill(
            conn,
            "translation",
            "iso_country, iso_language, parent_id",
            &[
                ("iso_country", sql(&t["iso_3166_1"])),
                ("iso_language", sql(&t["iso_639_1"])),
                ("plot", sql(&data["overview"])),
                ("title", sql(title)),
                ("tagline", sql(&data["tagline"])),
                ("parent_id", text(parent)),
            ],
        )?;
    }
    Ok(())
}

fn videos(conn: &Connection, parent: &str, v: &Value) -> Result<()> {
    for video in array(&v["videos"]["results"]) {
        if video["site"] != "YouTube" {
            continue;
        }
        let Some(key) = video["key"].as_str() else {
            continue;
        };
        fill(
            conn,
            "video",
            "path, parent_id",
            &[
                ("name", sql(&video["name"])),
                ("iso_country", sql(&video["iso_3166_1"])),
                ("iso_language", sql(&video["iso_639_1"])),
                ("release_date", sql(&video["published_at"])),
                ("key", text(key)),
                (
                    "path",
                    Sql::Text(format!(
                        "plugin://plugin.video.youtube/play/?video_id={key}"
                    )),
                ),
                ("content", sql(&video["type"])),
                ("parent_id", text(parent)),
            ],
        )?;
    }
    Ok(())
}

fn person_stub(conn: &Connection, ctx: &WriteContext, i: &Value) -> Result<()> {
    let Some(tmdb_id) = i["id"].as_i64() else {
        return Ok(());
    };
    let id = format!("person.{tmdb_id}");
    base_stub(conn, ctx, &id, "person")?;
    fill(
        conn,
        "person",
        "id",
        &[
            ("id", text(&id)),
            ("tmdb_id", Sql::Integer(tmdb_id)),
            ("name", sql(&i["name"])),
            ("gender", sql(&i["gender"])),
            ("known_for_department", sql(&i["known_for_department"])),
        ],
    )?;
    default_art_of(conn, &id, i)
}

fn first<'a>(j: &'a Value, i: &'a Value, key: &str) -> &'a Value {
    match &j[key] {
        Value::Null => &i[key],
        Value::String(s) if s.is_empty() => &i[key],
        found => found,
    }
}

/// `get_credits_data`: aggregate credits nest each role or job inside the person.
fn credits(
    conn: &Connection,
    ctx: &WriteContext,
    parent: &str,
    items: &Value,
    aggregate: bool,
) -> Result<()> {
    conn.prepare_cached("DELETE FROM castmember WHERE parent_id=?1")?
        .execute([parent])?;
    conn.prepare_cached("DELETE FROM crewmember WHERE parent_id=?1")?
        .execute([parent])?;
    for (subkey, nested, is_cast, guest) in [
        ("cast", "roles", true, false),
        ("crew", "jobs", false, false),
        ("guest_stars", "roles", true, true),
    ] {
        for i in array(&items[subkey]) {
            person_stub(conn, ctx, i)?;
            let entries: Vec<&Value> = if aggregate {
                array(&i[nested]).iter().collect()
            } else {
                vec![i]
            };
            for j in entries {
                if j["credit_id"].is_null() {
                    continue;
                }
                let appearances = sql(first(j, i, "total_episode_count"));
                if is_cast {
                    fill(
                        conn,
                        "castmember",
                        "tmdb_id, role, parent_id",
                        &[
                            ("tmdb_id", sql(&i["id"])),
                            ("role", sql(first(j, i, "character"))),
                            ("ordering", sql(first(j, i, "order"))),
                            ("appearances", appearances),
                            ("guest", if guest { Sql::Integer(1) } else { Sql::Null }),
                            ("parent_id", text(parent)),
                        ],
                    )?;
                } else {
                    fill(
                        conn,
                        "crewmember",
                        "tmdb_id, role, department, parent_id",
                        &[
                            ("tmdb_id", sql(&i["id"])),
                            ("role", sql(first(j, i, "job"))),
                            ("department", sql(first(j, i, "department"))),
                            ("appearances", appearances),
                            ("parent_id", text(parent)),
                        ],
                    )?;
                }
            }
        }
    }
    Ok(())
}

/// `get_media_item_data`: the short form of a title from a credit or collection list.
fn media_stub(conn: &Connection, ctx: &WriteContext, i: &Value, movie: bool) -> Result<()> {
    let Some(tmdb_id) = i["id"].as_i64() else {
        return Ok(());
    };
    let (table, mediatype, premiered, title, prefix) = if movie {
        ("movie", "movie", "release_date", "title", "movie")
    } else {
        ("tvshow", "tvshow", "first_air_date", "name", "tv")
    };
    let id = format!("{prefix}.{tmdb_id}");
    base_stub(conn, ctx, &id, mediatype)?;
    fill(
        conn,
        table,
        "id",
        &[
            ("id", text(&id)),
            ("tmdb_id", Sql::Integer(tmdb_id)),
            ("year", year(&i[premiered])),
            ("premiered", sql(&i[premiered])),
            ("plot", sql(&i["overview"])),
            ("title", sql(&i[title])),
            ("originaltitle", sql(&i["original_title"])),
            ("rating", sql(&i["vote_average"])),
            ("votes", sql(&i["vote_count"])),
            ("popularity", sql(&i["popularity"])),
        ],
    )?;
    for genre_id in array(&i["genre_ids"]).iter().filter_map(Value::as_i64) {
        if let Some(name) = ctx.genres.get(&genre_id) {
            fill(
                conn,
                "genre",
                "tmdb_id, parent_id",
                &[
                    ("name", text(name)),
                    ("tmdb_id", Sql::Integer(genre_id)),
                    ("parent_id", text(&id)),
                ],
            )?;
        }
    }
    default_art_of(conn, &id, i)
}

fn episode_type(i: &Value) -> Sql {
    let season = i["season_number"].as_i64();
    let episode = i["episode_number"].as_i64();
    Sql::Text(
        if season == Some(0) {
            "special"
        } else if episode == Some(1) {
            if season == Some(1) {
                "series_premiere"
            } else {
                "season_premiere"
            }
        } else if i["episode_type"] == "finale" {
            "season_finale"
        } else if i["episode_type"] == "mid_season" {
            "mid_season_finale"
        } else {
            "standard"
        }
        .to_owned(),
    )
}

fn episode_stub(
    conn: &Connection,
    ctx: &WriteContext,
    tv_id: i64,
    i: &Value,
    status: Option<&str>,
) -> Result<()> {
    let (Some(season), Some(episode)) = (i["season_number"].as_i64(), i["episode_number"].as_i64())
    else {
        return Ok(());
    };
    let id = build_episode_id(tv_id, season, episode);
    let season_id = build_season_id(tv_id, season);
    let tvshow_id = format!("tv.{tv_id}");
    base_stub(conn, ctx, &season_id, "season")?;
    fill(
        conn,
        "season",
        "id",
        &[
            ("id", text(&season_id)),
            ("season", Sql::Integer(season)),
            ("tvshow_id", text(&tvshow_id)),
        ],
    )?;
    base_stub(conn, ctx, &id, "episode")?;
    fill(
        conn,
        "episode",
        "id",
        &[
            ("id", text(&id)),
            ("episode", Sql::Integer(episode)),
            ("year", year(&i["air_date"])),
            ("premiered", sql(&i["air_date"])),
            ("title", sql(&i["name"])),
            ("plot", sql(&i["overview"])),
            ("rating", sql(&i["vote_average"])),
            ("votes", sql(&i["vote_count"])),
            ("status", status.map_or_else(|| episode_type(i), text)),
            ("duration", Sql::Integer(runtime_seconds(&i["runtime"]))),
            ("season_id", text(&season_id)),
            ("tvshow_id", text(&tvshow_id)),
        ],
    )?;
    if let Some(still) = i["still_path"].as_str().filter(|p| !p.is_empty()) {
        fill(
            conn,
            "art",
            "icon, type, parent_id",
            &[
                ("aspect_ratio", Sql::Integer(4)),
                ("icon", text(still)),
                ("type", text("stills")),
                ("extension", extension(still)),
                ("parent_id", text(&id)),
            ],
        )?;
    }
    Ok(())
}

fn season_stub(conn: &Connection, ctx: &WriteContext, tv_id: i64, i: &Value) -> Result<()> {
    let Some(season) = i["season_number"].as_i64() else {
        return Ok(());
    };
    let id = build_season_id(tv_id, season);
    base_stub(conn, ctx, &id, "season")?;
    fill(
        conn,
        "season",
        "id",
        &[
            ("id", text(&id)),
            ("season", Sql::Integer(season)),
            ("episode_count", sql(&i["episode_count"])),
            ("year", year(&i["air_date"])),
            ("premiered", sql(&i["air_date"])),
            ("title", sql(&i["name"])),
            ("plot", sql(&i["overview"])),
            ("rating", sql(&i["vote_average"])),
            ("tvshow_id", text(&format!("tv.{tv_id}"))),
        ],
    )?;
    default_art_of(conn, &id, i)
}

pub fn write_movie(conn: &Connection, ctx: &WriteContext, m: &Value) -> Result<()> {
    let tmdb_id = m["id"].as_i64().unwrap_or_default();
    let id = format!("movie.{tmdb_id}");
    let expiry = expiry_seconds(DEFAULT_EXPIRY, &m["release_date"], &Value::Null);
    base_full(conn, ctx, &id, "movie", expiry)?;
    store(
        conn,
        "movie",
        &[
            ("id", text(&id)),
            ("tmdb_id", Sql::Integer(tmdb_id)),
            ("year", year(&m["release_date"])),
            ("plot", sql(&m["overview"])),
            ("title", sql(&m["title"])),
            ("originaltitle", sql(&m["original_title"])),
            ("duration", Sql::Integer(runtime_seconds(&m["runtime"]))),
            ("tagline", sql(&m["tagline"])),
            ("premiered", sql(&m["release_date"])),
            ("status", sql(&m["status"])),
            ("rating", sql(&m["vote_average"])),
            ("votes", sql(&m["vote_count"])),
            ("popularity", sql(&m["popularity"])),
        ],
    )?;
    genres(conn, &id, m)?;
    languages(conn, &id, m)?;
    countries(conn, &id, m)?;
    certifications_movie(conn, &id, m)?;
    translations(conn, &id, m)?;
    videos(conn, &id, m)?;
    organizations(conn, &id, &m["production_companies"], "company", "studio")?;
    providers(conn, &id, m)?;
    credits(conn, ctx, &id, &m["credits"], false)?;
    unique_ids(conn, &id, m)?;
    customs(conn, &id, m)?;
    default_art_of(conn, &id, m)?;
    images(conn, &id, m)?;
    let collection = &m["belongs_to_collection"];
    if let Some(collection_tmdb_id) = collection["id"].as_i64() {
        let collection_id = format!("collection.{collection_tmdb_id}");
        base_stub(conn, ctx, &collection_id, "set")?;
        fill(
            conn,
            "collection",
            "id",
            &[
                ("id", text(&collection_id)),
                ("tmdb_id", Sql::Integer(collection_tmdb_id)),
                ("title", sql(&collection["name"])),
            ],
        )?;
        // `get_belongs_to_collection` gives the collection's art to the movie.
        default_art_of(conn, &id, collection)?;
        belongs(conn, &id, &collection_id)?;
    }
    Ok(())
}

fn belongs(conn: &Connection, id: &str, parent: &str) -> Result<()> {
    fill(
        conn,
        "belongs",
        "id, parent_id",
        &[("id", text(id)), ("parent_id", text(parent))],
    )
}

fn certifications_movie(conn: &Connection, parent: &str, m: &Value) -> Result<()> {
    conn.prepare_cached("DELETE FROM certification WHERE parent_id=?1")?
        .execute([parent])?;
    for country in array(&m["release_dates"]["results"]) {
        for r in array(&country["release_dates"]) {
            let release_type = r["type"]
                .as_i64()
                .and_then(|t| RELEASE_TYPES.get((t - 1) as usize))
                .map_or(Sql::Null, |t| text(t));
            fill(
                conn,
                "certification",
                "iso_country, iso_language, release_date, release_type, parent_id",
                &[
                    ("name", sql(&r["certification"])),
                    ("iso_country", sql(&country["iso_3166_1"])),
                    ("iso_language", sql(&r["iso_639_1"])),
                    ("release_date", sql(&r["release_date"])),
                    ("release_type", release_type),
                    ("parent_id", text(parent)),
                ],
            )?;
        }
    }
    Ok(())
}

/// Writes a show and the episode groups fetched alongside it.
pub fn write_tv(conn: &Connection, ctx: &WriteContext, t: &Value, groups: &[Value]) -> Result<()> {
    let tmdb_id = t["id"].as_i64().unwrap_or_default();
    let id = format!("tv.{tmdb_id}");
    let next = &t["next_episode_to_air"];
    let last = &t["last_episode_to_air"];
    let expiry = expiry_seconds(DEFAULT_EXPIRY, &t["first_air_date"], next);
    base_full(conn, ctx, &id, "tvshow", expiry)?;
    let episode_id = |e: &Value| match (e["season_number"].as_i64(), e["episode_number"].as_i64()) {
        (Some(s), Some(n)) => Sql::Text(build_episode_id(tmdb_id, s, n)),
        _ => Sql::Null,
    };
    // A show without a run time uses its latest episode's run time.
    let duration = [&t["episode_run_time"], &last["runtime"], &next["runtime"]]
        .into_iter()
        .map(runtime_seconds)
        .find(|d| *d > 0)
        .map_or(Sql::Null, Sql::Integer);
    store(
        conn,
        "tvshow",
        &[
            ("id", text(&id)),
            ("tmdb_id", Sql::Integer(tmdb_id)),
            ("year", year(&t["first_air_date"])),
            ("plot", sql(&t["overview"])),
            ("title", sql(&t["name"])),
            ("originaltitle", sql(&t["original_name"])),
            ("duration", duration),
            ("tagline", sql(&t["tagline"])),
            ("premiered", sql(&t["first_air_date"])),
            ("status", sql(&t["status"])),
            ("rating", sql(&t["vote_average"])),
            ("votes", sql(&t["vote_count"])),
            ("popularity", sql(&t["popularity"])),
            ("next_episode_to_air_id", episode_id(next)),
            ("last_episode_to_air_id", episode_id(last)),
            ("totalseasons", sql(&t["number_of_seasons"])),
            ("totalepisodes", sql(&t["number_of_episodes"])),
        ],
    )?;
    let finale =
        (t["in_production"] != Value::Bool(true) && next.is_null()).then_some("series_finale");
    for episode in [last, next] {
        if episode.is_object() {
            episode_stub(conn, ctx, tmdb_id, episode, finale)?;
        }
    }
    for season in array(&t["seasons"]) {
        season_stub(conn, ctx, tmdb_id, season)?;
    }
    genres(conn, &id, t)?;
    languages(conn, &id, t)?;
    countries(conn, &id, t)?;
    conn.prepare_cached("DELETE FROM certification WHERE parent_id=?1")?
        .execute([&id])?;
    for rating in array(&t["content_ratings"]["results"]) {
        fill(
            conn,
            "certification",
            "iso_country, iso_language, release_date, release_type, parent_id",
            &[
                ("name", sql(&rating["rating"])),
                ("iso_country", sql(&rating["iso_3166_1"])),
                ("parent_id", text(&id)),
            ],
        )?;
    }
    translations(conn, &id, t)?;
    videos(conn, &id, t)?;
    organizations(conn, &id, &t["production_companies"], "company", "studio")?;
    organizations(conn, &id, &t["networks"], "broadcaster", "network")?;
    providers(conn, &id, t)?;
    credits(conn, ctx, &id, &t["aggregate_credits"], true)?;
    for creator in array(&t["created_by"]) {
        person_stub(conn, ctx, creator)?;
        fill(
            conn,
            "crewmember",
            "tmdb_id, role, department, parent_id",
            &[
                ("tmdb_id", sql(&creator["id"])),
                ("role", text("Creator")),
                ("department", text("Creator")),
                ("parent_id", text(&id)),
            ],
        )?;
    }
    unique_ids(conn, &id, t)?;
    customs(conn, &id, t)?;
    default_art_of(conn, &id, t)?;
    images(conn, &id, t)?;
    for group in array(&t["episode_groups"]["results"]) {
        let Some(group_tmdb_id) = group["id"].as_str() else {
            continue;
        };
        let group_id = format!("episode_group.{group_tmdb_id}");
        base_stub(conn, ctx, &group_id, "episode_group")?;
        fill(
            conn,
            "episode_groups",
            "id",
            &[
                ("id", text(&group_id)),
                ("tmdb_id", text(group_tmdb_id)),
                ("tvshow_id", text(&id)),
                ("plot", sql(&group["description"])),
                ("title", sql(&group["name"])),
                ("type", sql(&group["type"])),
            ],
        )?;
    }
    for group in groups {
        write_episode_group(conn, ctx, tmdb_id, group)?;
    }
    Ok(())
}

/// `get_episode_group_seasons`: a group's seasons, and every episode they list.
fn write_episode_group(conn: &Connection, ctx: &WriteContext, tv_id: i64, g: &Value) -> Result<()> {
    let Some(group_tmdb_id) = g["id"].as_str() else {
        return Ok(());
    };
    let tvshow_id = format!("tv.{tv_id}");
    let id = format!("episode_group.{group_tmdb_id}");
    base_full(conn, ctx, &id, "episode_group", DEFAULT_EXPIRY)?;
    store(
        conn,
        "episode_groups",
        &[
            ("id", text(&id)),
            ("tmdb_id", text(group_tmdb_id)),
            ("tvshow_id", text(&tvshow_id)),
            ("plot", sql(&g["description"])),
            ("title", sql(&g["name"])),
            ("type", sql(&g["type"])),
        ],
    )?;
    for group in array(&g["groups"]) {
        let Some(season_group) = group["id"].as_str() else {
            continue;
        };
        let item_id = format!("episode_group_season.{season_group}");
        base_stub(conn, ctx, &item_id, "episode_group_season")?;
        let plot = if group["description"].is_null() {
            &g["description"]
        } else {
            &group["description"]
        };
        let group_type = if group["type"].is_null() {
            &g["type"]
        } else {
            &group["type"]
        };
        store(
            conn,
            "episode_group_seasons",
            &[
                ("id", text(&item_id)),
                ("tmdb_id", text(season_group)),
                ("tvshow_id", text(&tvshow_id)),
                ("group_id", text(group_tmdb_id)),
                ("plot", sql(plot)),
                ("title", sql(&group["name"])),
                ("ordering", sql(&group["order"])),
                ("type", sql(group_type)),
            ],
        )?;
        for episode in array(&group["episodes"]) {
            let (Some(season), Some(number)) = (
                episode["season_number"].as_i64(),
                episode["episode_number"].as_i64(),
            ) else {
                continue;
            };
            // Stubs first, because the group rows below refer to them.
            season_stub(
                conn,
                ctx,
                tv_id,
                &serde_json::json!({ "season_number": season }),
            )?;
            episode_stub(conn, ctx, tv_id, episode, None)?;
            let episode_id = build_episode_id(tv_id, season, number);
            fill(
                conn,
                "episode_group_season_episodes",
                "id, season_group_id",
                &[
                    ("id", text(&episode_id)),
                    ("tmdb_id", sql(&episode["id"])),
                    ("tvshow_id", text(&tvshow_id)),
                    ("season_group_id", text(season_group)),
                    ("group_id", text(group_tmdb_id)),
                    ("season", Sql::Integer(season)),
                    ("episode", Sql::Integer(number)),
                    ("ordering", sql(&episode["order"])),
                ],
            )?;
            let episode_tmdb_id = episode["id"].as_i64().unwrap_or_default();
            for (key, source) in [("title", "name"), ("plot", "overview")] {
                custom(
                    conn,
                    &item_id,
                    &format!("episode.{episode_tmdb_id}.{key}"),
                    sql(&episode[source]),
                )?;
            }
        }
    }
    Ok(())
}

pub fn write_season(
    conn: &Connection,
    ctx: &WriteContext,
    tv_id: i64,
    season: i64,
    s: &Value,
) -> Result<()> {
    let id = build_season_id(tv_id, season);
    let expiry = expiry_seconds(SHORTER_EXPIRY, &s["air_date"], &Value::Null);
    base_full(conn, ctx, &id, "season", expiry)?;
    let episodes = array(&s["episodes"]);
    store(
        conn,
        "season",
        &[
            ("id", text(&id)),
            ("season", Sql::Integer(season)),
            ("episode_count", Sql::Integer(episodes.len() as i64)),
            ("year", year(&s["air_date"])),
            ("plot", sql(&s["overview"])),
            ("title", sql(&s["name"])),
            ("premiered", sql(&s["air_date"])),
            ("rating", sql(&s["vote_average"])),
            ("tvshow_id", text(&format!("tv.{tv_id}"))),
        ],
    )?;
    for episode in episodes {
        episode_stub(conn, ctx, tv_id, episode, None)?;
    }
    providers(conn, &id, s)?;
    credits(conn, ctx, &id, &s["aggregate_credits"], true)?;
    translations(conn, &id, s)?;
    unique_ids(conn, &id, s)?;
    customs(conn, &id, s)?;
    default_art_of(conn, &id, s)?;
    images(conn, &id, s)
}

pub fn write_episode(
    conn: &Connection,
    ctx: &WriteContext,
    tv_id: i64,
    season: i64,
    episode: i64,
    e: &Value,
) -> Result<()> {
    let id = build_episode_id(tv_id, season, episode);
    let expiry = expiry_seconds(SHORTER_EXPIRY, &e["air_date"], &Value::Null);
    episode_stub(conn, ctx, tv_id, e, None)?;
    base_full(conn, ctx, &id, "episode", expiry)?;
    store(
        conn,
        "episode",
        &[
            ("id", text(&id)),
            ("episode", Sql::Integer(episode)),
            ("year", year(&e["air_date"])),
            ("plot", sql(&e["overview"])),
            ("title", sql(&e["name"])),
            ("premiered", sql(&e["air_date"])),
            ("duration", Sql::Integer(runtime_seconds(&e["runtime"]))),
            ("rating", sql(&e["vote_average"])),
            ("votes", sql(&e["vote_count"])),
            ("season_id", text(&build_season_id(tv_id, season))),
            ("tvshow_id", text(&format!("tv.{tv_id}"))),
        ],
    )?;
    // Episode credits list the regular cast, the crew, and the guest stars.
    let merged = serde_json::json!({
        "cast": e["credits"]["cast"],
        "crew": e["credits"]["crew"],
        "guest_stars": e["credits"]["guest_stars"],
    });
    credits(conn, ctx, &id, &merged, false)?;
    translations(conn, &id, e)?;
    unique_ids(conn, &id, e)?;
    customs(conn, &id, e)?;
    images(conn, &id, e)
}

fn person_credits(
    conn: &Connection,
    ctx: &WriteContext,
    person_tmdb_id: i64,
    items: &Value,
    movie: bool,
) -> Result<()> {
    let prefix = if movie { "movie" } else { "tv" };
    for i in array(&items["cast"]) {
        media_stub(conn, ctx, i, movie)?;
        fill(
            conn,
            "castmember",
            "tmdb_id, role, parent_id",
            &[
                ("tmdb_id", Sql::Integer(person_tmdb_id)),
                ("role", sql(&i["character"])),
                ("ordering", sql(&i["order"])),
                ("appearances", sql(&i["episode_count"])),
                (
                    "parent_id",
                    Sql::Text(format!("{prefix}.{}", i["id"].as_i64().unwrap_or_default())),
                ),
            ],
        )?;
    }
    for i in array(&items["crew"]) {
        media_stub(conn, ctx, i, movie)?;
        fill(
            conn,
            "crewmember",
            "tmdb_id, role, department, parent_id",
            &[
                ("tmdb_id", Sql::Integer(person_tmdb_id)),
                ("role", sql(&i["job"])),
                ("department", sql(&i["department"])),
                ("appearances", sql(&i["episode_count"])),
                (
                    "parent_id",
                    Sql::Text(format!("{prefix}.{}", i["id"].as_i64().unwrap_or_default())),
                ),
            ],
        )?;
    }
    Ok(())
}

pub fn write_person(conn: &Connection, ctx: &WriteContext, p: &Value) -> Result<()> {
    let tmdb_id = p["id"].as_i64().unwrap_or_default();
    let id = format!("person.{tmdb_id}");
    base_full(conn, ctx, &id, "person", DEFAULT_EXPIRY)?;
    store(
        conn,
        "person",
        &[
            ("id", text(&id)),
            ("tmdb_id", Sql::Integer(tmdb_id)),
            ("name", sql(&p["name"])),
            ("known_for_department", sql(&p["known_for_department"])),
            ("gender", sql(&p["gender"])),
            ("biography", sql(&p["biography"])),
            ("birthday", sql(&p["birthday"])),
            ("deathday", sql(&p["deathday"])),
            ("place_of_birth", sql(&p["place_of_birth"])),
            ("popularity", sql(&p["popularity"])),
        ],
    )?;
    person_credits(conn, ctx, tmdb_id, &p["movie_credits"], true)?;
    person_credits(conn, ctx, tmdb_id, &p["tv_credits"], false)?;
    unique_ids(conn, &id, p)?;
    customs(conn, &id, p)?;
    default_art_of(conn, &id, p)?;
    images(conn, &id, p)
}

pub fn write_collection(conn: &Connection, ctx: &WriteContext, c: &Value) -> Result<()> {
    let tmdb_id = c["id"].as_i64().unwrap_or_default();
    let id = format!("collection.{tmdb_id}");
    base_full(conn, ctx, &id, "set", DEFAULT_EXPIRY)?;
    store(
        conn,
        "collection",
        &[
            ("id", text(&id)),
            ("tmdb_id", Sql::Integer(tmdb_id)),
            ("plot", sql(&c["overview"])),
            ("title", sql(&c["name"])),
        ],
    )?;
    for part in array(&c["parts"]) {
        media_stub(conn, ctx, part, true)?;
        belongs(
            conn,
            &format!("movie.{}", part["id"].as_i64().unwrap_or_default()),
            &id,
        )?;
    }
    customs(conn, &id, c)?;
    default_art_of(conn, &id, c)?;
    images(conn, &id, c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_ratio_buckets_match_tmdbhelper() {
        assert_eq!(aspect_ratio_bucket(0.667), 1);
        assert_eq!(aspect_ratio_bucket(1.0), 2);
        assert_eq!(aspect_ratio_bucket(1.5), 3);
        assert_eq!(aspect_ratio_bucket(1.778), 4);
        assert_eq!(aspect_ratio_bucket(2.39), 5);
    }

    #[test]
    fn money_matches_the_addon_format() {
        assert_eq!(
            money(&serde_json::json!(63000000)),
            Some(Sql::Text("$63,000,000".into()))
        );
        assert_eq!(money(&serde_json::json!(0)), Some(Sql::Text("$0".into())));
    }

    #[test]
    fn date_arithmetic_matches_known_days() {
        assert_eq!(days_from_date("1970-01-01"), Some(0));
        assert_eq!(days_from_date("2000-03-01"), Some(11017));
        assert_eq!(days_from_date("nope"), None);
    }

    #[test]
    fn episode_types_match_the_addon() {
        let of = |s: i64, e: i64, t: &str| {
            episode_type(
                &serde_json::json!({"season_number": s, "episode_number": e, "episode_type": t}),
            )
        };
        assert_eq!(of(0, 1, "standard"), Sql::Text("special".into()));
        assert_eq!(of(1, 1, "standard"), Sql::Text("series_premiere".into()));
        assert_eq!(of(2, 1, "standard"), Sql::Text("season_premiere".into()));
        assert_eq!(of(2, 9, "finale"), Sql::Text("season_finale".into()));
        assert_eq!(
            of(2, 5, "mid_season"),
            Sql::Text("mid_season_finale".into())
        );
        assert_eq!(of(2, 5, "standard"), Sql::Text("standard".into()));
    }
}
