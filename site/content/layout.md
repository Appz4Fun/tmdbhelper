+++
title = "Project layout"
description = "Where each part of warmup-rs lives in the repository, from the TMDB client to the Kodi image cache writer."
eyebrow = "Reference"
lede = "warmup-rs is one crate with a library and a binary. The binary parses flags and hands off to the library."
+++

## Source

| Path | What it holds |
|---|---|
| `src/main.rs` | Flag parsing, path defaults, and mode dispatch. |
| `src/paths.rs` | Platform detection, default paths, and creating a fresh `ItemDetails.db`. |
| `src/worker.rs` | The metadata crawl: fetch, find children, and the batch writer. |
| `src/seed.rs` | TMDB's trending and popular lists, and its daily ID lists, used as seeds. |
| `src/fanart.rs` | The fanart.tv crawl. |
| `src/state.rs` | The `state.db` queue and visited set. |
| `src/id.rs` | TMDBHelper's item ID and media type conventions. |
| `src/api/` | The TMDB client, which sorts failures by what the crawl does next. |
| `src/cache/` | The `ItemDetails.db` writer, a port of TMDBHelper's `mappings.py`. |
| `src/images/` | The image crawl, Kodi's CRC-32 hash, and the `Textures13.db` writer. |

## Everything else

| Path | What it holds |
|---|---|
| `schema/ItemDetails.sql` | TMDBHelper's schema and seed rows, built into the binary. |
| `tests/` | Add-on parity, live TMDB, path, and queue tests, plus fixtures. |
| `site/` | This documentation site. |
| `scripts/` | Helpers for the Vale recipes, the opkg package build, and checking warmup-rs against a TMDBHelper checkout. |
| `Justfile` | The recipes. See [Just recipes](justfile.md). |
| `Dockerfile` | The multi-stage image build. |
| `packaging/opkg/` | The Entware package's systemd units, settings file, and install scripts. |
| `tmdbhelper-warmup-rs.service` | The systemd unit for metadata mode. |
| `ssd-trim.sh`, `ssd-trim.py` | TRIM helpers for USB SSDs. See [Operate](operate.md#ssd-trim-helpers). |
