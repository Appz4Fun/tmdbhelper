#!/usr/bin/env python3
"""Writes schema/ItemDetails.sql from a TMDBHelper checkout's table definitions.

Usage: scripts/addon-schema.py <plugin.video.themoviedb.helper checkout>
"""
import importlib.util
import re
import sqlite3
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "schema" / "ItemDetails.sql"
LIB = "resources/tmdbhelper/lib/items/database"


def load_tabledef(addon):
    path = addon / LIB / "tabledef.py"
    spec = importlib.util.spec_from_file_location("tabledef", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def addon_tables(addon, tabledef):
    """Reads the table order and the version from the add-on's database.py."""
    source = (addon / LIB / "database.py").read_text()
    version = int(re.search(r"database_version = (\d+)", source).group(1))
    aliases = dict(re.findall(r"(\w+_columns) = (\w+_COLUMNS)", source))
    body = source[source.index("def database_tables") :]
    tables = re.findall(r"'(\w+)': self\.(\w+_columns)", body)
    return version, [(name, getattr(tabledef, aliases[attr])) for name, attr in tables]


def create(conn, name, columns):
    parts = [f'{k} {v["data"]}' for k, v in columns.items()]
    parts += [
        f'FOREIGN KEY({k}) REFERENCES {v["foreign_key"]} ON DELETE CASCADE'
        for k, v in columns.items()
        if "foreign_key" in v
    ]
    unique = [k for k, v in columns.items() if v.get("unique")]
    if unique:
        parts.append("UNIQUE ({})".format(", ".join(unique)))
    conn.execute(f'CREATE TABLE {name}({", ".join(parts)})')
    for key, value in columns.items():
        if value.get("indexed"):
            conn.execute(f"CREATE INDEX {name}_{key}_x ON {name}({key})")


def main():
    addon = Path(sys.argv[1])
    version, tables = addon_tables(addon, load_tabledef(addon))
    seeds = [l for l in OUT.read_text().splitlines() if l.startswith(("INSERT INTO languages", "INSERT INTO countries"))]
    conn = sqlite3.connect(":memory:")
    for name, columns in tables:
        create(conn, name, columns)
    for seed in seeds:
        conn.execute(seed)
    lines = list(conn.iterdump())
    OUT.write_text("PRAGMA foreign_keys=OFF;\n" + "\n".join(lines) + f"\nPRAGMA user_version={version};\n")
    print(f"schema version {version}, {len(tables)} tables, {len(seeds)} seed rows")


if __name__ == "__main__":
    main()
