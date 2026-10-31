+++
title = "Entware package"
description = "Install warmup-rs on CoreELEC from its Entware opkg feed, then run the metadata and image crawls as systemd services."
eyebrow = "Get started"
lede = "The opkg package installs warmup-rs and its systemd units on a CoreELEC box with Entware. Upgrades keep your settings and your API key."
+++

## Before you start

- **An arm64 box.** The feed has one package, for Entware's `aarch64-k3.10` architecture. Run `opkg print-architecture` to check yours.
- **Entware.** On CoreELEC, run `installentware` and reboot.
- **TMDBHelper**, installed and run once, so `ItemDetails.db` exists.
- **A USB SSD**, mounted at `/var/media/CACHE_DRIVE`. To use other locations, set the paths in the settings file.

## Install

The feed uses HTTPS, so opkg needs `wget-ssl` and `ca-bundle` first:

```bash
opkg update
opkg install wget-ssl ca-bundle
echo 'src/gz warmup-rs https://appz4fun.github.io/tmdbhelper/opkg/aarch64-k3.10' >> /opt/etc/opkg.conf
opkg update
opkg install warmup-rs
```

The package installs:

| Path | What it is |
|---|---|
| `/opt/bin/warmup-rs` | The static binary. |
| `/opt/etc/warmup-rs.env` | The settings file, readable only by root. |
| `/storage/.config/system.d/tmdbhelper-warmup-rs.service` | The metadata crawl. |
| `/storage/.config/system.d/tmdbhelper-warmup-images.service` | The image crawl. |

## Start the crawls

A new install starts nothing, because the services need your [TMDB API key](quick-start.md#get-a-tmdb-api-key) first.

1. Open `/opt/etc/warmup-rs.env`, remove the `#` from the `WARMUP_TMDB_API_KEY` line, and add your key.
2. Create the folder for the crawl queue:

    ```bash
    mkdir -p /var/media/CACHE_DRIVE/tmdb/scriptcache
    ```

3. Start the services, and keep them running after a reboot:

    ```bash
    systemctl enable --now tmdbhelper-warmup-rs tmdbhelper-warmup-images
    ```

See [Operate](operate.md) for logs and progress.

## Change the settings

Each line in `/opt/etc/warmup-rs.env` sets one environment variable from [Command-line options](cli.md). After you change the file, restart the services:

```bash
systemctl restart tmdbhelper-warmup-rs tmdbhelper-warmup-images
```

## Upgrade

```bash
opkg update
opkg upgrade warmup-rs
```

An upgrade restarts any service that's running, and enabled services stay enabled.

> [!NOTE] The message about the settings file
> If you changed the settings file, opkg prints a message under `Collected errors` that the file differs from the new package's copy. That's expected. opkg keeps your file and writes the new copy to `/opt/etc/warmup-rs.env-opkg`.

## Remove

```bash
opkg remove warmup-rs
```

This stops and disables the services and removes the units and the binary. It keeps your edited settings file and all the caches. Delete `/opt/etc/warmup-rs.env` by hand if you don't plan to reinstall, because it holds your API key.

## Build the package yourself

`just build-opkg` writes the package and a feed to `target/opkg/aarch64-k3.10`. To install that package without a feed, copy the `.ipk` file to the box and run `opkg install` with its path.
