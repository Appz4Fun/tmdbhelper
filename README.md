# warmup-rs — Rust TMDBHelper cache warmer

A small, static Rust binary that pre-warms two caches used by Kodi's
[TMDBHelper](https://github.com/jurialmunkey/plugin.video.themoviedb.helper)
addon by writing **directly to SQLite** — no JSON-RPC, no Python plugin
invocation, no Kodi in the loop:

1. **`ItemDetails.db`** — TMDBHelper's metadata cache (~25 normalized tables:
   `movie`, `tv`, `person`, `collection`, `credits`, `art`, `ratings`,
   `certification`, `provider`, `translation`, …).
2. **`Textures13.db` + `Thumbnails/`** — Kodi's image texture cache, so the UI
   never has to fetch artwork on-demand.

It runs standalone on the device and is **independent of the addon** — it just
needs the schema the addon already created.

> Extracted from the [nzbdav Kodi addon](https://github.com/Appz4Fun/nzbdavkodi)
> repository into its own project. The crate/binary name is `warmup-rs`.

## Why

Warming these caches through Kodi's plugin layer (the usual Python + JSON-RPC
approach) is bottlenecked by plugin serialization and is painfully slow.
`warmup-rs` skips Kodi entirely and writes the same rows TMDBHelper would have
written, fetching from TMDB with `append_to_response` so one HTTP request packs
images, videos, credits, external IDs, translations, keywords, similar,
recommendations and watch providers into a single response. On a 4-core ARM box
with a USB 3.0 SSD the author observes roughly **40 metadata items/s** and
**45 images/s**.

## Modes

The binary selects a mode with `--mode` (default `metadata`):

| Mode | What it does |
|------|--------------|
| `metadata` (default) | Crawls TMDB and writes `ItemDetails.db`. Seeds from TMDB's popular/trending lists, then does a breadth-first walk through credits, similar/recommended titles, person filmographies and collections. |
| `images` | Reads every art path out of `ItemDetails.db`, downloads it from the TMDB CDN at the right resolution, saves it under `Thumbnails/`, and registers every URL variant in `Textures13.db` so Kodi gets a cache hit regardless of its `imageres`/`fanartres` setting. |
| `smoke` | Opens an in-memory SQLite + builds an HTTP client and exits — a zero-side-effect "does this binary run on the box" check. |

Both real modes are **restart-safe**: metadata mode resumes from its on-disk
priority queue, and images mode skips files already present on disk and URLs
already registered in `Textures13.db`. The two modes are independent processes
and can run at the same time.

## How it works

### Metadata crawl (`--mode=metadata`)

- **Seeds** come from `trending/all/week`, `movie/popular`, `movie/now_playing`,
  `movie/upcoming`, `tv/popular` and `person/popular` (a few pages each;
  see `src/seed.rs`).
- A separate SQLite **`state.db`** holds the work queue and a `visited` set.
  Items are popped in priority order — `depth ASC, popularity DESC,
  enqueued_at ASC` — so popular, shallow items are warmed first.
- Each fetched item enqueues its children: movies enqueue cast + crew + similar
  + recommendations + their collection; TV enqueues cast + crew + similar +
  recommendations; persons enqueue their combined filmography; collections
  enqueue their parts.
- The crawl is bounded to **2 hops from the seeds** (`MAX_DEPTH = 2` in
  `src/worker.rs`): depth-0 seeds and depth-1 items enqueue children; depth-2
  items are warmed but expand no further.
- Async fetchers (Tokio, bounded by `--concurrency`) feed a single blocking
  **writer task**. The writer accumulates up to 200 items and commits them in
  one **mega-transaction**, which is what makes this fast on USB storage —
  un-batched auto-commits cost ~10 ms each.
- Items that 404 or fail are re-queued; items older than 30 days are
  re-crawled when the queue drains (`requeue_expired`).

### Image crawl (`--mode=images`)

- Reads `SELECT DISTINCT icon, type FROM art` from `ItemDetails.db`.
- Downloads each image once at the largest useful size (`original` for
  backdrops, `w1280` for everything else).
- Computes the cached filename with Kodi's **CRC-32/MPEG-2** hash (ported
  byte-for-byte from `xbmc/utils/Crc32.cpp`) — `<lowercased-url>` → `crc32` →
  `Thumbnails/<first-hex-digit>/<crc>.<ext>`. The hash is verified against real
  `Textures13.db` filenames in the unit tests.
