use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TmdbType {
    Movie,
    Tv,
    Season,
    Episode,
    Person,
    Collection,
}

impl TmdbType {
    /// The string used in `baseitem.mediatype`. Note: collection→"set", tv→"tvshow".
    pub fn mediatype(&self) -> &'static str {
        match self {
            TmdbType::Movie => "movie",
            TmdbType::Tv => "tvshow",
            TmdbType::Season => "season",
            TmdbType::Episode => "episode",
            TmdbType::Person => "person",
            TmdbType::Collection => "set",
        }
    }

    /// The prefix used in `baseitem.id` strings.
    pub fn id_prefix(&self) -> &'static str {
        match self {
            TmdbType::Movie => "movie",
            TmdbType::Tv | TmdbType::Season | TmdbType::Episode => "tv",
            TmdbType::Person => "person",
            TmdbType::Collection => "collection",
        }
    }
}

impl fmt::Display for TmdbType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TmdbType::Movie => "movie",
            TmdbType::Tv => "tv",
            TmdbType::Season => "season",
            TmdbType::Episode => "episode",
            TmdbType::Person => "person",
            TmdbType::Collection => "collection",
        })
    }
}

/// Build the baseitem.id string. Examples:
/// build_item_id(Movie, 550) → "movie.550"
/// build_item_id(Tv, 76479)  → "tv.76479"
/// build_item_id(Collection, 10) → "collection.10"
pub fn build_item_id(t: TmdbType, tmdb_id: i64) -> String {
    format!("{}.{}", t.id_prefix(), tmdb_id)
}

pub fn build_season_id(tv_tmdb_id: i64, season: i64) -> String {
    format!("tv.{}.{}", tv_tmdb_id, season)
}

pub fn build_episode_id(tv_tmdb_id: i64, season: i64, episode: i64) -> String {
    format!("tv.{}.{}.{}", tv_tmdb_id, season, episode)
}

const SEASON_BITS: u32 = 14;
const EPISODE_BITS: u32 = 17;

/// The queue keys every item by one integer, so a season or episode packs its
/// show ID (31 bits), season (14 bits), and episode (17 bits) together.
pub fn pack_episode_key(tv_tmdb_id: i64, season: i64, episode: i64) -> i64 {
    (tv_tmdb_id << (SEASON_BITS + EPISODE_BITS)) | (season << EPISODE_BITS) | episode
}

/// Reverses `pack_episode_key`, returning (show ID, season, episode).
pub fn unpack_episode_key(key: i64) -> (i64, i64, i64) {
    (
        key >> (SEASON_BITS + EPISODE_BITS),
        (key >> EPISODE_BITS) & ((1 << SEASON_BITS) - 1),
        key & ((1 << EPISODE_BITS) - 1),
    )
}

/// Reverses the `build_*_id` functions, returning the queue key and type.
/// Returns `None` for IDs the crawl doesn't queue, such as episode groups.
pub fn parse_item_id(id: &str) -> Option<(i64, TmdbType)> {
    let mut parts = id.split('.');
    let prefix = parts.next()?;
    let numbers = parts
        .map(|p| p.parse::<i64>().ok())
        .collect::<Option<Vec<i64>>>()?;
    Some(match (prefix, numbers.as_slice()) {
        ("movie", [id]) => (*id, TmdbType::Movie),
        ("person", [id]) => (*id, TmdbType::Person),
        ("collection", [id]) => (*id, TmdbType::Collection),
        ("tv", [id]) => (*id, TmdbType::Tv),
        ("tv", [id, season]) => (pack_episode_key(*id, *season, 0), TmdbType::Season),
        ("tv", [id, season, episode]) => {
            (pack_episode_key(*id, *season, *episode), TmdbType::Episode)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_ids_parse_back_to_queue_keys() {
        assert_eq!(parse_item_id("movie.550"), Some((550, TmdbType::Movie)));
        assert_eq!(parse_item_id("tv.1396"), Some((1396, TmdbType::Tv)));
        assert_eq!(
            parse_item_id("tv.1396.2"),
            Some((pack_episode_key(1396, 2, 0), TmdbType::Season))
        );
        assert_eq!(
            parse_item_id("tv.1396.2.3"),
            Some((pack_episode_key(1396, 2, 3), TmdbType::Episode))
        );
        assert_eq!(
            parse_item_id("collection.10"),
            Some((10, TmdbType::Collection))
        );
        assert_eq!(parse_item_id("episode_group.5b11"), None);
        assert_eq!(parse_item_id("tv.x"), None);
    }

    #[test]
    fn id_format_matches_observed_tmdbhelper_strings() {
        assert_eq!(build_item_id(TmdbType::Movie, 1226863), "movie.1226863");
        assert_eq!(build_item_id(TmdbType::Tv, 76479), "tv.76479");
        assert_eq!(build_item_id(TmdbType::Person, 58321), "person.58321");
        assert_eq!(build_item_id(TmdbType::Collection, 10), "collection.10");
        assert_eq!(build_season_id(76479, 5), "tv.76479.5");
        assert_eq!(build_episode_id(76479, 5, 3), "tv.76479.5.3");
    }

    #[test]
    fn mediatype_strings_match_tmdbhelper_baseitem_enum() {
        assert_eq!(TmdbType::Movie.mediatype(), "movie");
        assert_eq!(TmdbType::Tv.mediatype(), "tvshow");
        assert_eq!(TmdbType::Season.mediatype(), "season");
        assert_eq!(TmdbType::Episode.mediatype(), "episode");
        assert_eq!(TmdbType::Person.mediatype(), "person");
        assert_eq!(TmdbType::Collection.mediatype(), "set");
    }

    #[test]
    fn episode_keys_round_trip() {
        for (tv, s, e) in [(1396, 1, 1), (2_147_483_647, 16_383, 131_071), (1, 0, 0)] {
            assert_eq!(unpack_episode_key(pack_episode_key(tv, s, e)), (tv, s, e));
        }
    }
}
