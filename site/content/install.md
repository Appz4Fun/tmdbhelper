+++
title = "Install and build"
description = "Build warmup-rs from source with Cargo or Just, cross-compile static Linux binaries, or build a Docker image."
eyebrow = "Get started"
lede = "warmup-rs builds with stable Rust. The Just recipes wrap the usual Cargo, cross, and Docker commands."
+++

## Requirements

| Tool | Needed for | Notes |
|---|---|---|
| Rust and Cargo | Every build | Install with [rustup](https://rustup.rs). The build bundles SQLite, so you need no system library. |
| [Just](https://github.com/casey/just) | The recipes | Optional. Every recipe is a short Cargo, cross, or Docker command that you can run by hand. |
| [cross](https://github.com/cross-rs/cross) and Docker | Static Linux and Windows builds | cross runs each build in a container with the right linker. |
| Docker Buildx | Multi-arch images | Ships with Docker Desktop and OrbStack. |
| [Vale](https://vale.sh) and Python 3 | Prose checks | Only for contributors. See [Contributing](contributing.md). |

## Build for this computer

```bash
just build          # or: cargo build --release
```

The binary is `target/release/warmup-rs`. The release profile uses fat LTO, one codegen unit, `panic=abort`, and strips symbols. For a debug build, run `just build-debug`.

To put the binary on your `PATH`, run `just install`, which runs `cargo install --locked --path .` and copies it to `~/.cargo/bin`.

## Build static Linux binaries

```bash
just build-release-arm   # aarch64-unknown-linux-musl, for CoreELEC and other arm64 boxes
just build-release-x86   # x86_64-unknown-linux-musl
```

Each binary is static, about 5.5 MB, and has no runtime dependencies. The output is `target/<target>/release/warmup-rs`.

> [!NOTE] cross and Homebrew Rust
> cross needs rustup's toolchain. The recipes put `~/.cargo/bin` first on the `PATH`, so a Homebrew `cargo` earlier on the `PATH` doesn't confuse cross.

## Build for Windows

```bash
just build-release-win   # x86_64-pc-windows-gnu, beta
```

The Windows build is a beta. It cross-compiles with MinGW through cross, and it isn't tested on Windows yet. See `TODO.md` in the repository for its status.

## Build a Docker image

```bash
just build-docker        # this computer's architecture
just build-dockerx       # linux/amd64 and linux/arm64 in one manifest
```

See [Docker](docker.md) for running the image and pushing it to a registry.
