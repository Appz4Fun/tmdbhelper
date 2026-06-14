# warmup-rs -- Rust-based TMDBHelper cache warmer

A static Rust binary that warms TMDBHelper's `ItemDetails.db` cache and Kodi's
image texture cache by writing directly to SQLite -- no JSON-RPC, no plugin
invocation. It is **independent of the addon** and runs standalone on the device.

> Extracted from the [nzbdav Kodi addon](https://github.com/Appz4Fun/nzbdavkodi)
> repository into its own project. The crate/binary name remains `warmup-rs`.

## Two modes

| Mode | What it does | Throughput |
|------|-------------|------------|
| `--mode=metadata` (default) | Fetches movies, TV, persons, and collections from the TMDB API with `append_to_response` and writes all 30+ tables in `ItemDetails.db`. Crawls 5 hops deep: seeds + similar + cast filmographies + crew + collections. | ~40 items/s |
| `--mode=images` | Reads every art path from `ItemDetails.db`, downloads from the TMDB CDN at the correct resolution (original for backdrops, w1280 for everything else), saves to `Thumbnails/`, and registers all URL variants in `Textures13.db` so Kodi never needs to fetch images on-demand. | ~45 images/s |

Both modes share the same `state.db` priority queue (depth ASC, popularity DESC)
and are restart-safe -- the metadata service picks up where it left off, and the
image service skips already-downloaded files on disk.

## Key design choices

- Bypasses TMDBHelper entirely -- writes directly to its `ItemDetails.db` schema (30+ normalized tables). This is 290x faster than the Python JSON-RPC approach (~0.14/s) because there's no Kodi plugin serialization bottleneck.
- Mega-transactions: 200 items per SQLite commit for B-tree page locality on USB storage.
- WAL mode with `synchronous=OFF` for safe concurrent reads (Kodi) + writes (warmup-rs).
- Image mode uses Kodi's CRC-32/MPEG-2 hash (ported from `xbmc/utils/Crc32.cpp`) to compute cached filenames. Registers multiple URL variants per image (original, w1280, w780, etc.) pointing to one cached file, so Kodi finds a cache hit regardless of its `imageres` setting.

## Requirements

- CoreELEC (or any aarch64 Linux). The binary is a static musl build with no runtime dependencies.
- A USB SSD (recommended) or fast storage mounted at `/var/media/CACHE_DRIVE`. Internal eMMC works but is slower and has limited space for images.
- TMDBHelper 6.x installed (it created the `ItemDetails.db` schema). The warmer creates tables if needed for `Textures13.db`.

## Build and deploy

```bash
# Cross-compile from macOS/Linux (requires Docker for cross).
cross build --release --target aarch64-unknown-linux-musl

# Deploy the ~5.5 MB static binary.
scp target/aarch64-unknown-linux-musl/release/warmup-rs \
    root@coreelec.local:/storage/tmdb/warmup-rs

# Smoke test on the box.
ssh root@coreelec.local '/storage/tmdb/warmup-rs --mode=smoke'
# Expected: "smoke ok: rusqlite=3.x.y"
```

## Systemd units

Two separate services so they can be started/stopped independently:

```bash
# Deploy both service files.
scp tmdbhelper-warmup-rs.service tmdbhelper-warmup-images.service \
    root@coreelec.local:/storage/.config/system.d/

# Enable + start both.
ssh root@coreelec.local '
  systemctl daemon-reload
  systemctl enable tmdbhelper-warmup-rs tmdbhelper-warmup-images
  systemctl start tmdbhelper-warmup-rs tmdbhelper-warmup-images
'
```

## Operate

```bash
# Monitor both services.
ssh root@coreelec.local 'journalctl -u tmdbhelper-warmup-rs -f'
ssh root@coreelec.local 'journalctl -u tmdbhelper-warmup-images -f'

# Check progress.
ssh root@coreelec.local '
  sqlite3 /var/media/CACHE_DRIVE/tmdb/scriptcache/state.db \
    "SELECT (SELECT COUNT(*) FROM visited), (SELECT COUNT(*) FROM queue);"
  sqlite3 /var/media/CACHE_DRIVE/tmdb/Textures13.db "SELECT COUNT(*) FROM texture;"
  du -sh /var/media/CACHE_DRIVE/tmdb/Thumbnails/
'

# Lower concurrency during video playback.
ssh root@coreelec.local '
  systemctl stop tmdbhelper-warmup-rs tmdbhelper-warmup-images
  # Edit concurrency in the .service files, then:
  systemctl daemon-reload
  systemctl start tmdbhelper-warmup-rs tmdbhelper-warmup-images
'

# Stop gracefully.
ssh root@coreelec.local 'systemctl stop tmdbhelper-warmup-rs tmdbhelper-warmup-images'
```

## CLI flags

| Flag | Default | Description |
|------|---------|-------------|
| `--mode` | `metadata` | `metadata`, `images`, or `smoke` |
| `--concurrency` | `40` | Number of concurrent TMDB API / CDN fetchers |
| `--batch-size` | `200` | Items per SQLite mega-transaction (metadata mode) |
| `--state-db` | `/var/media/CACHE_DRIVE/tmdb/scriptcache/state.db` | Priority queue + visited tracking |
| `--item-details-db` | (see note) | Path to TMDBHelper's ItemDetails.db |
| `--textures-db` | `/var/media/CACHE_DRIVE/tmdb/Textures13.db` | Kodi's texture cache DB (images mode) |
| `--thumbnails-dir` | `/var/media/CACHE_DRIVE/tmdb/Thumbnails` | Kodi's thumbnail directory (images mode) |
| `--tmdb-api-key` | TMDBHelper's bundled key | TMDB API v3 key |

> **Note:** The `--item-details-db` default in the binary points to Kodi's standard addon_data path, but on CoreELEC with a CACHE_DRIVE the DB is symlinked elsewhere. The systemd unit overrides this to `/var/media/CACHE_DRIVE/tmdb/database_07/ItemDetails.db`.

## Notes

- Both services run at `Nice=19` and `IOSchedulingPriority=7` (lowest) so Kodi always gets priority for CPU and disk I/O.
- Running both services simultaneously at high concurrency saturates USB SSD write bandwidth. The sweet spot on a 4-core ARM box with USB 3.0 SSD is ~40 metadata workers + ~25 image workers (~85 items/s combined).
- The image service registers multiple Textures13.db entries per image (e.g., original + w1280 + w780 for backdrops) so Kodi finds a cache hit regardless of its `imageres`/`fanartres` settings.
- Image disk budget is ~250-350 GB for a full 1.3M-image crawl. Monitor `df` on CACHE_DRIVE.
- The queue at depth 5 grows to ~1.3M items. Full metadata crawl takes ~9 hours at 40/s; full image crawl takes ~8 hours at 45/s. Both run in parallel.

## License

GPLv3 -- see [LICENSE](LICENSE) for details.
