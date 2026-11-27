#!/usr/bin/env bash
set -euo pipefail
python3 - <<'PY'
import pathlib, re, sys

bad = []
for p in sorted(pathlib.Path("crates").rglob("*.rs")):
    text = p.read_text(encoding="utf-8")
    in_str = False
    in_raw = None
    in_char = False
    esc = False
    i = 0
    line = 1
    n = len(text)
    while i < n:
        c = text[i]
        if c == '\n':
            line += 1
            in_char = False
            esc = False
            i += 1
            continue
        if in_raw is not None:
            hashes = '#' * in_raw
            if c == '"' and text[i+1:i+1+in_raw] == hashes:
                i += 1 + in_raw
                in_raw = None
                continue
            i += 1
            continue
        if in_str:
            if esc:
                esc = False
            elif c == '\\':
                esc = True
            elif c == '"':
                in_str = False
            i += 1
            continue
        if in_char:
            if esc:
                esc = False
            elif c == '\\':
                esc = True
            elif c == '\'':
                in_char = False
            i += 1
            continue
        if c == 'r' and i + 1 < n and text[i+1] in '#"':
            j = i + 1
            while j < n and text[j] == '#':
                j += 1
            if j < n and text[j] == '"':
                in_raw = j - (i + 1)
                i = j + 1
                continue
        if c == 'b' and i + 2 < n and text[i+1] == 'r' and text[i+2] in '#"':
            j = i + 2
            while j < n and text[j] == '#':
                j += 1
            if j < n and text[j] == '"':
                in_raw = j - (i + 2)
                i = j + 1
                continue
        if c == '"':
            in_str = True
            i += 1
            continue
        if c == '\'':
            if i + 2 < n and (text[i+1] == '\\' or text[i+2] == '\''):
                in_char = True
                i += 1
                continue
        if c == '/' and i + 1 < n and text[i+1] in '/*':
            bad.append(f"{p}:{line}")
            break
        i += 1

if bad:
    print("Comments found in Rust files:")
    for b in bad:
        print(" ", b)
    sys.exit(1)
print("Comment check passed: 0 comments in crates/**/*.rs")
PY
