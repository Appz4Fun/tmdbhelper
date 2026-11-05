//! Writes small TMDB responses and checks them against what TMDBHelper requires
//! before it treats a row as a cache hit. The responses are trimmed from real ones.
mod common;

use rusqlite::{Connection, OptionalExtension};
use serde_json::json;
use std::collections::HashMap;
use warmup_rs::cache::{mapper, WriteContext, SCHEMA_VERSION};

fn ctx() -> WriteContext {
    WriteContext {
        language: "en-US".into(),
        genres: HashMap::from([(18, "Drama".to_owned())]),
    }
}

fn value(conn: &Connection, sql: &str) -> Option<String> {
    conn.query_row(sql, [], |r| {
        r.get::<_, rusqlite::types::Value>(0)
            .map(|v| format!("{v:?}"))
    })
    .optional()
    .unwrap()
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// TMDBHelper's cached_data_conditions: a full, unexpired row in its language.
fn is_cache_hit(conn: &Connection, id: &str) -> bool {
    count(
        conn,
        &format!(
            "SELECT COUNT(*) FROM baseitem WHERE id='{id}' AND expiry>=strftime('%s','now')
             AND datalevel>=5 AND fanart_tv>=0 AND translation>=0 AND language='en-US'"
        ),
    ) == 1
}

fn no_dangling_rows(conn: &Connection) {
    let violations = count(conn, "SELECT COUNT(*) FROM pragma_foreign_key_check");
    assert_eq!(violations, 0, "the add-on writes with foreign keys on");
}

#[test]
fn schema_is_the_version_the_addon_keeps() {
    let (conn, _) = common::scratch_db();
    assert_eq!(
        count(&conn, "PRAGMA user_version"),
        SCHEMA_VERSION,
        "an older non-zero version makes the add-on drop every table"
    );
}

#[test]
fn movie_is_a_cache_hit_with_the_addon_fields() {
    let (conn, _) = common::scratch_db();
    let movie = json!({
        "id": 550, "title": "Fight Club", "original_title": "Fight Club",
        "original_language": "en", "overview": "Soap.", "tagline": "Mischief.",
        "release_date": "1999-10-15", "runtime": 139, "status": "Released",
        "vote_average": 8.4, "vote_count": 33180, "popularity": 40.5,
        "budget": 63000000, "revenue": 0, "homepage": "",
        "poster_path": "/poster.jpg", "backdrop_path": "/backdrop.jpg",
        "belongs_to_collection": {"id": 9, "name": "Set", "poster_path": "/set.jpg"},
        "genres": [{"id": 18, "name": "Drama"}],
        "spoken_languages": [{"iso_639_1": "en", "name": "English", "english_name": "English"}],
        "production_countries": [{"iso_3166_1": "US", "name": "United States"}],
        "production_companies": [{"id": 1, "name": "Co", "logo_path": null, "origin_country": ""}],
        "release_dates": {"results": [{"iso_3166_1": "US", "release_dates": [
            {"certification": "R", "iso_639_1": "", "release_date": "1999-10-15", "type": 3}]}]},
        "videos": {"results": [
            {"site": "YouTube", "key": "abc", "name": "T", "type": "Trailer", "iso_639_1": "en", "iso_3166_1": "US", "published_at": "2020"},
            {"site": "Vimeo", "key": "skip", "name": "V", "type": "Clip", "iso_639_1": "en", "iso_3166_1": "US", "published_at": "2020"}]},
        "watch/providers": {"results": {"US": {"link": "x", "flatrate": [
            {"provider_id": 8, "provider_name": "Netflix", "logo_path": "/n.png", "display_priority": 1}]}}},
        "credits": {
            "cast": [{"id": 287, "name": "Brad", "gender": 2, "known_for_department": "Acting",
                      "profile_path": "/brad.jpg", "character": "Tyler", "order": 0, "credit_id": "c1"}],
            "crew": [{"id": 7, "name": "David", "job": "Director", "department": "Directing", "credit_id": "c2"}]},
        "external_ids": {"imdb_id": "tt0137523", "facebook_id": null},
        "images": {"posters": [{"file_path": "/poster.jpg", "aspect_ratio": 0.667, "width": 2000,
                                "height": 3000, "iso_639_1": "en", "iso_3166_1": "US",
                                "vote_average": 5.5, "vote_count": 3}]},
    });
    mapper::write_movie(&conn, &ctx(), &movie).unwrap();

    assert!(is_cache_hit(&conn, "movie.550"));
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM ratings"),
        0,
        "a ratings row stops the add-on fetching ratings"
    );
    assert_eq!(
        value(&conn, "SELECT duration FROM movie").unwrap(),
        "Integer(8340)"
    );
    assert_eq!(
        value(&conn, "SELECT value FROM custom WHERE key='budget'").unwrap(),
        "Text(\"$63,000,000\")"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM custom WHERE key='homepage'"),
        0
    );
    assert_eq!(
        value(&conn, "SELECT value FROM unique_id WHERE key='facebook'").unwrap(),
        "Text(\"None\")"
    );
    assert_eq!(
        value(&conn, "SELECT release_type FROM certification").unwrap(),
        "Text(\"Theatrical\")"
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video"), 1);
    assert_eq!(
        value(&conn, "SELECT availability FROM provider").unwrap(),
        "Text(\"flatrate\")"
    );
    assert_eq!(
        value(
            &conn,
            "SELECT icon FROM default_art WHERE parent_id='movie.550' AND type='posters'"
        )
        .unwrap(),
        "Text(\"/poster.jpg\")"
    );
    assert_eq!(
        value(&conn, "SELECT quality FROM art WHERE icon='/poster.jpg'").unwrap(),
        "Integer(30)"
    );
    assert_eq!(
        value(&conn, "SELECT rating FROM art WHERE icon='/poster.jpg'").unwrap(),
        "Integer(550)"
    );
    assert_eq!(
        value(&conn, "SELECT parent_id FROM belongs").unwrap(),
        "Text(\"collection.9\")"
    );
    assert_eq!(
        value(
            &conn,
            "SELECT mediatype FROM baseitem WHERE id='collection.9'"
        )
        .unwrap(),
        "Text(\"set\")"
    );
    // Credits leave stubs that stay misses, so the add-on fetches the person if the crawl hasn't.
    assert_eq!(
        value(&conn, "SELECT name FROM person WHERE tmdb_id=287").unwrap(),
        "Text(\"Brad\")"
    );
    assert!(!is_cache_hit(&conn, "person.287"));
    assert_eq!(
        value(&conn, "SELECT icon FROM art WHERE parent_id='person.287'").unwrap(),
        "Text(\"/brad.jpg\")"
    );
    no_dangling_rows(&conn);

    // A refresh must not duplicate rows whose UNIQUE columns hold NULLs.
    mapper::write_movie(&conn, &ctx(), &movie).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM certification"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM crewmember"), 1);
}

