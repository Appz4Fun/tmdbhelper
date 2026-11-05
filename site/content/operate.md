+++
title = "Operate"
description = "Follow warmup-rs logs, check crawl progress, tune concurrency, and keep a USB SSD healthy with the TRIM helpers."
eyebrow = "Guides"
lede = "Both crawls log to the journal on a Kodi box and to the terminal on a desktop. Their progress lives in plain SQLite tables."
+++

## Logs

warmup-rs logs with `tracing`. Set `RUST_LOG` to change the level, for example `RUST_LOG=warmup_rs=debug`. The default is `warmup_rs=info,warn`.

```bash
ssh root@coreelec.local 'journalctl -u tmdbhelper-warmup-rs -f'
ssh root@coreelec.local 'journalctl -u tmdbhelper-warmup-images -f'
```

The metadata crawl logs a line like this every 100 items:

```text
written=4200 visited=4180 queue=91234 rate=41.2/s batch=200 avg_write=3ms avg_state=1ms
```

## Progress

```bash
sqlite3 output/state.db \
  "SELECT (SELECT COUNT(*) FROM visited) AS visited, (SELECT COUNT(*) FROM queue) AS queued;"
sqlite3 output/Textures13.db "SELECT COUNT(*) FROM texture;"
du -sh output/images
```

On a Kodi box, use the paths under `/var/media/CACHE_DRIVE/tmdb` instead.

## Stop

On a desktop, press Ctrl+C. On a Kodi box:

```bash
ssh root@coreelec.local 'systemctl stop tmdbhelper-warmup-rs tmdbhelper-warmup-images'
```

Both crawls resume from where they stopped, including the items that were in flight.

## Tune concurrency

Running both crawls at high concurrency can saturate a USB SSD. On a 4-core ARM box with a USB 3.0 SSD, about 20 metadata requests and 25 image requests at once work well. Lower `--concurrency` if playback stutters. With more than about 20, a fast connection hits TMDB's rate limit, which slows the crawl down.

## SSD TRIM helpers

CoreELEC's 4.9 kernel doesn't expose TRIM for USB-attached SSDs, so heavy image churn can slow writes over time. Two helpers in the repository read ext4 free-block ranges and send TRIM to the drive:

- `ssd-trim.sh` uses `hdparm` ATA passthrough.
- `ssd-trim.py` uses SCSI `UNMAP` through `sg_unmap` from `sg3_utils`.

> [!CAUTION] Check the device first
> Both helpers expect a Samsung T5 at `/dev/sda`. Review the device and offsets before you run them. They write to the block device, and `ssd-trim.sh` passes `--please-destroy-my-drive` to `hdparm`.
