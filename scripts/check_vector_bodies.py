#!/usr/bin/env python3
"""Drift gate for the vector FUNC bodies (bug-339 A1, post-split form).

`scripts/gen_vector_package.py` is still the single source of truth for the
`__vector_*` member arithmetic, but the checked-in bodies now live as `BODY*`
raw-string consts spread across `src/codegen/builtins/vector/func_*.rs` and
`helper_*.rs` (one FUNC per const). This checker regenerates the FUNC tail,
extracts every checked-in body, and fails when any FUNC's text drifts — so
"edit the generator and re-run it" remains safe and a hand-landed body change
cannot silently diverge from the generator.

Exit 0 on match; exit 1 with a per-FUNC diff summary on drift.
"""
import glob
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.path.join(ROOT, "src/codegen/builtins/vector")


def generated_funcs():
    out = subprocess.run(
        [sys.executable, os.path.join(ROOT, "scripts/gen_vector_package.py")],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    funcs = {}
    for m in re.finditer(r"^FUNC (__vector_\w+)\([\s\S]*?^END FUNC$", out, re.M):
        funcs[m.group(1)] = m.group(0)
    return funcs


def checked_in_funcs():
    funcs = {}
    for path in glob.glob(f"{PKG}/func_*.rs") + glob.glob(f"{PKG}/helper_*.rs"):
        text = open(path).read()
        for m in re.finditer(r'r#+"([\s\S]*?)"#+', text):
            body = m.group(1)
            fm = re.search(r"^FUNC (__vector_\w+)\(", body, re.M)
            if not fm:
                continue
            # strip any attached leading ' comment lines: the generator's FUNC
            # text is compared comment-free on both sides below.
            funcs[fm.group(1)] = body
    return funcs


def strip_comments(text):
    lines = [l for l in text.split("\n") if not l.startswith("'")]
    return "\n".join(lines).strip()


def main():
    gen = generated_funcs()
    checked = checked_in_funcs()
    status = 0
    for name, gbody in sorted(gen.items()):
        if name not in checked:
            print(f"DRIFT: {name} generated but not checked in", file=sys.stderr)
            status = 1
            continue
        if strip_comments(gbody) != strip_comments(checked[name]):
            print(
                f"DRIFT: {name} differs between scripts/gen_vector_package.py "
                f"and its checked-in BODY const",
                file=sys.stderr,
            )
            status = 1
    for name in sorted(set(checked) - set(gen)):
        print(f"DRIFT: {name} checked in but not generated", file=sys.stderr)
        status = 1
    if status == 0:
        print(f"ok: {len(gen)} vector FUNC bodies match scripts/gen_vector_package.py")
    return status


if __name__ == "__main__":
    sys.exit(main())