#[test]
fn show_season_and_episode_are_cache_hits() {
    let (conn, _) = common::scratch_db();
    let show = json!({
        "id": 1396, "name": "Breaking Bad", "original_name": "Breaking Bad",
        "first_air_date": "2008-01-20", "status": "Ended", "in_production": false,
        "episode_run_time": [], "number_of_seasons": 1, "number_of_episodes": 2,
        "vote_average": 8.9, "vote_count": 19000, "popularity": 212.8, "original_language": "en",
        "last_episode_to_air": {"id": 2, "season_number": 1, "episode_number": 2, "name": "Last",
                                "runtime": 56, "air_date": "2008-01-27", "episode_type": "finale"},
        "next_episode_to_air": null,
        "seasons": [{"season_number": 1, "episode_count": 2, "air_date": "2008-01-20",
                     "name": "Season 1", "poster_path": "/s1.jpg", "vote_average": 8.4}],
        "networks": [{"id": 174, "name": "AMC", "logo_path": "/amc.png", "origin_country": "US"}],
        "created_by": [{"id": 66633, "name": "Vince", "gender": 2}],
        "content_ratings": {"results": [{"iso_3166_1": "US", "rating": "TV-MA"}]},
        "aggregate_credits": {
            "cast": [{"id": 17419, "name": "Bryan", "order": 0, "total_episode_count": 62,
                      "roles": [{"credit_id": "r1", "character": "Walter", "episode_count": 62}]}],
            "crew": [{"id": 5, "name": "Crew", "department": "Directing", "total_episode_count": 3,
                      "jobs": [{"credit_id": "j1", "job": "Director", "episode_count": 3}]}]},
        "episode_groups": {"results": [{"id": "g1", "name": "DVD", "description": "", "type": 3}]},
    });
    let group = json!({
        "id": "g1", "name": "DVD", "description": "Order", "type": 3,
        "groups": [{"id": "gs1", "name": "Disc 1", "order": 1, "episodes": [
            {"id": 1, "season_number": 1, "episode_number": 1, "order": 0, "name": "Pilot", "overview": "Plot"}]}],
    });
    mapper::write_tv(&conn, &ctx(), &show, &[group]).unwrap();

    assert!(is_cache_hit(&conn, "tv.1396"));
    assert!(!is_cache_hit(&conn, "tv.1396.1"));
    assert_eq!(
        value(&conn, "SELECT duration FROM tvshow").unwrap(),
        "Integer(3360)"
    );
    assert_eq!(
        value(&conn, "SELECT last_episode_to_air_id FROM tvshow").unwrap(),
        "Text(\"tv.1396.1.2\")"
    );
    assert_eq!(
        value(&conn, "SELECT status FROM episode WHERE id='tv.1396.1.2'").unwrap(),
        "Text(\"series_finale\")"
    );
    assert_eq!(
        value(&conn, "SELECT episode_count FROM season").unwrap(),
        "Integer(2)"
    );
    assert_eq!(
        value(&conn, "SELECT name FROM certification").unwrap(),
        "Text(\"TV-MA\")"
    );
    assert_eq!(
        value(&conn, "SELECT appearances FROM castmember").unwrap(),
        "Integer(62)"
    );
    assert_eq!(
        value(&conn, "SELECT role FROM castmember").unwrap(),
        "Text(\"Walter\")"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM crewmember WHERE role='Creator'"
        ),
        1
    );
    assert_eq!(
        value(&conn, "SELECT type FROM episode_groups").unwrap(),
        "Integer(3)"
    );
    assert_eq!(
        value(&conn, "SELECT group_id FROM episode_group_seasons").unwrap(),
        "Text(\"g1\")"
    );
    assert_eq!(
        value(&conn, "SELECT id FROM episode_group_season_episodes").unwrap(),
        "Text(\"tv.1396.1.1\")"
    );
    assert!(is_cache_hit(&conn, "episode_group.g1"));
    no_dangling_rows(&conn);

    let season = json!({
        "id": 3572, "season_number": 1, "name": "Season 1", "overview": "Chemistry.",
        "air_date": "2008-01-20", "vote_average": 8.4, "poster_path": "/s1.jpg",
        "episodes": [
            {"id": 1, "season_number": 1, "episode_number": 1, "name": "Pilot", "air_date": "2008-01-20",
             "runtime": 59, "episode_type": "standard", "still_path": "/still.jpg", "vote_average": 8.5, "vote_count": 570},
            {"id": 2, "season_number": 1, "episode_number": 2, "name": "Last", "air_date": "2008-01-27",
             "runtime": 56, "episode_type": "finale"}],
    });
    mapper::write_season(&conn, &ctx(), 1396, 1, &season).unwrap();
    assert!(is_cache_hit(&conn, "tv.1396.1"));
    assert_eq!(
        value(&conn, "SELECT status FROM episode WHERE id='tv.1396.1.1'").unwrap(),
        "Text(\"series_premiere\")"
    );
    assert_eq!(
        value(&conn, "SELECT icon FROM art WHERE parent_id='tv.1396.1.1'").unwrap(),
        "Text(\"/still.jpg\")"
    );

    let episode = json!({
        "id": 1, "season_number": 1, "episode_number": 1, "name": "Pilot", "overview": "Plot.",
        "air_date": "2008-01-20", "runtime": 59, "vote_average": 8.5, "vote_count": 570,
        "credits": {"cast": [], "crew": [],
            "guest_stars": [{"id": 9, "name": "Guest", "character": "Tuco", "order": 500, "credit_id": "g"}]},
        "external_ids": {"imdb_id": "tt0959621"},
        "images": {"stills": [{"file_path": "/still.jpg", "aspect_ratio": 1.778, "width": 1920,
                               "height": 1080, "iso_639_1": null, "iso_3166_1": null,
                               "vote_average": 6.08, "vote_count": 18}]},
    });
    mapper::write_episode(&conn, &ctx(), 1396, 1, 1, &episode).unwrap();
    assert!(is_cache_hit(&conn, "tv.1396.1.1"));
    assert_eq!(
        value(&conn, "SELECT status FROM episode WHERE id='tv.1396.1.1'").unwrap(),
        "Text(\"series_premiere\")",
        "a full write keeps the stub's status"
    );
    assert_eq!(
        value(&conn, "SELECT guest FROM castmember WHERE tmdb_id=9").unwrap(),
        "Integer(1)"
    );
    assert_eq!(
        value(
            &conn,
            "SELECT rating FROM art WHERE parent_id='tv.1396.1.1'"
        )
        .unwrap(),
        "Integer(608)"
    );
    no_dangling_rows(&conn);
}