- Registers **multiple URL variants** per image (e.g. a backdrop is registered
  as `original`, `w1280` and `w780`) all pointing at the one cached file, so
  Kodi finds a hit no matter which resolution it asks for.
- Parses JPEG/PNG headers to fill in the `sizes` table without decoding the
  whole image.
- Re-scans on a 5-minute cycle, skips already-downloaded files, and pauses if
  free space drops below 10 GB.

### Storage tuning

Both DB writers open SQLite in **WAL mode with `synchronous=OFF`** and a long
`busy_timeout`, so warmup-rs can write while Kodi reads concurrently. The
metadata writer also enables `foreign_keys=ON`, a large page cache and mmap, and
does a passive WAL checkpoint after each batch.

## Requirements

- **CoreELEC, or any aarch64 Linux.** The release binary is a static musl build
  with no runtime dependencies (~5.5 MB).
- **TMDBHelper installed**, so that `ItemDetails.db` and its schema already
  exist. warmup-rs writes into that schema; it does not create it. (It *does*
  create `Textures13.db` tables if needed.)
- **A TMDB API v3 key** — required, passed via `--tmdb-api-key` or the
  `WARMUP_TMDB_API_KEY` env var.
- A **USB SSD** mounted at `/var/media/CACHE_DRIVE` is strongly recommended.
  Internal eMMC works but is slower and lacks space for a full image crawl
  (budget hundreds of GB for images).

## Build

