#!/usr/bin/env python3
"""Repoint stale ``[[src/path.rs:symbol]]`` source citations in src/spec/**.

The embedded spec (``mfb spec``) annotates prose with maintainer breadcrumbs of
the form ``[[src/some/file.rs:symbol]]`` pointing at the code that implements the
described behavior. ``strip_citations`` removes them at render time, so they are
cosmetic — but when a source file is split or a symbol moves, the path goes
stale. This script fixes those paths after a refactor.

Algorithm (conservative — only touches citations that are actually broken):

  For each unique ``[[oldpath:symbol]]`` cited in src/spec/**:
    * If ``oldpath`` still exists AND still defines ``symbol`` -> leave it.
    * Otherwise resolve ``symbol`` to the file(s) under src/ that define it
      (struct/enum/trait/union/fn/const/static/type/macro_rules):
        - exactly one definition file        -> repoint to it
        - several, but exactly one under the directory that replaced
          ``oldpath`` (``src/ir.rs`` -> ``src/ir/``) or its parent subtree
                                              -> repoint to that one
        - check MANUAL_OVERRIDES for an explicit answer
        - else                                -> report as unresolved

Citations whose anchor is a bare line number (``[[src/foo.rs:123]]``) cannot be
resolved by symbol; they are reported as unresolved when their file has moved,
and left untouched when their file still exists.

Usage:
    python3 scripts/fix_citations.py            # dry run: print what would change
    python3 scripts/fix_citations.py --apply    # rewrite the spec files in place
"""

import os
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).parent.parent
SRC_DIR = REPO_ROOT / "src"
SPEC_DIR = SRC_DIR / "spec"

# Explicit answers for citations the resolver cannot pin down on its own
# (ambiguous symbols, enum variants/fields the def-regex misses, or bare
# line-number anchors you want to convert to a symbol). Keyed by the cited
# ``(oldpath, anchor)`` exactly as it appears between the ``[[`` ``]]``; the
# value is the new ``path`` (anchor is preserved) or a full ``path:anchor``
# replacement. Empty by default — add entries as needed for a given migration.
#
# Examples (from the plan-11 large-file split; already applied):
#   ("src/rules.rs", "FMT_CHECK_FAILED"): "src/rules/table.rs",
#   ("src/ir.rs", "3015"): "src/ir/lower.rs:internalize",
MANUAL_OVERRIDES: dict[tuple[str, str], str] = {}

CITE_RX = re.compile(r"\[\[(src/[^\]:]+):([^\]]+)\]\]")


def def_regexes(sym: str) -> list[re.Pattern]:
    s = re.escape(sym)
    return [
        re.compile(r"\b(?:struct|enum|trait|union)\s+" + s + r"\b"),
        re.compile(r"\bfn\s+" + s + r"\b"),
        re.compile(r"\b(?:const|static)\s+" + s + r"\b"),
        re.compile(r"\btype\s+" + s + r"\b"),
        re.compile(r"\bmacro_rules!\s+" + s + r"\b"),
    ]


def load_src_files() -> dict[str, str]:
    files = {}
    for root, _, names in os.walk(SRC_DIR):
        for n in names:
            if n.endswith(".rs"):
                p = os.path.join(root, n)
                rel = os.path.relpath(p, REPO_ROOT)
                try:
                    files[rel] = open(p, encoding="utf-8").read()
                except OSError:
                    pass
    return files


def main() -> int:
    apply = "--apply" in sys.argv[1:]
    src_files = load_src_files()

    def defines(text: str, sym: str) -> bool:
        return any(rx.search(text) for rx in def_regexes(sym))

    def find_def_files(sym: str) -> list[str]:
        return [p for p, t in src_files.items() if defines(t, sym)]

    # Gather unique (oldpath, anchor) citations across the spec tree.
    citations: set[tuple[str, str]] = set()
    spec_files: list[str] = []
    for root, _, names in os.walk(SPEC_DIR):
        for n in names:
            p = os.path.join(root, n)
            spec_files.append(p)
            try:
                txt = open(p, encoding="utf-8").read()
            except OSError:
                continue
            for m in CITE_RX.finditer(txt):
                citations.add((m.group(1), m.group(2)))

    replacements: dict[tuple[str, str], str] = {}  # (oldpath, anchor) -> "path" or "path:anchor"
    unresolved: list[tuple[str, str, str]] = []
    valid = 0

    for oldpath, anchor in sorted(citations):
        if not oldpath.endswith(".rs"):
            continue  # non-Rust citations (.mfb etc.) are out of scope
        if (oldpath, anchor) in MANUAL_OVERRIDES:
            replacements[(oldpath, anchor)] = MANUAL_OVERRIDES[(oldpath, anchor)]
            continue
        old_text = src_files.get(oldpath)
        if old_text is not None and defines(old_text, anchor):
            valid += 1
            continue  # still correct
        candidates = find_def_files(anchor)
        if not candidates:
            unresolved.append((oldpath, anchor, "no-definition-found"))
            continue
        if len(candidates) == 1:
            replacements[(oldpath, anchor)] = candidates[0]
            continue
        stem_dir = oldpath[:-3] + "/"               # src/ir.rs -> src/ir/
        parent_dir = os.path.dirname(oldpath) + "/"  # for fan-out splits
        pref = [c for c in candidates if c.startswith(stem_dir)]
        if len(pref) == 1:
            replacements[(oldpath, anchor)] = pref[0]
            continue
        pref2 = [c for c in candidates if c.startswith(parent_dir)]
        if len(pref2) == 1:
            replacements[(oldpath, anchor)] = pref2[0]
            continue
        unresolved.append((oldpath, anchor, "ambiguous:" + ",".join(sorted(set(candidates)))))

    print(
        f"unique citations: {len(citations)}  already-valid: {valid}  "
        f"to-fix: {len(replacements)}  unresolved: {len(unresolved)}"
    )
    print("\n=== REPLACEMENTS (oldpath -> new : anchor) ===")
    for (op, anchor), new in sorted(replacements.items()):
        print(f"{op}  ->  {new}   :{anchor}")
    print("\n=== UNRESOLVED (left untouched) ===")
    for op, anchor, reason in sorted(unresolved):
        print(f"{op}:{anchor}   [{reason}]")

    if not apply:
        print("\n(dry run — pass --apply to rewrite spec files)")
        return 0

    changed = 0
    for p in spec_files:
        try:
            txt = open(p, encoding="utf-8").read()
        except OSError:
            continue
        orig = txt
        for (op, anchor), new in replacements.items():
            old_cite = f"[[{op}:{anchor}]]"
            new_cite = f"[[{new}]]" if ":" in new else f"[[{new}:{anchor}]]"
            txt = txt.replace(old_cite, new_cite)
        if txt != orig:
            open(p, "w", encoding="utf-8").write(txt)
            changed += 1
    print(f"\nAPPLIED to {changed} spec files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
