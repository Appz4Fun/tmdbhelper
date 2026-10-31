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

1. **Seeds.** When the queue is empty, warmup-rs reads TMDB's `trending/all/week`, `movie/popular`, `movie/now_playing`, `movie/upcoming`, `tv/popular`, and `person/popular` lists, a few pages each. With `--all-ids`, it also queues every title, person, and collection from TMDB's daily ID lists, leaving out adult items.
2. **Queue.** `state.db` holds the work queue and a set of visited items. Items come off the queue by depth first, then by popularity, so popular, shallow items are ready first. An item stays in the queue until it's written, so stopping the crawl at any moment loses nothing.
3. **Fetch.** Each item is one TMDB request that uses `append_to_response` to include the same data TMDBHelper asks for, such as images, videos, credits, translations, and watch providers. It adds similar titles and recommendations for the crawl. A TV show also fetches each of its episode groups. The `--concurrency` flag limits how many requests run at once.
4. **Expand.** A movie adds its cast, crew, similar titles, recommendations, and collection to the queue. A TV show adds its cast, crew, similar titles, recommendations, and seasons. A season adds its episodes, and an episode adds its guest stars and crew. A person adds their movie and TV credits, and a collection adds its movies.
5. **Depth limit.** The crawl stops two hops from the seeds. warmup-rs writes items at depth 2 but doesn't add their children. Seasons and episodes stay at their show's depth and popularity, so every show gets all of its episodes.
6. **Write.** One writer task collects up to 200 items and commits them in a single transaction. Batching is what makes the crawl fast on USB storage, where each separate commit costs about 10 ms.
7. **Failures.** A network error or rate limit puts an item back on the queue. An item TMDB doesn't have (a 404) is dropped, and an item that fails 5 times is dropped until its next refresh. A rejected API key stops the crawl with exit code 2.
8. **Refresh.** When the queue runs dry, items older than 30 days go back on it too, and the seeder runs again. If no work remains, warmup-rs waits an hour and checks again.

## Image crawl

1. warmup-rs reads every art path from `ItemDetails.db`, most popular title first. It skips SVG logos, because TMDBHelper only shows PNG logos.
2. It downloads each image once, at `original` size, the only size TMDB serves unchanged. At startup, it deletes any file an older version downloaded at `w1280`, so the original replaces it.
3. It names the file the way Kodi does: the CRC-32/MPEG-2 hash of the lowercase URL, stored as `<first hex digit>/<hash>.<extension>`. The hash is a port of Kodi's `xbmc/utils/Crc32.cpp`, and the tests check it against real `Textures13.db` filenames.
4. It registers several URL sizes for each image, all pointing at the one file. These are the URLs TMDBHelper requests at each `artwork_quality` setting. For example, it registers a poster as `original`, `w780`, `w500`, and `w342`.
5. It reads image sizes from the JPEG or PNG header, without decoding the image, to fill Kodi's `sizes` table.
6. It scans again every 5 minutes, and it pauses while the disk has less free space than `--min-free-gb`, 10 GB by default. Metadata mode pauses the same way.

## SQLite settings

Both writers open SQLite in WAL mode with `synchronous=OFF` and a long busy timeout, so warmup-rs can write while Kodi reads. The metadata writer leaves `foreign_keys` off, so a refresh can't cascade into child rows, and creates each parent before its children. It also turns on a large page cache and memory-mapped I/O, and runs a passive WAL checkpoint after each batch.

## Matching TMDBHelper

TMDBHelper only reads a cached row when the row is unexpired, has a full data level, and matches the add-on's language, so warmup-rs writes each `baseitem` with `--language`, `en-US` by default. Set `--language` to match the add-on's setting. It also leaves `ratings` empty, because a row there stops the add-on fetching ratings from its other sources.

`schema/ItemDetails.sql` sets the add-on's schema version. Keep it current: the add-on drops every table in a database with an older non-zero version. warmup-rs refuses to write to a database with a different version, and `scripts/addon-schema.py` rebuilds the schema from a TMDBHelper checkout.

warmup-rs writes no `fanart_tv` rows and no ratings from other services, since those need other API keys. If you turn on the add-on's fanart.tv lookup, it fetches each item again the first time it needs it.