Cross-compile to a static aarch64 binary with [`cross`](https://github.com/cross-rs/cross)
(needs Docker):

```bash
cross build --release --target aarch64-unknown-linux-musl
```

The binary lands at `target/aarch64-unknown-linux-musl/release/warmup-rs`. The
release profile is tuned for size and speed: fat LTO, single codegen unit,
stripped, `panic=abort`.

Run the test suite (host-native, no device needed):

```bash
cargo test
```

Tests cover the SQLite round-trips against a checked-in empty `ItemDetails.db`
schema fixture (`tests/fixtures/ItemDetails-empty.db`), schema fidelity, the
state-DB queue, and the CRC-32 / dimension-parsing units.

## Deploy

```bash
# Copy the binary onto the box.
scp target/aarch64-unknown-linux-musl/release/warmup-rs \
    root@coreelec.local:/storage/tmdb/warmup-rs

# Smoke test it on the device.
ssh root@coreelec.local '/storage/tmdb/warmup-rs --mode=smoke'
# Expected: "smoke ok: rusqlite=3.x.y"
```

### systemd units

This repo ships one unit, [`tmdbhelper-warmup-rs.service`](tmdbhelper-warmup-rs.service),
which runs **metadata mode** with the binary's default paths. It runs at
`Nice=10` and `IOSchedulingPriority=7` (best-effort, lowest) so Kodi keeps
priority for CPU and disk, and checkpoints the WAL before starting.

> **Paths:** the binary defaults `--item-details-db` to Kodi's standard
> `addon_data` path. If your DB lives on a CACHE_DRIVE, set
> `WARMUP_ITEM_DETAILS_DB` (or `--item-details-db`) in the unit accordingly.

To also run the **image** crawler, add a second unit that passes
`--mode=images`, for example `tmdbhelper-warmup-images.service`:

```ini
[Service]
ExecStart=/storage/tmdb/warmup-rs --mode=images
Environment=RUST_LOG=warmup_rs=info,warn
Nice=15
IOSchedulingClass=2
IOSchedulingPriority=7
Restart=on-failure
RestartSec=60
```

Then on the box:

```bash
scp tmdbhelper-warmup-rs.service tmdbhelper-warmup-images.service \
    root@coreelec.local:/storage/.config/system.d/

ssh root@coreelec.local '
  systemctl daemon-reload
  systemctl enable --now tmdbhelper-warmup-rs tmdbhelper-warmup-images
'
```

## Operate

```bash
# Follow the logs.
ssh root@coreelec.local 'journalctl -u tmdbhelper-warmup-rs -f'
ssh root@coreelec.local 'journalctl -u tmdbhelper-warmup-images -f'

# Check progress.
ssh root@coreelec.local '
  sqlite3 /var/media/CACHE_DRIVE/tmdb/scriptcache/state.db \
    "SELECT (SELECT COUNT(*) FROM visited) AS visited, (SELECT COUNT(*) FROM queue) AS queued;"
  sqlite3 /var/media/CACHE_DRIVE/tmdb/Textures13.db "SELECT COUNT(*) FROM texture;"
  du -sh /var/media/CACHE_DRIVE/tmdb/Thumbnails/
'

# Stop gracefully.
ssh root@coreelec.local 'systemctl stop tmdbhelper-warmup-rs tmdbhelper-warmup-images'
```

Running both crawlers at high concurrency can saturate USB SSD write
bandwidth; on a 4-core ARM box with a USB 3.0 SSD the author's sweet spot is
~40 metadata workers + ~25 image workers. Lower `--concurrency` if playback
stutters.

## CLI flags

Every flag has an environment-variable equivalent (handy for systemd units).

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--mode` | `WARMUP_MODE` | `metadata` | `metadata`, `images`, or `smoke` |
| `--tmdb-api-key` | `WARMUP_TMDB_API_KEY` | *(required)* | TMDB API v3 key |
| `--concurrency` | `WARMUP_CONCURRENCY` | `40` | Concurrent TMDB API / CDN fetchers |
| `--batch-size` | `WARMUP_BATCH_SIZE` | `200` | Items popped from the queue per loop (metadata mode) |
| `--state-db` | `WARMUP_STATE_DB` | `/var/media/CACHE_DRIVE/tmdb/scriptcache/state.db` | Priority queue + visited tracking |
| `--item-details-db` | `WARMUP_ITEM_DETAILS_DB` | `…/addon_data/plugin.video.themoviedb.helper/database_07/ItemDetails.db` | TMDBHelper's metadata DB |
| `--textures-db` | `WARMUP_TEXTURES_DB` | `/var/media/CACHE_DRIVE/tmdb/Textures13.db` | Kodi's texture cache DB (images mode) |
| `--thumbnails-dir` | `WARMUP_THUMBNAILS_DIR` | `/var/media/CACHE_DRIVE/tmdb/Thumbnails` | Kodi's thumbnail directory (images mode) |

## SSD TRIM helpers

CoreELEC's kernel 4.9 doesn't expose TRIM for USB-attached SSDs, so heavy image
churn can degrade write performance over time. Two standalone helpers in this
repo work around that by parsing ext4 free-block ranges and issuing TRIM
directly to the drive:

- [`ssd-trim.sh`](ssd-trim.sh) — `hdparm` ATA passthrough (`DATA SET MANAGEMENT`).
- [`ssd-trim.py`](ssd-trim.py) — SCSI `UNMAP` via `sg_unmap` (from `sg3_utils`).

Both are tailored to a Samsung T5 on `/dev/sda`; **review and adjust the device
and offsets before running** — they pass `--please-destroy-my-drive` to hdparm
and write directly to the block device.

## Project layout

```
src/
  main.rs            CLI parsing, mode dispatch
  worker.rs          metadata crawl: fetch → extract children → mega-tx writer
  seed.rs            TMDB popular/trending seed lists
  state.rs           state.db priority queue + visited set
  id.rs              TMDBHelper baseitem id / mediatype string conventions
  api/               TMDB client + typed responses (movie/tv/person/collection)
  cache/             ItemDetails.db writers, one module per entity
  images/            image crawl, Kodi CRC-32 hashing, Textures13.db writer
tests/               round-trip + schema-fidelity + state tests, fixtures
```

## MCP Setup

This project uses a Claude Code MCP server (`@steipete/claude-code-mcp`) to enable
agent-in-agent delegation for complex tasks from other MCP clients.

### Current MCP server

| Server | Type | Scope | Status |
|--------|------|-------|--------|
| `claude-code-mcp` | stdio | local | `✔ Connected` |

### Tool available

| Tool | Purpose |
|------|---------|
| `claude_code` | Delegates coding, file, Git, and terminal tasks to a separate Claude Code subprocess |

### Add it

```bash
claude mcp add claude-code-mcp -- npx -y @steipete/claude-code-mcp
```

### Verify it

```bash
claude mcp get claude-code-mcp
```

To remove:

```bash
claude mcp remove claude-code-mcp -s local
```

## License

GPLv3 — see [LICENSE](LICENSE).
