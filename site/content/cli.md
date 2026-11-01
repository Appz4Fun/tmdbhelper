+++
title = "Command-line options"
description = "Every warmup-rs flag, its environment variable, and its default on desktop systems and Kodi appliances."
eyebrow = "Reference"
lede = "Every flag has an environment variable, which is handy in systemd units. A flag on the command line wins over its variable."
+++

## Options

| Flag | Variable | Default | Description |
|---|---|---|---|
| `--mode` | `WARMUP_MODE` | `metadata` | `metadata`, `images`, `fanart`, or `smoke`. |
| `--tmdb-api-key` | `WARMUP_TMDB_API_KEY` | none | TMDB API v3 key. Required in metadata mode. |
| `--fanart-api-key` | `WARMUP_FANART_API_KEY` | none | fanart.tv API key. Required in fanart mode. |
| `--fanart-client-key` | `WARMUP_FANART_CLIENT_KEY` | none | fanart.tv personal key, sent alongside a project's API key in fanart mode. |
| `--concurrency` | `WARMUP_CONCURRENCY` | `20` | Number of concurrent requests to TMDB. |
| `--language` | `WARMUP_LANGUAGE` | `en-US` | The add-on's language setting. TMDBHelper ignores rows in any other language. |
| `--all-ids` | `WARMUP_ALL_IDS` | off | Queues every title, person, and collection from TMDB's daily ID lists in metadata mode. |
| `--min-free-gb` | `WARMUP_MIN_FREE_GB` | `10` | Pauses when the output disk has less free space than this many GB. |
| `--batch-size` | `WARMUP_BATCH_SIZE` | `200` | Number of items to take from the queue per loop in metadata mode. |
| `--output-dir` | `WARMUP_OUTPUT_DIR` | `./output` | Base folder for the default paths on desktop systems. |
| `--state-db` | `WARMUP_STATE_DB` | See [Paths](#paths) | Crawler queue and visited-item database. |
| `--item-details-db` | `WARMUP_ITEM_DETAILS_DB` | See [Paths](#paths) | TMDBHelper's metadata database. |
| `--textures-db` | `WARMUP_TEXTURES_DB` | See [Paths](#paths) | Kodi's texture cache database, used in images mode. |
| `--thumbnails-dir` | `WARMUP_THUMBNAILS_DIR` | See [Paths](#paths) | Folder for downloaded images, used in images mode. |

`RUST_LOG` sets the log level. The default is `warmup_rs=info,warn`.

## Paths

| File | Desktop | CoreELEC, LibreELEC, OpenELEC |
|---|---|---|
| `ItemDetails.db` | `<output-dir>/ItemDetails.db` | `/storage/.kodi/userdata/addon_data/plugin.video.themoviedb.helper/database_07/ItemDetails.db` |
| `state.db` | `<output-dir>/state.db` | `/var/media/CACHE_DRIVE/tmdb/scriptcache/state.db` |
| `Textures13.db` | `<output-dir>/Textures13.db` | `/var/media/CACHE_DRIVE/tmdb/Textures13.db` |
| Images | `<output-dir>/images` | `/var/media/CACHE_DRIVE/tmdb/Thumbnails` |

On a desktop, warmup-rs creates the folders and, if it's missing, an `ItemDetails.db` with TMDBHelper's schema. On a Kodi appliance, it creates nothing in Kodi's folders, so TMDBHelper must have created `ItemDetails.db` first.

## Help text

This is the output of `warmup-rs --help`:

<!-- cli-help -->
