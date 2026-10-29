+++
title = "warmup-rs"
nav_title = "Overview"
template = "home"
toc = false
description = "warmup-rs builds TMDBHelper's metadata database and Kodi's image cache by writing directly to SQLite, without Kodi or the add-on in the loop."
eyebrow = "TMDBHelper cache builder · Rust"
headline = "A full TMDBHelper cache,"
headline_accent = "built without Kodi."
lede = "warmup-rs crawls TMDB and writes the same rows TMDBHelper writes, straight into SQLite. Run it on a desktop to build a ready-made ItemDetails.db and images folder, or on a CoreELEC box to keep Kodi's own caches warm."
facts = ["One static binary", "Restart-safe crawls", "Kodi-compatible image cache", "GPL v3 or later"]
+++

## Start here

<div class="cards">
<a class="card" href="install.html"><p class="eyebrow">Get started</p><h3>Install and build</h3><p>Compile from source with Cargo or Just, or cross-compile a static Linux binary.</p></a>
<a class="card" href="quick-start.html"><p class="eyebrow">Get started</p><h3>Quick start</h3><p>Build a populated database and images folder under ./output on your desktop.</p></a>
<a class="card" href="kodi-box.html"><p class="eyebrow">Guides</p><h3>Run it on a Kodi box</h3><p>Deploy to CoreELEC, LibreELEC, or OpenELEC and run both crawlers as systemd services.</p></a>
<a class="card" href="how-it-works.html"><p class="eyebrow">Overview</p><h3>How it works</h3><p>The crawl order, the batch writer, and how images get Kodi's cache filenames.</p></a>
<a class="card" href="cli.html"><p class="eyebrow">Reference</p><h3>Command-line options</h3><p>Every flag, its environment variable, and its default on each platform.</p></a>
<a class="card" href="justfile.html"><p class="eyebrow">Reference</p><h3>Just recipes</h3><p>Test, lint, build for each target, package Docker images, and check prose.</p></a>
</div>

## What it does

[TMDBHelper](https://github.com/jurialmunkey/plugin.video.themoviedb.helper) is a Kodi add-on that shows movie, TV, and people pages from TMDB. It caches what it fetches in two places:

- **`ItemDetails.db`**, TMDBHelper's metadata cache of about 30 tables, such as `movie`, `tvshow`, `person`, `castmember`, `art`, and `ratings`.
- **`Textures13.db` and the thumbnails folder**, Kodi's image cache, so artwork shows without a download.

Filling those caches through Kodi is slow, because every item goes through the Python add-on and JSON-RPC. warmup-rs skips that layer. It asks TMDB for everything about an item in one request, writes the rows TMDBHelper expects, and commits them in large batches. On a 4-core ARM box with a USB 3.0 SSD, it warms about 40 metadata items and 45 images a second.

## Where it writes

warmup-rs picks its default paths from the system it runs on:

| System | Databases | Images |
|---|---|---|
| Windows, macOS, or desktop Linux | `./output` | `./output/images` |
| CoreELEC, LibreELEC, or OpenELEC | Kodi's `addon_data` folder and `/var/media/CACHE_DRIVE/tmdb` | `/var/media/CACHE_DRIVE/tmdb/Thumbnails` |

On a desktop, warmup-rs creates the output folder and an empty `ItemDetails.db` from TMDBHelper's schema, so you don't need Kodi or the add-on installed. Copy the results to a Kodi device when the crawl finishes. See [Command-line options](cli.md) for every path.
