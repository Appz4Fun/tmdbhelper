+++
title = "Quick start"
description = "Build a populated TMDBHelper ItemDetails.db and a Kodi image cache under ./output on Windows, macOS, or Linux."
eyebrow = "Get started"
lede = "On a desktop, warmup-rs writes everything to an output folder in the current directory. You need a TMDB API key and some disk space."
+++

## Get a TMDB API key

Create a free account at [themoviedb.org](https://www.themoviedb.org), then request an API key under **Settings > API**. warmup-rs uses the v3 **API Key**, a 32-character hex string, not the longer **API Read Access Token**. It's the only credential warmup-rs needs.

## Save the key

warmup-rs reads the key from `--tmdb-api-key` or `WARMUP_TMDB_API_KEY`, and it doesn't look for a key file. To avoid retyping the key, and to keep it out of your shell history, save it in a private file. This works in bash and zsh. Paste the key at the prompt, which doesn't show what you paste:

```bash
mkdir -p ~/.config/tmdbhelper
(umask 077; printf 'TMDB API key: '; read -rs key; echo; printf '%s\n' "$key" > ~/.config/tmdbhelper/tmdb.key)
```

Keep the file out of any Git repository.

## Run the metadata crawl

From the folder where you want the output:

```bash
export WARMUP_TMDB_API_KEY="$(cat ~/.config/tmdbhelper/tmdb.key)"
warmup-rs
```

In Windows PowerShell, set the variable with `$env:WARMUP_TMDB_API_KEY = "<your key>"`.

From a clone of the repository, `just run` does the same with a release build.

warmup-rs creates this layout:

```text
output/
  ItemDetails.db    TMDBHelper's metadata cache
  state.db          the crawl queue and visited items
  Textures13.db     Kodi's image cache database, from images mode
  images/           cached images, from images mode
```

It logs its progress every 100 items. The crawl doesn't end on its own: after it finishes the queue, it seeds again and refreshes items older than 30 days. Press Ctrl+C to stop it. The next run picks up where it left off.

## Run the image crawl

In a second terminal, from the same folder:

```bash
warmup-rs --mode=images
```

Images mode doesn't need an API key. It reads the art paths that the metadata crawl wrote, so start it after the metadata crawl has written some items. It scans for new art every 5 minutes.

## Collect everything

The default crawl covers popular titles and everything two hops from them. To queue every title, person, and collection TMDB lists, leaving out adult items:

```bash
warmup-rs --all-ids --min-free-gb=100
```

This is about 6.5 million items plus all of their seasons and episodes, which takes weeks. The full image set can take several TB. The most popular items come first, and both crawls pause at `--min-free-gb`, so you can stop whenever you have enough.

## Change the output folder

Use `--output-dir` or `WARMUP_OUTPUT_DIR` to write somewhere else:

```bash
warmup-rs --output-dir=/mnt/media/tmdb
```

Each path flag, such as `--item-details-db`, overrides one file. See [Command-line options](cli.md).

## Use the results in Kodi

Stop both crawls, then copy the files to the Kodi device:

| File | Kodi location |
|---|---|
| `output/ItemDetails.db` | `userdata/addon_data/plugin.video.themoviedb.helper/database_07/ItemDetails.db` |
| `output/Textures13.db` | `userdata/Database/Textures13.db` |
| `output/images/` | `userdata/Thumbnails/` |

Stop Kodi before you replace its databases, and update TMDBHelper first: warmup-rs writes schema version 50, from TMDBHelper 6.19.

Both databases use a write-ahead log, so copy them only after you merge it:

```bash
sqlite3 output/ItemDetails.db 'PRAGMA wal_checkpoint(TRUNCATE);'
sqlite3 output/Textures13.db 'PRAGMA wal_checkpoint(TRUNCATE);'
``` To keep the caches warm on the device itself, see [Run it on a Kodi box](kodi-box.md).
