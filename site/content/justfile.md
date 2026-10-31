+++
title = "Just recipes"
description = "The warmup-rs Justfile recipes for testing, linting, building each target, packaging Docker images, and checking prose with Vale."
eyebrow = "Reference"
lede = "Run just with no arguments to list the recipes. Recipes that take arguments show them after the name."
+++

## Develop

| Recipe | What it does |
|---|---|
| `just test` | Runs `cargo test`. Extra arguments go to Cargo, for example `just test paths`. |
| `just fmt` | Formats the code with `cargo fmt`. |
| `just lint` | Checks formatting and runs Clippy with warnings as errors. |
| `just run` | Runs a release build with the defaults. Extra flags go to warmup-rs, for example `just run --mode=images`. |
| `just help` | Prints `warmup-rs --help`. |

## Build

| Recipe | What it does |
|---|---|
| `just build` | Builds the release binary for this computer. Same as `just build-release`. |
| `just build-release` | Builds the release binary for this computer. |
| `just build-debug` | Builds the debug binary for this computer. |
| `just build-release-x86` | Builds a static `x86_64-unknown-linux-musl` binary with cross. |
| `just build-release-arm` | Builds a static `aarch64-unknown-linux-musl` binary with cross. |
| `just build-release-win` | Builds an `x86_64-pc-windows-gnu` binary with cross. Beta. |
| `just build-opkg` | Builds an Entware opkg package and feed in `target/opkg`. See [Entware package](opkg.md). |
| `just install` | Installs the release binary into `~/.cargo/bin`. |

## Docker

| Recipe | What it does |
|---|---|
| `just build-docker [tag]` | Builds an image for this computer's architecture. |
| `just build-dockerx [tag]` | Builds `linux/amd64` and `linux/arm64` with Buildx and merges the manifest. |
| `just build-docker-x86 [tag]` | Builds a `linux/amd64` image. |
| `just build-docker-arm [tag]` | Builds a `linux/arm64` image. |
| `just push-docker <repo> [tag]` | Builds both architectures and pushes them to `<repo>`. Falls back to one architecture without Buildx. |

The tag defaults to `latest`.

## Docs

| Recipe | What it does |
|---|---|
| `just vale-lint` | Checks the docs, the help text, the recipe list, and the strings in the source with the Vale Google style. |
| `just vale-fmt` | Applies Vale's automatic fixes to the docs, then runs `vale-lint`. |
| `just site` | Builds the documentation site into `target/site` and checks its links. |
| `just site-serve` | Builds the site and serves it at `http://localhost:8000`. |
