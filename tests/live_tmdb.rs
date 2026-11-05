//! Calls the real TMDB API. Run with `just test-live`, which needs TMDB_API_KEY.
mod common;

use warmup_rs::api::{TmdbClient, TmdbError};
use warmup_rs::cache::{mapper, WriteContext};

fn client() -> TmdbClient {
    let key = std::env::var("TMDB_API_KEY").expect("set TMDB_API_KEY to run the live tests");
    TmdbClient::new(key, "en-US".into()).unwrap()
}

#[tokio::test]
#[ignore = "calls TMDB and needs TMDB_API_KEY"]
async fn every_item_type_writes_from_a_real_response() {
    let client = client();
    let ctx = WriteContext {
        language: "en-US".into(),
        genres: client.get_genres().await.unwrap(),
    };
    let (conn, _) = common::scratch_db();
    mapper::write_movie(&conn, &ctx, &client.get_movie(550).await.unwrap()).unwrap();
    let tv = client.get_tv(1396).await.unwrap();
    mapper::write_tv(&conn, &ctx, &tv, &[]).unwrap();
    let season = client.get_season(1396, 1).await.unwrap();
    mapper::write_season(&conn, &ctx, 1396, 1, &season).unwrap();
    let episode = client.get_episode(1396, 1, 1).await.unwrap();
    mapper::write_episode(&conn, &ctx, 1396, 1, 1, &episode).unwrap();
    mapper::write_person(&conn, &ctx, &client.get_person(287).await.unwrap()).unwrap();
    mapper::write_collection(&conn, &ctx, &client.get_collection(10).await.unwrap()).unwrap();

    let hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM baseitem WHERE expiry>0 AND datalevel=5 AND language='en-US'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 6);
    let dangling: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(dangling, 0);
}

#[tokio::test]
#[ignore = "calls TMDB and needs TMDB_API_KEY"]
async fn a_missing_item_is_not_found() {
    assert!(matches!(
        client().get_movie(999_999_999).await,
        Err(TmdbError::NotFound(_))
    ));
}

#[tokio::test]
#[ignore = "calls TMDB"]
async fn a_bad_key_is_unauthorized() {
    let client = TmdbClient::new("not-a-real-key".into(), "en-US".into()).unwrap();
    assert!(matches!(
        client.check_key().await,
        Err(TmdbError::Unauthorized(_))
    ));
}
