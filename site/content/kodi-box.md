+++
title = "Run it on a Kodi box"
description = "Deploy warmup-rs to CoreELEC, LibreELEC, or OpenELEC, where it writes straight into Kodi's TMDBHelper and image caches."
eyebrow = "Guides"
lede = "On a Kodi appliance, warmup-rs keeps its original defaults and writes into the caches that Kodi reads. It runs at low priority next to Kodi."
+++

## Before you start

- **TMDBHelper**, installed and run once, so `ItemDetails.db` exists. On a Kodi appliance, warmup-rs writes into the add-on's schema and doesn't create the database.
- **A USB SSD**, mounted at `/var/media/CACHE_DRIVE`. Internal eMMC works, but it's slower and too small for a full image crawl.
- **An arm64 binary**, from `just build-release-arm`.

warmup-rs treats the system as a Kodi appliance when `/etc/os-release` names CoreELEC, LibreELEC, or OpenELEC in `ID`, `ID_LIKE`, or `NAME`.

## Default paths

| File | Path |
|---|---|
| `ItemDetails.db` | `/storage/.kodi/userdata/addon_data/plugin.video.themoviedb.helper/database_07/ItemDetails.db` |
| `state.db` | `/var/media/CACHE_DRIVE/tmdb/scriptcache/state.db` |
| `Textures13.db` | `/var/media/CACHE_DRIVE/tmdb/Textures13.db` |
| Thumbnails | `/var/media/CACHE_DRIVE/tmdb/Thumbnails` |

If your `ItemDetails.db` lives on the cache drive, set `WARMUP_ITEM_DETAILS_DB`, or pass `--item-details-db`, in the systemd unit.

## Copy and test the binary

```bash
scp target/aarch64-unknown-linux-musl/release/warmup-rs \
    root@coreelec.local:/storage/tmdb/warmup-rs

ssh root@coreelec.local '/storage/tmdb/warmup-rs --mode=smoke'
# Prints: smoke OK: rusqlite=3.x.y
```

## Install the services

The repository ships `tmdbhelper-warmup-rs.service`, which runs metadata mode with the default paths. It runs at `Nice=10` and the lowest I/O priority in the best-effort class, so Kodi keeps priority for CPU and disk, and it checkpoints the WAL before it starts.

The unit needs your API key. Add it to the `[Service]` section:

```ini
Environment=WARMUP_TMDB_API_KEY=<your key>
```

For the image crawl, add a second unit, for example `tmdbhelper-warmup-images.service`:

```ini
[Unit]
Description=TMDBHelper image cache warmup (Rust)
After=kodi.service

[Service]
ExecStart=/storage/tmdb/warmup-rs --mode=images
Environment=RUST_LOG=warmup_rs=info,warn
Nice=15
IOSchedulingClass=2
IOSchedulingPriority=7
Restart=on-failure
RestartSec=60

[Install]
WantedBy=multi-user.target
```

Then copy both units and start them:

```bash
scp tmdbhelper-warmup-rs.service tmdbhelper-warmup-images.service \
    root@coreelec.local:/storage/.config/system.d/

ssh root@coreelec.local '
  systemctl daemon-reload
  systemctl enable --now tmdbhelper-warmup-rs tmdbhelper-warmup-images
'
```

See [Operate](operate.md) for logs, progress, and tuning.
