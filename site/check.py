#!/usr/bin/env python3
"""Check a built documentation site before it's deployed.

Fails when an internal link or fragment points nowhere, when a page lacks a
title, a description or exactly one h1, or when an id repeats on a page.

    python3 site/check.py build/site
"""

from __future__ import annotations

import sys
import tomllib
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit


class PageScan(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.ids: list[str] = []
        self.links: list[str] = []
        self.h1 = 0
        self.title = ""
        self.description = False
        self._in_title = False

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        a = dict(attrs)
        if a.get("id"):
            self.ids.append(a["id"])
        if tag == "a" and a.get("href"):
            self.links.append(a["href"])
        if tag in ("link", "script", "img") and (a.get("href") or a.get("src")):
            if a.get("rel") != "canonical":
                self.links.append(a.get("href") or a.get("src"))
        if tag == "h1":
            self.h1 += 1
        if tag == "title":
            self._in_title = True
        if tag == "meta" and a.get("name") == "description" and a.get("content"):
            self.description = True

    def handle_endtag(self, tag: str) -> None:
        if tag == "title":
            self._in_title = False

    def handle_data(self, data: str) -> None:
        if self._in_title:
            self.title += data


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    root = Path(sys.argv[1]).resolve()
    config = tomllib.loads((Path(__file__).resolve().parent / "site.toml").read_text(encoding="utf-8"))
    base_path = urlsplit(config["site"]["base_url"]).path or "/"
    pages = {p.relative_to(root).as_posix(): p for p in root.rglob("*.html")}
    scans: dict[str, PageScan] = {}
    for name, path in pages.items():
        scan = PageScan()
        scan.feed(path.read_text(encoding="utf-8"))
        scans[name] = scan

    errors: list[str] = []
    for name, scan in scans.items():
        if not scan.title.strip():
            errors.append(f"{name}: no <title>")
        if not scan.description:
            errors.append(f"{name}: no meta description")
        if scan.h1 != 1:
            errors.append(f"{name}: {scan.h1} h1 elements, expected 1")
        dupes = {i for i in scan.ids if scan.ids.count(i) > 1}
        if dupes:
            errors.append(f"{name}: repeated ids {sorted(dupes)}")
        for link in scan.links:
            parts = urlsplit(link)
            if parts.scheme or link.startswith("//") or link.startswith("mailto:"):
                continue
            if parts.path.startswith("/"):
                if not parts.path.startswith(base_path):
                    errors.append(f"{name}: link outside the site {link}")
                    continue
                target = unquote(parts.path[len(base_path):]) or "index.html"
            else:
                target = name if not parts.path else (Path(name).parent / unquote(parts.path)).as_posix()
            target = Path(target).as_posix()
            if not (root / target).exists():
                errors.append(f"{name}: broken link {link}")
                continue
            if parts.fragment and target in scans and unquote(parts.fragment) not in scans[target].ids:
                errors.append(f"{name}: missing anchor {link}")

    for error in errors:
        print(error)
    print(f"checked {len(scans)} pages: {len(errors)} problems")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