#[test]
fn person_and_collection_are_cache_hits() {
    let (conn, _) = common::scratch_db();
    let person = json!({
        "id": 287, "name": "Brad Pitt", "gender": 2, "biography": "Actor.", "birthday": "1963-12-18",
        "known_for_department": "Acting", "place_of_birth": "Oklahoma", "popularity": 10.8,
        "profile_path": "/brad.jpg",
        "movie_credits": {"cast": [{"id": 550, "title": "Fight Club", "release_date": "1999-10-15",
            "genre_ids": [18, 999], "character": "Tyler", "order": 1, "credit_id": "c1", "poster_path": "/p.jpg"}], "crew": []},
        "tv_credits": {"cast": [{"id": 1, "name": "Show", "first_air_date": "2000-01-01",
            "character": "Self", "episode_count": 4, "credit_id": "c2"}], "crew": []},
        "images": {"profiles": [{"file_path": "/brad.jpg", "aspect_ratio": 0.667, "width": 1000,
                                 "height": 1500, "iso_639_1": null, "iso_3166_1": null,
                                 "vote_average": 5.0, "vote_count": 1}]},
    });
    mapper::write_person(&conn, &ctx(), &person).unwrap();
    assert!(is_cache_hit(&conn, "person.287"));
    assert_eq!(
        value(&conn, "SELECT title FROM movie").unwrap(),
        "Text(\"Fight Club\")"
    );
    assert!(!is_cache_hit(&conn, "movie.550"));
    assert_eq!(
        value(&conn, "SELECT parent_id FROM castmember WHERE role='Self'").unwrap(),
        "Text(\"tv.1\")"
    );
    assert_eq!(
        value(
            &conn,
            "SELECT appearances FROM castmember WHERE role='Self'"
        )
        .unwrap(),
        "Integer(4)"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM genre WHERE parent_id='movie.550'"
        ),
        1
    );

    let collection = json!({
        "id": 10, "name": "Star Wars", "overview": "Space.", "poster_path": "/sw.jpg",
        "parts": [{"id": 11, "title": "Star Wars", "release_date": "1977-05-25", "poster_path": "/p.jpg"}],
    });
    mapper::write_collection(&conn, &ctx(), &collection).unwrap();
    assert!(is_cache_hit(&conn, "collection.10"));
    assert_eq!(
        value(
            &conn,
            "SELECT id FROM belongs WHERE parent_id='collection.10'"
        )
        .unwrap(),
        "Text(\"movie.11\")"
    );
    no_dangling_rows(&conn);
}

