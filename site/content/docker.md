+++
title = "Docker"
description = "Run warmup-rs from a Docker image, build images for amd64 and arm64, and push a multi-arch manifest to a registry."
eyebrow = "Get started"
lede = "The image holds only the static binary. Runs write to /data/output in the container, so mount a folder there."
+++

## Run the image

```bash
docker run --rm --init -e WARMUP_TMDB_API_KEY=<your key> \
  -v "$PWD/output:/data/output" warmup-rs

docker run --rm --init -v "$PWD/output:/data/output" warmup-rs --mode=images
```

The container counts as a desktop system, so warmup-rs uses the `./output` defaults inside its `/data` working folder. Any flag after the image name goes to warmup-rs. The `--init` flag lets Ctrl+C and `docker stop` end the crawl right away.

## Build images

| Recipe | Result |
|---|---|
| `just build-docker` | `warmup-rs:latest` for this computer's architecture. |
| `just build-docker-x86` | `warmup-rs:latest-amd64` for `linux/amd64`. |
| `just build-docker-arm` | `warmup-rs:latest-arm64` for `linux/arm64`, such as Apple silicon Macs. |
| `just build-dockerx` | A multi-arch build for `linux/amd64` and `linux/arm64` with one merged manifest. |

Each recipe takes an optional tag, for example `just build-docker 0.1.0`. The build runs `cargo build --release` in an Alpine Rust image and copies the binary into an empty `scratch` image.

> [!NOTE] Loading a multi-arch build
> Docker's default image store can't load a multi-arch image, so `just build-dockerx` keeps the result in the Buildx cache. To get a multi-arch image somewhere you can use it, push it with `just push-docker`.

## Push to a registry

```bash
docker login ghcr.io
just push-docker ghcr.io/<owner>/warmup-rs          # tag latest
just push-docker ghcr.io/<owner>/warmup-rs 0.1.0    # another tag
```

`push-docker` builds both architectures with Buildx and pushes one manifest. If Buildx isn't available, it builds and pushes an image for this computer's architecture.
