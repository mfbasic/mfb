#!/usr/bin/env python3
"""LOC census for plan-176: the independent oracle for tests/guards/file_size_budget.rs.

Counts non-comment, non-blank lines of .rs and .mfb files under src/, packages/,
examples/, repository/ and tests/, split into code and test lines. Stdlib only.
See README.md for the counting rules.

  census.py [--root DIR] [--json OUT]       per-file JSON (path, code, test, gen, testfile)
  census.py [--root DIR] --seed             ceiling rows for every over-limit (file, kind)
  census.py [--root DIR] --file PATH        "code test" for one file
  census.py ... --file PATH --lines A B     "code test" for lines A..B (1-based, inclusive)
"""
import argparse
import json
import os
import re
import sys

DIRS = ["src", "packages", "examples", "repository", "tests"]
SKIP = {"target", ".git"}
LIMITS = {"code": 1000, "inline": 250, "test": 1000}
GEN = re.compile(
    r"^\s*(//!?|/\*|'|REM)\s.*\bGENERATED\b|^\s*(//!?|/\*|'|REM)\s*GENERATED\b"
)
CHAR_LIT = re.compile(r"'(\\(x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)|[^\\'\n])'")
RAW_START = re.compile(r'(b|c)?r(#*)"')
CFG = re.compile(r"#\[cfg\((all\()?test\b")
CFGMOD = re.compile(
    r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+([a-z_0-9]+)\s*;"
)
PATHMOD = re.compile(
    r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*#\[path\s*=\s*\"([^\"]+)\"\]\s*"
    r"(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+[a-z_0-9]+\s*;"
)
FILE_CFG_TEST = re.compile(r"^#!\[cfg\(test\)\]", re.M)

# Per-line kinds.
NONE, CODE, TEST = 0, 1, 2


def ident_char(c):
    return c.isalnum() or c == "_"


def rust_code_lines(text):
    """Each line's code text, with comments removed and literals blanked to 'S'."""
    out = [[] for _ in text.split("\n")]
    i, n, line = 0, len(text), 0

    def put(c):
        out[line].append(c)

    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
            continue
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        if text.startswith("/*", i):
            d = 1
            i += 2
            while i < n and d:
                if text.startswith("/*", i):
                    d += 1
                    i += 2
                elif text.startswith("*/", i):
                    d -= 1
                    i += 2
                else:
                    if text[i] == "\n":
                        line += 1
                    i += 1
            continue
        m = RAW_START.match(text[i : i + 300])
        if m and (i == 0 or not ident_char(text[i - 1])):
            put("S")
            i += m.end()
            end = '"' + m.group(2)
            while i < n and not text.startswith(end, i):
                if text[i] == "\n":
                    line += 1
                    put("S")
                i += 1
            i += len(end)
            continue
        if c == '"' or (
            c in "bc"
            and text.startswith('"', i + 1)
            and (i == 0 or not ident_char(text[i - 1]))
        ):
            if c != '"':
                i += 1
            put("S")
            i += 1
            while i < n and text[i] != '"':
                if text[i] == "\\":
                    i += 1
                if i < n and text[i] == "\n":
                    line += 1
                    put("S")
                i += 1
            i += 1
            continue
        if c == "'":
            m = CHAR_LIT.match(text[i : i + 16])
            if m:
                put("S")
                i += m.end()
                continue
        put(c)
        i += 1
    return ["".join(x) for x in out]


def rust_kinds(text):
    """Per-line NONE/CODE/TEST for a Rust file (TEST = inside a #[cfg(test)] item)."""
    kinds = []
    state = None  # [start_depth, seen_body]
    depth = 0
    for l in rust_code_lines(text):
        s = l.strip()
        if not s:
            kinds.append(NONE)
            continue
        line_is_test = state is not None
        at_start = True  # the scan starts at the beginning of the line
        s_scan = s
        if state is None:
            m = CFG.search(s)
            if m:
                state = [depth, False]
                line_is_test = True
                if m.start() > 0:
                    at_start = False
                    for ch in s[: m.start()]:
                        if ch in "([{":
                            depth += 1
                        elif ch in ")]}":
                            depth -= 1
                    state[0] = depth
                    s_scan = s[m.start() :]
        for i, ch in enumerate(s_scan):
            if ch in "([{":
                depth += 1
            elif ch in ")]}":
                depth -= 1
                if state is not None and ch == "}" and depth == state[0] and state[1]:
                    state = None
            elif ch in ";," and state is not None and depth == state[0]:
                state = None
            if state is not None and depth < state[0]:
                state = None
                if i == 0 and at_start:
                    line_is_test = False
            if state is not None and ch == "{" and depth == state[0] + 1:
                state[1] = True
        kinds.append(TEST if line_is_test else CODE)
    return kinds


