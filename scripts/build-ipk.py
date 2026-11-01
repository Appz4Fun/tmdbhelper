#!/usr/bin/env python3
"""Build an Entware opkg package and a package feed for warmup-rs.

Writes `<out>/<feed>/warmup-rs_<version>_<arch>.ipk`, plus the `Packages` and
`Packages.gz` indexes that opkg reads. The package is reproducible: every file
belongs to root and has a fixed timestamp.

    python3 scripts/build-ipk.py --binary target/aarch64-unknown-linux-musl/release/warmup-rs
"""
import argparse
import gzip
import hashlib
import io
import pathlib
import tarfile
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent
SOURCE = ROOT / "packaging" / "opkg"
# Entware names its feed folder aarch64-k3.10 and its package architecture aarch64-3.10.
FEED = "aarch64-k3.10"
ARCH = "aarch64-3.10"
RELEASE = "1"
ENV_FILE = "opt/etc/warmup-rs.env"
UNITS = ("tmdbhelper-warmup-rs.service", "tmdbhelper-warmup-images.service")
# Init scripts for Entware systems without systemd. They're conffiles, so an
# upgrade keeps the ENABLED setting.
INIT_SCRIPTS = ("S90warmup-rs", "S91warmup-images")


def gzipped(data):
    """Gzip without a timestamp or filename, so the output is reproducible."""
    buffer = io.BytesIO()
    with gzip.GzipFile(filename="", fileobj=buffer, mode="wb", mtime=0) as f:
        f.write(data)
    return buffer.getvalue()


def tar_gz(entries):
    """Pack (path, mode, bytes or None for a folder) entries into a .tar.gz."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w", format=tarfile.GNU_FORMAT) as tar:
        for path, mode, data in entries:
            info = tarfile.TarInfo(f"./{path}" if path else ".")
            info.mode = mode
            if data is None:
                info.type = tarfile.DIRTYPE
                tar.addfile(info)
            else:
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))
    return gzipped(buffer.getvalue())


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", type=pathlib.Path, required=True, help="the static aarch64 binary")
    parser.add_argument("--out", type=pathlib.Path, default=ROOT / "target" / "opkg", help="output folder")
    args = parser.parse_args()

    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]
    version = f"{cargo['version']}-{RELEASE}"
    binary = args.binary.read_bytes()
    if binary[:4] != b"\x7fELF" or int.from_bytes(binary[18:20], "little") != 183:
        raise SystemExit(f"{args.binary} isn't an aarch64 Linux binary")

    unit_files = [(f"opt/share/warmup-rs/systemd/{u}", 0o644, (SOURCE / u).read_bytes()) for u in UNITS]
    init_files = [(f"opt/etc/init.d/{n}", 0o755, (SOURCE / "init.d" / n).read_bytes()) for n in INIT_SCRIPTS]
    data_entries = [
        ("opt", 0o755, None),
        ("opt/bin", 0o755, None),
        ("opt/bin/warmup-rs", 0o755, binary),
        ("opt/etc", 0o755, None),
        (ENV_FILE, 0o600, (SOURCE / "warmup-rs.env").read_bytes()),
        ("opt/etc/init.d", 0o755, None),
        *init_files,
        ("opt/share", 0o755, None),
        ("opt/share/warmup-rs", 0o755, None),
        ("opt/share/warmup-rs/warmup.sh", 0o644, (SOURCE / "init.d" / "warmup.sh").read_bytes()),
        ("opt/share/warmup-rs/systemd", 0o755, None),
        *unit_files,
    ]
    installed_size = sum(len(d) for _, _, d in data_entries if d)

    fields = (
        f"Package: warmup-rs\nVersion: {version}\nSection: multimedia\n"
        f"Architecture: {ARCH}\nLicense: GPL-3.0-or-later\n"
        f"URL: https://appz4fun.github.io/tmdbhelper/\n"
    )
    description = "Description:  Fills the TMDBHelper metadata cache and the Kodi image cache by writing directly to SQLite.\n"
    control = (
        fields
        + f"Installed-Size: {installed_size}\n"
        + "Maintainer: xbmc4lyfe <273732874+xbmc4lyfe@users.noreply.github.com>\n"
        + description
    )
    control_tar = tar_gz([
        ("", 0o755, None),
        ("control", 0o644, control.encode()),
        ("conffiles", 0o644, "".join(f"/{f}\n" for f in [ENV_FILE, *(path for path, _, _ in init_files)]).encode()),
        ("postinst", 0o755, (SOURCE / "postinst").read_bytes()),
        ("prerm", 0o755, (SOURCE / "prerm").read_bytes()),
        ("postrm", 0o755, (SOURCE / "postrm").read_bytes()),
    ])
    data_tar = tar_gz([("", 0o755, None), *data_entries])
    ipk = tar_gz([
        ("debian-binary", 0o644, b"2.0\n"),
        ("data.tar.gz", 0o644, data_tar),
        ("control.tar.gz", 0o644, control_tar),
    ])

    feed = args.out / FEED
    feed.mkdir(parents=True, exist_ok=True)
    name = f"warmup-rs_{version}_{ARCH}.ipk"
    (feed / name).write_bytes(ipk)
    index = (
        fields
        + f"Installed-Size: {installed_size}\nFilename: {name}\nSize: {len(ipk)}\n"
        + f"SHA256sum: {hashlib.sha256(ipk).hexdigest()}\n"
        + description
        + "\n"
    ).encode()
    (feed / "Packages").write_bytes(index)
    (feed / "Packages.gz").write_bytes(gzipped(index))
    print(f"Built {feed / name}")


if __name__ == "__main__":
    main()
