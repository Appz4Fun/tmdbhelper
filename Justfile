# Recipes for building, testing, and packaging warmup-rs.
# Run `just` or `just --list` to see them all.

bin := "warmup-rs"
image := "warmup-rs"
target_x86 := "x86_64-unknown-linux-musl"
target_arm := "aarch64-unknown-linux-musl"
target_win := "x86_64-pc-windows-gnu"
prose := "README.md TODO.md site/content"
# cross needs the rustup toolchain, so ~/.cargo/bin goes ahead of a system Rust, such as Homebrew Rust.
cross := 'env PATH="$HOME/.cargo/bin:$PATH" cross'

# Lists the recipes.
default:
    @just --list

# Runs the test suite.
test *args:
    cargo test {{args}}

# Formats the Rust code with rustfmt.
fmt:
    cargo fmt --all

# Checks formatting and runs Clippy with warnings as errors.
lint:
    cargo fmt --all -- --check
    cargo clippy --all-targets -- -D warnings

# Runs warmup-rs with its defaults. Pass extra flags after the recipe name.
run *args:
    cargo run --release -- {{args}}

# Prints the warmup-rs help.
help:
    @cargo run --quiet -- --help

# Builds the release binary for this computer.
build: build-release

# Builds the release binary for this computer.
build-release:
    cargo build --release
    @echo "Built target/release/{{bin}}"

# Builds the debug binary for this computer.
build-debug:
    cargo build
    @echo "Built target/debug/{{bin}}"

# Builds a static musl release binary for x86_64 Linux. Needs cross and Docker.
build-release-x86:
    {{cross}} build --release --target {{target_x86}}
    @echo "Built target/{{target_x86}}/release/{{bin}}"

# Builds a static musl release binary for arm64 Linux. Needs cross and Docker.
build-release-arm:
    {{cross}} build --release --target {{target_arm}}
    @echo "Built target/{{target_arm}}/release/{{bin}}"

# Builds a beta x86_64 Windows release binary. Needs cross and Docker.
build-release-win:
    {{cross}} build --release --target {{target_win}}
    @echo "Built target/{{target_win}}/release/{{bin}}.exe"

# Installs the release binary for this computer into ~/.cargo/bin.
install:
    cargo install --locked --path .

# Builds a Docker image for this computer's architecture.
build-docker tag="latest":
    docker build -t {{image}}:{{tag}} .

# Builds an amd64 and arm64 image with Buildx and merges the manifest.
build-dockerx tag="latest":
    docker buildx build --platform linux/amd64,linux/arm64 -t {{image}}:{{tag}} .

# Builds a Docker image for linux/amd64.
build-docker-x86 tag="latest":
    docker buildx build --platform linux/amd64 --load -t {{image}}:{{tag}}-amd64 .

# Builds a Docker image for linux/arm64, such as Apple silicon Macs.
build-docker-arm tag="latest":
    docker buildx build --platform linux/arm64 --load -t {{image}}:{{tag}}-arm64 .

# Pushes a multi-arch image to a repository, for example `just push-docker ghcr.io/appz4fun/warmup-rs`.
push-docker repo tag="latest":
    #!/usr/bin/env sh
    set -eu
    if docker buildx inspect >/dev/null 2>&1; then
        docker buildx build --platform linux/amd64,linux/arm64 -t "{{repo}}:{{tag}}" --push .
    else
        echo "Buildx isn't available, so this pushes a single-arch image."
        docker build -t "{{repo}}:{{tag}}" .
        docker push "{{repo}}:{{tag}}"
    fi

# Lints the docs, help text, recipe list, and source strings with the Vale Google style.
vale-lint: _vale-generated
    vale --minAlertLevel=warning {{prose}} target/vale/help.md target/vale/just.md target/vale/strings.md

# Applies Vale's automatic fixes to the docs, then lints them.
vale-fmt:
    python3 scripts/vale-fix.py {{prose}}
    just vale-lint

# Builds the documentation site into target/site and checks its links.
site: _site-venv
    cargo run --quiet -- --help > site/data/help.txt
    target/site-venv/bin/python site/build.py --out target/site
    target/site-venv/bin/python site/check.py target/site

# Builds the documentation site and serves it at http://localhost:8000.
site-serve: site
    python3 -m http.server 8000 --directory target/site

[private]
_site-venv:
    @test -x target/site-venv/bin/python || (python3 -m venv target/site-venv && target/site-venv/bin/pip install --quiet --require-hashes -r site/requirements.lock)

[private]
_vale-generated:
    @mkdir -p target/vale
    @cargo run --quiet -- --help | python3 scripts/help-to-md.py > target/vale/help.md
    @just --list | python3 scripts/help-to-md.py --just > target/vale/just.md
    @python3 scripts/extract-strings.py src target/vale/strings.md >/dev/null