def mfb_kinds(text):
    """Per-line NONE/CODE/TEST for an MFBASIC file (TEST = inside TESTING ... END TESTING)."""
    kinds = []
    in_doc = in_test = False
    for raw in text.split("\n"):
        s = raw.strip()
        if in_doc:
            if re.match(r"END DOC\b", s):
                in_doc = False
            kinds.append(NONE)
            continue
        if re.match(r"DOC(\s+[A-Z]+)*\s*$", s):
            in_doc = True
            kinds.append(NONE)
            continue
        if not s or s.startswith("'") or re.match(r"REM(\s|$)", s):
            kinds.append(NONE)
            continue
        if re.match(r"TESTING\s*$", s):
            in_test = True
        kinds.append(TEST if in_test else CODE)
        if re.match(r"END TESTING\b", s):
            in_test = False
    return kinds


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def walk(root):
    """Every in-scope .rs/.mfb file as (absolute path, '/'-separated relative path), sorted."""
    found = []
    for d in DIRS:
        for dp, dns, fns in os.walk(os.path.join(root, d)):
            dns[:] = [x for x in dns if x not in SKIP]
            for f in fns:
                if f.endswith(".rs") or f.endswith(".mfb"):
                    p = os.path.join(dp, f)
                    found.append((p, os.path.relpath(p, root).replace(os.sep, "/")))
    found.sort(key=lambda x: x[1])
    return found


def test_roots(root, files):
    """Paths (files and directories) that are test-only because a #[cfg(test)] mod declares them."""
    roots = set()
    for p, _ in files:
        if not p.endswith(".rs"):
            continue
        t = read(p)
        dp, f = os.path.split(p)
        base = dp if f in ("mod.rs", "main.rs", "lib.rs") else os.path.join(dp, f[:-3])
        for m in CFGMOD.finditer(t):
            for cand in (os.path.join(base, m.group(1) + ".rs"), os.path.join(base, m.group(1))):
                if os.path.exists(cand):
                    roots.add(os.path.relpath(cand, root).replace(os.sep, "/"))
        for m in PATHMOD.finditer(t):
            cand = os.path.normpath(os.path.join(dp, m.group(1)))
            if os.path.exists(cand):
                roots.add(os.path.relpath(cand, root).replace(os.sep, "/"))
    for r in list(roots):
        if r.endswith("/mod.rs"):
            roots.add(r[:-7])
        elif r.endswith(".rs"):
            roots.add(r[:-3])
    return roots


def is_test_file(rel, text, roots):
    parts = rel.split("/")
    if "tests" in parts[:-1]:
        return True
    name = parts[-1]
    if name.endswith(".mfb") and name.startswith("test_"):
        return True
    if rel in roots or any(rel.startswith(r + "/") for r in roots):
        return True
    return name.endswith(".rs") and FILE_CFG_TEST.search(text) is not None


def is_generated(text):
    return any(GEN.match(h) for h in text.split("\n")[:3])


def line_kinds(rel, text, roots):
    """Per-line kinds with the whole-file test rule applied."""
    kinds = rust_kinds(text) if rel.endswith(".rs") else mfb_kinds(text)
    if is_test_file(rel, text, roots):
        kinds = [TEST if k else NONE for k in kinds]
    return kinds


def census(root):
    files = walk(root)
    roots = test_roots(root, files)
    res = []
    for p, rel in files:
        t = read(p)
        kinds = line_kinds(rel, t, roots)
        res.append(
            dict(
                path=rel,
                code=kinds.count(CODE),
                test=kinds.count(TEST),
                gen=is_generated(t),
                testfile=is_test_file(rel, t, roots),
            )
        )
    return res


def seed_rows(res):
    rows = []
    for r in res:
        if r["gen"]:
            continue
        if r["testfile"]:
            counts = {"test": r["test"]}
        else:
            counts = {"code": r["code"], "inline": r["test"]}
        for kind, n in counts.items():
            if n > LIMITS[kind]:
                rows.append((r["path"], kind, n))
    rows.sort()
    return ["ceiling %s %d %s" % (kind, n, path) for path, kind, n in rows]


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=os.path.normpath(os.path.join(here, "..", "..")))
    ap.add_argument("--json", metavar="OUT")
    ap.add_argument("--seed", action="store_true")
    ap.add_argument("--file", metavar="PATH")
    ap.add_argument("--lines", nargs=2, type=int, metavar=("A", "B"))
    a = ap.parse_args()
    root = os.path.abspath(a.root)
    if a.lines and not a.file:
        ap.error("--lines needs --file")
    if a.file:
        p = a.file if os.path.isabs(a.file) else os.path.join(root, a.file)
        rel = os.path.relpath(p, root).replace(os.sep, "/")
        kinds = line_kinds(rel, read(p), test_roots(root, walk(root)))
        if a.lines:
            lo, hi = a.lines
            kinds = kinds[max(lo, 1) - 1 : hi]
        print(kinds.count(CODE), kinds.count(TEST))
        return
    res = census(root)
    if a.json:
        with open(a.json, "w") as f:
            json.dump(res, f, indent=0)
    if a.seed:
        sys.stdout.write("".join(row + "\n" for row in seed_rows(res)))
    elif not a.json:
        json.dump(res, sys.stdout, indent=0)
        sys.stdout.write("\n")


if __name__ == "__main__":
    main()
