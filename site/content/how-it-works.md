+++
title = "How it works"
description = "How warmup-rs seeds and walks TMDB, batches its writes, and names cached images the way Kodi does."
eyebrow = "Overview"
lede = "warmup-rs has two independent modes. Metadata mode fills ItemDetails.db, and images mode reads that database and fills Kodi's image cache."
+++

## Modes

| Mode | What it does |
|---|---|
| `metadata` | The default. Crawls TMDB and writes `ItemDetails.db`. |
| `images` | Reads every art path from `ItemDetails.db`, downloads each image, and registers it in `Textures13.db`. |
| `smoke` | Opens an in-memory SQLite database and builds an HTTP client, then exits. Use it to check that the binary runs on a device. |

Both real modes are restart-safe. Metadata mode resumes from its queue in `state.db`, and images mode skips files already on disk and URLs already in `Textures13.db`. The two modes are separate processes, so you can run them at the same time.

## Metadata crawl

1. **Seeds.** When the queue is empty, warmup-rs reads TMDB's `trending/all/week`, `movie/popular`, `movie/now_playing`, `movie/upcoming`, `tv/popular`, and `person/popular` lists, a few pages each.
2. **Queue.** `state.db` holds the work queue and a set of visited items. Items come off the queue by depth first, then by popularity, so popular, shallow items are ready first.
3. **Fetch.** Each item is one TMDB request that uses `append_to_response` to include images, videos, credits, external IDs, translations, keywords, similar titles, recommendations, and watch providers. The `--concurrency` flag limits how many requests run at once.
4. **Expand.** A movie adds its cast, crew, similar titles, recommendations, and collection to the queue. A TV show adds its cast, crew, similar titles, and recommendations. A person adds their combined filmography, and a collection adds its movies.
5. **Depth limit.** The crawl stops two hops from the seeds. warmup-rs writes items at depth 2 but doesn't add their children.
6. **Write.** One writer task collects up to 200 items and commits them in a single transaction. Batching is what makes the crawl fast on USB storage, where each separate commit costs about 10 ms.
7. **Refresh.** Items that fail go back on the queue. When the queue runs dry, items older than 30 days go back on it too, and the seeder runs again. If no work remains, warmup-rs waits an hour and checks again.

## Image crawl

1. warmup-rs reads `SELECT DISTINCT icon, type FROM art` from `ItemDetails.db`.
2. It downloads each image once, at `original` size for backdrops and `w1280` for everything else.
3. It names the file the way Kodi does: the CRC-32/MPEG-2 hash of the lowercase URL, stored as `<first hex digit>/<hash>.<extension>`. The hash is a port of Kodi's `xbmc/utils/Crc32.cpp`, and the tests check it against real `Textures13.db` filenames.
4. It registers several URL sizes for each image, all pointing at the one file. For example, it registers a backdrop as `original`, `w1280`, and `w780`, so Kodi finds a cache hit at any `imageres` or `fanartres` setting.
5. It reads image sizes from the JPEG or PNG header, without decoding the image, to fill Kodi's `sizes` table.
6. It scans again every 5 minutes, and it pauses while the images folder has less than 10 GB free.

## SQLite settings

Both writers open SQLite in WAL mode with `synchronous=OFF` and a long busy timeout, so warmup-rs can write while Kodi reads. The metadata writer also turns on `foreign_keys`, a large page cache, and memory-mapped I/O, and runs a passive WAL checkpoint after each batch.
