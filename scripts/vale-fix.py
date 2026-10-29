#!/usr/bin/env python3
"""Apply Vale's automatic fixes to files in place.

Runs `vale --output=JSON` on the given paths and applies every alert that
carries a fix action (`replace`, `remove`, or `edit`). Alerts without a fix
stay for a person to resolve, so run `just vale-lint` afterward.
"""
import json
import subprocess
import sys
from collections import defaultdict


def fix_for(alert, text):
    action = alert.get("Action") or {}
    name, params = action.get("Name"), action.get("Params") or []
    if name == "replace" and params:
        return params[0]
    if name == "remove":
        return ""
    if name == "edit" and params:
        op = params[0]
        if op == "trim" and len(params) > 1:
            return text.strip(params[1])
        if op == "trim_right" and len(params) > 1:
            return text.rstrip(params[1])
        if op == "trim_left" and len(params) > 1:
            return text.lstrip(params[1])
        if op == "truncate" and len(params) > 1:
            return text.split(params[1])[0]
        if op == "regex" and len(params) > 2:
            import re
            return re.sub(params[1], params[2], text)
    return None


def main(paths):
    result = subprocess.run(
        ["vale", "--output=JSON", "--no-exit", *paths],
        capture_output=True, text=True, check=False,
    )
    try:
        report = json.loads(result.stdout or "{}")
    except json.JSONDecodeError:
        sys.stderr.write(result.stdout + result.stderr)
        return 1
    fixed = 0
    for path, alerts in report.items():
        with open(path, encoding="utf-8") as f:
            lines = f.read().split("\n")
        edits = defaultdict(list)
        for a in alerts:
            line = a["Line"] - 1
            start, end = a["Span"][0] - 1, a["Span"][1]
            original = lines[line][start:end]
            replacement = fix_for(a, original)
            if replacement is not None and replacement != original:
                edits[line].append((start, end, replacement, a["Check"]))
        for line, items in edits.items():
            last_start = None
            for start, end, replacement, check in sorted(items, reverse=True):
                if last_start is not None and end > last_start:
                    continue
                text = lines[line]
                lines[line] = text[:start] + replacement + text[end:]
                last_start = start
                fixed += 1
                print(f"{path}:{line + 1}: {check}: {text[start:end]!r} -> {replacement!r}")
        if edits:
            with open(path, "w", encoding="utf-8") as f:
                f.write("\n".join(lines))
    print(f"Applied {fixed} fixes.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:] or ["."]))
