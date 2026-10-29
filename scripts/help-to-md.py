#!/usr/bin/env python3
"""Turn `--help` or `just --list` output on stdin into Markdown for Vale.

Flags, paths, environment variables, defaults, and command lines become
inline code, so Vale checks only the prose in the help text.
"""
import re
import sys

CODE_LINE = re.compile(r"^\s*(Usage:|-\w|--\w|warmup-rs\b|WARMUP_\w+=|just\b)")
TOKENS = re.compile(
    r"\[(?:env|default|aliases?): [^\]]*\]"  # clap annotations
    r"|<[\w-]+>"                              # <VALUE> placeholders
    r"|(?<![\w`])--[\w-]+(?:=\S+)?"           # flags
    r"|\b[A-Z][A-Z0-9]*_[A-Z0-9_]+\b"         # environment variables
    r"|\S*/\S*"                               # paths and URLs
    r"|\b[\w-]+\.(?:db|md|txt|rs|toml)\b"     # file names
)
BULLET = re.compile(r"^(\s*- )([\w-]+):")
RECIPE = re.compile(r"^(\s*)([\w-]+(?: [^#]*)?)(#.*)?$")


def convert(line, just):
    if not line.strip():
        return ""
    if just:
        if line.startswith("Available recipes"):
            return "| Recipe | Description |\n|---|---|"
        m = RECIPE.match(line)
        name, comment = m.group(2).strip(), (m.group(3) or "").lstrip("# ")
        return f"| `{name}` | {TOKENS.sub(lambda t: f'`{t.group(0)}`', comment)} |"
    if CODE_LINE.match(line):
        return f"`{line.strip()}`"
    line = BULLET.sub(lambda m: f"{m.group(1)}`{m.group(2)}`:", line)
    return TOKENS.sub(lambda m: f"`{m.group(0)}`", line.strip())


def main():
    just = "--just" in sys.argv
    out = [convert(line.rstrip("\n"), just) for line in sys.stdin]
    sep = "\n" if just else "\n\n"
    sys.stdout.write(sep.join(l for l in out if l) + "\n")


if __name__ == "__main__":
    main()
