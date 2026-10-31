use warmup_rs::id::TmdbType;
use warmup_rs::state::{StateDb, MAX_ATTEMPTS};

fn state() -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(&dir.path().join("state.db")).unwrap();
    (dir, db)
}

#[test]
fn enqueue_pop_visited_roundtrip() {
    let (_dir, mut s) = state();
    s.enqueue_child(550, TmdbType::Movie, 0, 100.0).unwrap();
    s.enqueue_child(680, TmdbType::Movie, 0, 50.0).unwrap();
    s.enqueue_child(287, TmdbType::Person, 1, 80.0).unwrap();

    let batch = s.pop_batch(10).unwrap();
    assert_eq!(
        batch.iter().map(|i| i.tmdb_id).collect::<Vec<_>>(),
        [550, 680, 287],
        "depth first, then popularity"
    );
    assert!(
        s.pop_batch(10).unwrap().is_empty(),
        "claimed items don't come back"
    );

    s.visit_and_enqueue_multi(&[(550, TmdbType::Movie, &[(287, TmdbType::Person, 0.0, 1)])])
        .unwrap();
    assert_eq!(s.visited_count().unwrap(), 1);
    assert_eq!(s.queue_size().unwrap(), 2);

    s.enqueue_child(550, TmdbType::Movie, 0, 100.0).unwrap();
    assert_eq!(
        s.queue_size().unwrap(),
        2,
        "visited items should not re-enqueue"
    );
}

#[test]
fn stopping_mid_batch_loses_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    {
        let mut s = StateDb::open(&path).unwrap();
        s.enqueue_child(550, TmdbType::Movie, 0, 1.0).unwrap();
        assert_eq!(s.pop_batch(10).unwrap().len(), 1);
    }
    let mut s = StateDb::open(&path).unwrap();
    assert_eq!(s.release_claims().unwrap(), 1);
    assert_eq!(s.pop_batch(10).unwrap()[0].tmdb_id, 550);
}

#[test]
fn a_dead_id_finishes_instead_of_returning() {
    let (_dir, mut s) = state();
    s.enqueue_child(1, TmdbType::Movie, 0, 1.0).unwrap();
    s.pop_batch(1).unwrap();
    s.finish(1, TmdbType::Movie, "not_found").unwrap();
    assert_eq!(s.queue_size().unwrap(), 0);
    assert_eq!(s.visited_count().unwrap(), 1);
}

#[test]
fn a_failing_item_stops_after_max_attempts() {
    let (_dir, mut s) = state();
    s.enqueue_child(1, TmdbType::Movie, 0, 1.0).unwrap();
    for attempt in 1..=MAX_ATTEMPTS {
        assert_eq!(s.pop_batch(1).unwrap().len(), 1, "attempt {attempt}");
        s.record_failure(1, TmdbType::Movie).unwrap();
    }
    assert!(s.pop_batch(1).unwrap().is_empty());
    assert_eq!(s.visited_count().unwrap(), 1);
}

#[test]
fn an_outage_does_not_count_against_an_item() {
    let (_dir, mut s) = state();
    s.enqueue_child(1, TmdbType::Movie, 0, 1.0).unwrap();
    for _ in 0..MAX_ATTEMPTS * 2 {
        assert_eq!(s.pop_batch(1).unwrap().len(), 1);
        s.release(1, TmdbType::Movie).unwrap();
    }
    assert_eq!(s.visited_count().unwrap(), 0);
}

#[test]
fn a_closer_path_raises_a_queued_item() {
    let (_dir, mut s) = state();
    s.enqueue_child(2, TmdbType::Movie, 2, 1.0).unwrap();
    s.enqueue_child(3, TmdbType::Movie, 1, 1.0).unwrap();
    s.enqueue_child(2, TmdbType::Movie, 0, 5.0).unwrap();
    assert_eq!(s.pop_batch(1).unwrap()[0].tmdb_id, 2);
}
