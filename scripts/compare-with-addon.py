#!/usr/bin/env python3
"""Compares an ItemDetails.db from warmup-rs with one the add-on wrote itself.

Usage: scripts/compare-with-addon.py <ours.db> <addon.db> <item id>...

For each item, it prints the rows only one database has, and the columns where
the add-on has a value and warmup-rs doesn't, or the two disagree.
"""
import sqlite3
import sys

# Tables that hang off an item through parent_id, with their UNIQUE columns.
CHILDREN = {
    "art": ("icon", "type"),
    "default_art": ("type",),
    "castmember": ("tmdb_id", "role"),
    "crewmember": ("tmdb_id", "role", "department"),
    "certification": ("iso_country", "iso_language", "release_date", "release_type"),
    "country": ("iso_country",),
    "custom": ("key",),
    "genre": ("tmdb_id",),
    "language": ("iso_language",),
    "network": ("tmdb_id",),
    "provider": ("tmdb_id", "iso_country"),
    "studio": ("tmdb_id",),
    "translation": ("iso_country", "iso_language"),
    "unique_id": ("key",),
    "video": ("path",),
}
OWN = ("baseitem", "movie", "tvshow", "season", "episode", "person", "collection")
# TMDB changes these between the two runs, so a difference means nothing.
VOLATILE = {"expiry", "popularity", "votes", "rating", "translation", "appearances", "ordering"}


def rows(conn, sql, *args):
    conn.row_factory = sqlite3.Row
    return [dict(r) for r in conn.execute(sql, args)]


def compare_rows(label, ours, theirs):
    problems = 0
    for key in sorted(set(ours) | set(theirs), key=str):
        if key not in ours:
            print(f"  {label}: missing {key}")
            problems += 1
        elif key not in theirs:
            print(f"  {label}: extra {key}")
            problems += 1
        else:
            for column, value in theirs[key].items():
                if column in VOLATILE or column == "parent_id":
                    continue
                if ours[key].get(column) != value:
                    print(f"  {label} {key} {column}: ours={ours[key].get(column)!r} addon={value!r}")
                    problems += 1
    return problems


def main():
    ours, addon = sqlite3.connect(sys.argv[1]), sqlite3.connect(sys.argv[2])
    total = 0
    for item in sys.argv[3:]:
        print(item)
        for table in OWN:
            mine = rows(ours, f"SELECT * FROM {table} WHERE id=?", item)
            theirs = rows(addon, f"SELECT * FROM {table} WHERE id=?", item)
            if mine or theirs:
                total += compare_rows(table, {item: r for r in mine}, {item: r for r in theirs})
        for table, key in CHILDREN.items():
            make = lambda data: {tuple(r[k] for k in key): r for r in data}
            mine = make(rows(ours, f"SELECT * FROM {table} WHERE parent_id=?", item))
            theirs = make(rows(addon, f"SELECT * FROM {table} WHERE parent_id=?", item))
            total += compare_rows(table, mine, theirs)
    print(f"{total} differences")


if __name__ == "__main__":
    main()
