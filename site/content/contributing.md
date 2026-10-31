+++
title = "Contributing"
description = "How to test, lint, and check prose before you send a change to warmup-rs."
eyebrow = "Project"
lede = "Run the tests, the linters, and the prose check before you open a pull request. The checks in CI are the same ones."
+++

## Checks

```bash
just test
just lint
just vale-lint
just site
```

## Prose style

All text that warmup-rs or this repository writes follows the [Google developer documentation style guide](https://developers.google.com/style), checked by [Vale](https://vale.sh) with the Google package. That covers:

- the Markdown files, including this site and `README.md`
- `warmup-rs --help`
- the `just --list` recipe descriptions
- log and error messages in `src/`
- comments and messages in `packaging/`

Text that comes from TMDB, such as a plot or a biography, isn't checked.

`just vale-lint` collects the help text, the recipe list, the source strings, and the text in `packaging/` into `target/vale`, then runs Vale on them and on the docs. It fails on errors and shows warnings. `just vale-fmt` applies the fixes that Vale can make by itself, such as word substitutions, and leaves the rest for you.

Project words, such as product names, go in `.vale/styles/config/vocabularies/WarmupRs/accept.txt`.

## Schema changes

The binary embeds `schema/ItemDetails.sql`, which is a dump of `tests/fixtures/ItemDetails-empty.db`. If TMDBHelper changes its schema, update the fixture, then regenerate the dump:

```bash
sqlite3 tests/fixtures/ItemDetails-empty.db .dump > schema/ItemDetails.sql
```

## Releases

The `docs` workflow rebuilds the opkg feed from `main`. opkg upgrades only to a newer version, so raise `version` in `Cargo.toml` when a change should reach installed packages.

## The site

The site source is in `site/`. Pages are Markdown with TOML front matter in `site/content`, and `site/site.toml` sets the navigation. `site/build.py` renders them with the templates and the Oxide theme from the RustyGet documentation site. The `docs` workflow builds and checks the site on every push, and deploys it to GitHub Pages from `main`.