#[test]
fn fanart_marks_the_item_and_files_season_art() {
    let (conn, _) = common::scratch_db();
    let show =
        json!({"id": 1, "name": "Show", "first_air_date": "2000-01-01", "in_production": false});
    mapper::write_tv(&conn, &ctx(), &show, &[]).unwrap();
    assert_eq!(
        value(&conn, "SELECT fanart_tv FROM baseitem WHERE id='tv.1'").unwrap(),
        "Integer(0)"
    );

    let data = json!({
        "name": "Show", "thetvdb_id": "9",
        "hdtvlogo": [{"id": "1", "url": "https://assets.fanart.tv/logo.png", "lang": "en", "likes": "5"}],
        "tvposter": [{"id": "2", "url": "https://assets.fanart.tv/poster.jpg", "lang": "", "likes": "0"}],
        "showbackground": [
            {"id": "3", "url": "https://assets.fanart.tv/all.jpg", "lang": "", "likes": "1", "season": "all"},
            {"id": "4", "url": "https://assets.fanart.tv/s2.jpg", "lang": "en", "likes": "1", "season": "2"}],
        "seasonposter": [{"id": "5", "url": "https://assets.fanart.tv/sall.jpg", "lang": "en", "likes": "1", "season": "all"}],
        "tvunknown": [{"id": "6", "url": "https://assets.fanart.tv/skip.jpg"}],
    });
    mapper::write_fanart(&conn, &ctx(), "tv.1", &data).unwrap();

    assert_eq!(
        value(&conn, "SELECT fanart_tv FROM baseitem WHERE id='tv.1'").unwrap(),
        "Integer(1)"
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM fanart_tv"), 5);
    let row = "SELECT type || '|' || quality || '|' || likes || '|' || iso_language || '|' || extension FROM fanart_tv WHERE icon LIKE '%logo.png'";
    assert_eq!(value(&conn, row).unwrap(), "Text(\"clearlogo|1|5|en|png\")");
    let parent = |icon: &str| {
        value(
            &conn,
            &format!("SELECT parent_id FROM fanart_tv WHERE icon LIKE '%{icon}'"),
        )
        .unwrap()
    };
    assert_eq!(parent("all.jpg"), "Text(\"tv.1\")");
    assert_eq!(parent("s2.jpg"), "Text(\"tv.1.2\")");
    assert_eq!(parent("sall.jpg"), "Text(\"tv.1.-1\")");
    assert_eq!(
        value(
            &conn,
            "SELECT iso_language FROM fanart_tv WHERE icon LIKE '%poster.jpg'"
        )
        .unwrap(),
        "Null"
    );
    no_dangling_rows(&conn);
}
