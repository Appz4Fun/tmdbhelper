# warmup-rs

A small, static Rust binary that builds the two caches used by Kodi's
[TMDBHelper](https://github.com/jurialmunkey/plugin.video.themoviedb.helper)
add-on by writing directly to SQLite. It doesn't use JSON-RPC, the Python
add-on, or Kodi itself.

1. **`ItemDetails.db`**: TMDBHelper's metadata cache, about 30 tables such as
   `movie`, `tvshow`, `season`, `episode`, `person`, `castmember`, and `art`.
2. **`Textures13.db` and the thumbnails folder**: Kodi's image cache, so the
   interface never has to fetch artwork on demand.

Full documentation: <https://appz4fun.github.io/tmdbhelper/>

## Why

Filling these caches through Kodi's add-on layer is slow, because every item
goes through Python and JSON-RPC. warmup-rs skips that layer and writes the same
rows TMDBHelper writes. It fetches each item from TMDB with
`append_to_response`, so one request returns images, videos, credits, external
IDs, translations, keywords, similar titles, recommendations, and watch
providers. On a 4-core ARM box with a USB 3.0 SSD, it warms about 40 metadata
items and 45 images a second.

## Quick start

```bash
just build
export WARMUP_TMDB_API_KEY=<your key>
./target/release/warmup-rs                # metadata crawl
./target/release/warmup-rs --mode=images  # image crawl, in a second terminal
```

The key is the v3 **API Key** from **Settings > API** on
[themoviedb.org](https://www.themoviedb.org), and it's the only credential you
need. To keep it out of your shell history, see
[Save the key](https://appz4fun.github.io/tmdbhelper/quick-start.html#save-the-key).

On Windows, macOS, and desktop Linux, everything goes under `./output` in the
current folder:

```text
output/
  ItemDetails.db    TMDBHelper's metadata cache, created from the bundled schema
  state.db          the crawl queue and visited items
  Textures13.db     Kodi's image cache database
  images/           cached images, named the way Kodi names them
```

On CoreELEC, LibreELEC, and OpenELEC, the defaults point at Kodi's own files
instead:

| File | Default on a Kodi appliance |
|------|-----------------------------|
| `ItemDetails.db` | `/storage/.kodi/userdata/addon_data/plugin.video.themoviedb.helper/database_07/ItemDetails.db` |
| `state.db` | `/var/media/CACHE_DRIVE/tmdb/scriptcache/state.db` |
| `Textures13.db` | `/var/media/CACHE_DRIVE/tmdb/Textures13.db` |
| Thumbnails | `/var/media/CACHE_DRIVE/tmdb/Thumbnails` |

warmup-rs treats the system as a Kodi appliance when `/etc/os-release` names
CoreELEC, LibreELEC, or OpenELEC. On an appliance, TMDBHelper must have created
`ItemDetails.db` first. If your `ItemDetails.db` lives on the cache drive, set
`WARMUP_ITEM_DETAILS_DB`, or pass `--item-details-db`, in the systemd unit.

## Modes

| Mode | What it does |
|------|--------------|
| `metadata` | The default. Crawls TMDB and writes `ItemDetails.db`, starting from TMDB's trending and popular lists and following credits, similar titles, recommendations, filmographies, and collections up to two hops away. It also fetches every season and episode of each show. With `--all-ids`, it queues everything in TMDB's daily ID lists. |
| `images` | Reads every art path from `ItemDetails.db`, downloads each image at `original` size, saves it in the images folder, and registers every URL TMDBHelper can request in `Textures13.db`, so Kodi gets a cache hit at any `artwork_quality` setting. |
| `fanart` | Fetches fanart.tv art for the movies and shows in `ItemDetails.db`. TMDBHelper needs this when its fanart.tv lookup is on, or it ignores those rows. |
| `smoke` | Opens an in-memory SQLite database, builds an HTTP client, and exits. Use it to check that the binary runs on a device. |

Both real modes are restart-safe, and you can run them at the same time. See
[How it works](https://appz4fun.github.io/tmdbhelper/how-it-works.html) for
the crawl order, batching, and image naming.

## Options

Every flag has an environment variable. Run `warmup-rs --help` for the full
text.

| Flag | Variable | Default |
|------|----------|---------|
| `--mode` | `WARMUP_MODE` | `metadata` |
| `--tmdb-api-key` | `WARMUP_TMDB_API_KEY` | Required in metadata mode |
| `--fanart-api-key` | `WARMUP_FANART_API_KEY` | Required in fanart mode |
| `--fanart-client-key` | `WARMUP_FANART_CLIENT_KEY` | None |
| `--concurrency` | `WARMUP_CONCURRENCY` | `20` |
| `--language` | `WARMUP_LANGUAGE` | `en-US` |
| `--all-ids` | `WARMUP_ALL_IDS` | Off |
| `--min-free-gb` | `WARMUP_MIN_FREE_GB` | `10` |
| `--batch-size` | `WARMUP_BATCH_SIZE` | `200` |
| `--output-dir` | `WARMUP_OUTPUT_DIR` | `./output`, desktop systems only |
| `--state-db` | `WARMUP_STATE_DB` | Depends on the platform |
| `--item-details-db` | `WARMUP_ITEM_DETAILS_DB` | Depends on the platform |
| `--textures-db` | `WARMUP_TEXTURES_DB` | Depends on the platform |
| `--thumbnails-dir` | `WARMUP_THUMBNAILS_DIR` | Depends on the platform |

## Build

The [Justfile](Justfile) wraps the usual commands. Run `just` to list them.

| Recipe | What it does |
|--------|--------------|
| `just test`, `just test-live`, `just fmt`, `just lint` | Runs the offline tests, the tests that call TMDB, `cargo fmt`, and `cargo fmt --check` plus Clippy. |
| `just run`, `just help` | Runs the release build with its defaults, or prints its help. |
| `just build`, `just build-release`, `just build-debug` | Builds for this computer. |
| `just build-release-x86`, `just build-release-arm` | Builds static musl binaries for `x86_64` and `aarch64` Linux with [cross](https://github.com/cross-rs/cross). |
| `just build-release-win` | Builds an `x86_64` Windows binary with cross. Beta, see [TODO.md](TODO.md). |
| `just build-opkg` | Builds an Entware opkg package and feed for arm64. |
| `just install` | Installs the release build into `~/.cargo/bin`. |
| `just build-docker`, `just build-docker-x86`, `just build-docker-arm` | Builds a Docker image for this computer, `linux/amd64`, or `linux/arm64`. |
| `just build-dockerx` | Builds `linux/amd64` and `linux/arm64` with Buildx and merges the manifest. |
| `just push-docker <repo> [tag]` | Builds both architectures and pushes them to a registry. |
| `just vale-lint`, `just vale-fmt` | Checks prose with the Vale Google style, or applies Vale's automatic fixes first. |
| `just site`, `just site-serve` | Builds the documentation site, or builds and serves it locally. |

## Deploy to a Kodi box

On CoreELEC with Entware, install the opkg package. Its feed and the setup steps
are on the
[Entware package](https://appz4fun.github.io/tmdbhelper/opkg.html) page.

To deploy by hand:

```bash
just build-release-arm
scp target/aarch64-unknown-linux-musl/release/warmup-rs \
    root@coreelec.local:/storage/tmdb/warmup-rs
ssh root@coreelec.local '/storage/tmdb/warmup-rs --mode=smoke'
# Prints: smoke OK: rusqlite=3.x.y
```

The repository ships one systemd unit,
[`tmdbhelper-warmup-rs.service`](tmdbhelper-warmup-rs.service), which runs
metadata mode at low CPU and I/O priority. Add your API key to it, and see
[Run it on a Kodi box](https://appz4fun.github.io/tmdbhelper/kodi-box.html)
for a second unit that runs images mode.

## SSD TRIM helpers

CoreELEC's 4.9 kernel doesn't expose TRIM for USB-attached SSDs. Two helpers,
[`ssd-trim.sh`](ssd-trim.sh) with `hdparm` and [`ssd-trim.py`](ssd-trim.py)
with `sg_unmap`, send TRIM to the drive directly. Both expect a Samsung T5 at
`/dev/sda`. **Review the device and offsets before you run them**, because they
write to the block device.

## Contributing

Run `just test`, `just lint`, and `just vale-lint` before you open a pull
request. All hand-written text, including the docs, `--help`, the recipe list,
and log messages, follows the
[Google developer documentation style guide](https://developers.google.com/style).
Text that comes from TMDB isn't checked. See
[Contributing](https://appz4fun.github.io/tmdbhelper/contributing.html).

### Claude Code MCP server

The project can use the `@steipete/claude-code-mcp` Claude Code MCP server, so
other MCP clients can hand coding, file, Git, and terminal tasks to a separate
Claude Code process through its `claude_code` tool.

```bash
claude mcp add claude-code-mcp -- npx -y @steipete/claude-code-mcp
claude mcp get claude-code-mcp
claude mcp remove claude-code-mcp -s local
```

## License

GPLv3 or later. See [LICENSE](LICENSE).
