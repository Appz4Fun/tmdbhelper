#!/usr/bin/env python3
"""Collect user-facing string literals from the Rust sources for Vale.

Writes one Markdown paragraph per log, error, or help string found in
`src/`, plus each comment, message, and unit description in `packaging/`.
Format placeholders such as `{}` become inline code, so Vale checks the prose
around them. Text that comes back from TMDB isn't in the source, so it never
reaches this file.
"""
import pathlib
import re
import sys

MACROS = r"(?:info|warn|error|debug|trace|bail|anyhow|format|context)"
CALL = re.compile(MACROS + r"!?\(\s*\"((?:[^\"\\]|\\.)*)\"", re.S)
CONTEXT = re.compile(r"\.(?:context|with_context)\(\s*(?:\|\|\s*format!\()?\s*\"((?:[^\"\\]|\\.)*)\"", re.S)
PACKAGING = re.compile(r"^\s*(?:#\s+(.+)|echo \"(.+)\"|Description=(.+))$")
PLACEHOLDER = re.compile(r"\{[^{}]*\}")
IDENT = re.compile(r"(?<![\w`])(--[\w-]+|[A-Z][A-Z0-9]*_[A-Z0-9_]+)\b")
KEYVALUE = re.compile(r"\b\w+=\S*?(?=[\s,]|$)")


def clean(s):
    s = s.replace("\\n", " ").replace('\\"', '"')
    s = KEYVALUE.sub(lambda m: "`" + PLACEHOLDER.sub("N", m.group(0)) + "`", s)
    s = PLACEHOLDER.sub("`value`", s)
    s = IDENT.sub(r"`\1`", s)
    return " ".join(s.split())


def packaging_strings(root):
    """Comments, `echo` messages, and unit descriptions in the opkg files."""
    found = []
    for path in sorted(pathlib.Path(root).rglob("*")):
        if not path.is_file():
            continue
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            m = PACKAGING.match(line)
            if not m or "=" in (m.group(1) or "") or "shellcheck" in line:
                continue
            text = next(g for g in m.groups() if g)
            text = re.sub(r"\$\w+|\S*/\S*|\bsystemctl .*|\b[A-Z]+_[A-Z_]+\b|\bwarmup-rs:", lambda x: f"`{x.group(0)}`", text)
            found.append(f"<!-- {path}:{number} -->\n{text}\n")
    return found


def main(src, out):
    found = packaging_strings("packaging") if pathlib.Path("packaging").is_dir() else []
    for path in sorted(pathlib.Path(src).rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for rx in (CALL, CONTEXT):
            for m in rx.finditer(text):
                line = text.count("\n", 0, m.start()) + 1
                s = clean(m.group(1))
                if " " not in s:
                    continue  # a URL, path, or other identifier, not prose
                if len(re.sub(r"`[^`]*`", "", s).strip(" :,.")) > 2:
                    found.append(f"<!-- {path}:{line} -->\n{s}\n")
    pathlib.Path(out).write_text("\n".join(sorted(set(found))), encoding="utf-8")
    print(f"Wrote {len(set(found))} strings to {out}.")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "src", sys.argv[2] if len(sys.argv) > 2 else "target/vale/strings.md")
