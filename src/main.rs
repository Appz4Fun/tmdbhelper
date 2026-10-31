use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use tracing::info;
use warmup_rs::paths::{self, Paths, Platform};

const AFTER_HELP: &str = "\
Default paths:
  On CoreELEC, LibreELEC, and OpenELEC, the defaults point at Kodi's own files:
    ItemDetails.db  /storage/.kodi/userdata/addon_data/plugin.video.themoviedb.helper/database_07/ItemDetails.db
    state.db        /var/media/CACHE_DRIVE/tmdb/scriptcache/state.db
    Textures13.db   /var/media/CACHE_DRIVE/tmdb/Textures13.db
    Thumbnails      /var/media/CACHE_DRIVE/tmdb/Thumbnails
  On Windows, macOS, and desktop Linux, everything goes under --output-dir:
    ItemDetails.db, state.db, and Textures13.db in the output folder, and images in <output-dir>/images.
  A path flag or its environment variable overrides the default.

Examples:
  warmup-rs --mode=smoke
  WARMUP_TMDB_API_KEY=<key> warmup-rs
  WARMUP_TMDB_API_KEY=<key> warmup-rs --all-ids
  warmup-rs --mode=images --concurrency=25";

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Mode {
    /// crawl TMDB and write ItemDetails.db.
    Metadata,
    /// download artwork and register it in Textures13.db.
    Images,
    /// check that the binary runs, then exit without side effects.
    Smoke,
}

#[derive(Parser, Debug)]
#[command(
    name = "warmup-rs",
    version,
    about = "Fills TMDBHelper's metadata cache and Kodi's image cache by writing directly to SQLite.",
    after_help = AFTER_HELP
)]
struct Args {
    /// Run mode.
    #[arg(long, env = "WARMUP_MODE", value_enum, default_value_t = Mode::Metadata)]
    mode: Mode,

    /// TMDB API v3 key. Required in metadata mode.
    #[arg(long, env = "WARMUP_TMDB_API_KEY", hide_env_values = true)]
    tmdb_api_key: Option<String>,

    /// Number of concurrent requests to TMDB.
    #[arg(long, env = "WARMUP_CONCURRENCY", default_value_t = 20, value_parser = clap::value_parser!(u64).range(1..))]
    concurrency: u64,

    /// The add-on's language setting. TMDBHelper ignores rows in any other language.
    #[arg(long, env = "WARMUP_LANGUAGE", default_value = "en-US")]
    language: String,

    /// Queue every title, person, and collection from TMDB's daily ID lists in metadata mode.
    #[arg(long, env = "WARMUP_ALL_IDS")]
    all_ids: bool,

    /// Pause when the output disk has less free space than this many GB.
    #[arg(long, env = "WARMUP_MIN_FREE_GB", default_value_t = 10)]
    min_free_gb: u64,

    /// Number of items to take from the queue per loop in metadata mode.
    #[arg(long, env = "WARMUP_BATCH_SIZE", default_value_t = 200, value_parser = clap::value_parser!(u64).range(1..))]
    batch_size: u64,

    /// Base folder for the default paths on desktop systems.
    #[arg(long, env = "WARMUP_OUTPUT_DIR", default_value = paths::DEFAULT_OUTPUT_DIR)]
    output_dir: PathBuf,

    /// Crawler queue and visited-item database.
    #[arg(long, env = "WARMUP_STATE_DB")]
    state_db: Option<PathBuf>,

    /// TMDBHelper's metadata database.
    #[arg(long, env = "WARMUP_ITEM_DETAILS_DB")]
    item_details_db: Option<PathBuf>,

    /// Kodi's texture cache database, used in images mode.
    #[arg(long, env = "WARMUP_TEXTURES_DB")]
    textures_db: Option<PathBuf>,

    /// Folder for downloaded images, used in images mode.
    #[arg(long, env = "WARMUP_THUMBNAILS_DIR")]
    thumbnails_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warmup_rs=info,warn".into()),
        )
        .with_target(false)
        .init();

    let args = Args::parse();
    if args.mode == Mode::Metadata && args.tmdb_api_key.is_none() {
        anyhow::bail!(
            "metadata mode needs a TMDB API key: set --tmdb-api-key or WARMUP_TMDB_API_KEY"
        );
    }
    let concurrency = args.concurrency as usize;
    let batch_size = args.batch_size as usize;
    let min_free_bytes = args.min_free_gb * 1024 * 1024 * 1024;

    let platform = Platform::detect();
    let defaults = Paths::defaults(platform, &args.output_dir);
    let paths = Paths {
        state_db: args.state_db.unwrap_or(defaults.state_db),
        item_details_db: args.item_details_db.unwrap_or(defaults.item_details_db),
        textures_db: args.textures_db.unwrap_or(defaults.textures_db),
        thumbnails_dir: args.thumbnails_dir.unwrap_or(defaults.thumbnails_dir),
    };
    info!(
        "warmup-rs starting mode={} platform={} concurrency={} batch={}",
        args.mode
            .to_possible_value()
            .map_or_else(String::new, |v| v.get_name().to_owned()),
        platform.name(),
        concurrency,
        batch_size
    );

    if args.mode != Mode::Smoke && platform == Platform::Desktop {
        paths::ensure_dirs(&paths)?;
        if paths::create_item_details_if_missing(&paths.item_details_db)? {
            info!(
                "created {} from the bundled TMDBHelper schema",
                paths.item_details_db.display()
            );
        }
    }

    match args.mode {
        Mode::Metadata => {
            let api_key = args.tmdb_api_key.unwrap_or_default();
            info!(
                "metadata mode: state={} target={}",
                paths.state_db.display(),
                paths.item_details_db.display()
            );
            warmup_rs::worker::run(warmup_rs::worker::Options {
                state_path: paths.state_db,
                item_details_path: paths.item_details_db,
                api_key,
                language: args.language,
                concurrency,
                batch_size,
                all_ids: args.all_ids,
                min_free_bytes,
            })
            .await?;
        }
        Mode::Images => {
            info!(
                "images mode: source={} textures={} thumbnails={}",
                paths.item_details_db.display(),
                paths.textures_db.display(),
                paths.thumbnails_dir.display()
            );
            warmup_rs::images::run(
                paths.item_details_db,
                paths.textures_db,
                paths.thumbnails_dir,
                concurrency,
                min_free_bytes,
            )
            .await?;
        }
        Mode::Smoke => {
            let _ = rusqlite::Connection::open_in_memory()?;
            let _ = reqwest::Client::builder().build()?;
            info!("smoke OK: rusqlite={}", rusqlite::version());
        }
    }
    Ok(())
}
