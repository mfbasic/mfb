#!/usr/bin/env python3
"""Extract every fenced code block under an `## Examples` heading from the man
pages under src/docs/man/ and verify each one compiles with `mfb build -q`.

Usage:
    check-man-examples.py [path-glob-substring ...]

Prints one FAIL record per non-compiling block and a final summary.

Why this exists (bug-336): 627 of the corpus's 1012 example blocks did not
compile. A man-page example is the part a reader copies, so an example that does
not build is the same defect class as a page that misstates an error code — and
it is the one a reader hits first.

Two block shapes are legitimately not standalone programs and are reported
separately rather than as failures: a block with no `main` entry (a `TCASE` body,
a worker `ISOLATED FUNC`, a `MATCH`/`LAMBDA` fragment), and a block importing a
non-builtin package (every `thread` example needs a second package supplying the
worker). A fence tagged with a non-MFBASIC language (```text, ```c) is skipped:
those are sample output, not code.
"""
import concurrent.futures
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = "/Users/justinzaun/Development/mfb"
MAN = os.path.join(ROOT, "src", "docs", "man")
MFB = os.path.join(ROOT, "target", "release", "mfb")

PROJECT = {
    "name": "probe",
    "version": "0.1.0",
    "mfb": "1.0",
    "kind": "executable",
    "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}],
    "entry": "main",
    "targets": ["native"],
}

HEADING = re.compile(r"^##\s+(.*?)\s*$")
FENCE = re.compile(r"^```")

# A fence tagged with any language other than MFBASIC is not MFBASIC source --
# console output samples (```text) and other-language tours (```c, ```python)
# are rendered verbatim and must not be compiled.
MFB_LANGS = {"", "mfb", "mfbasic", "basic"}


def is_mfb_fence(line):
    return line.strip()[3:].strip().lower() in MFB_LANGS


def extract(path):
    """Yield (line_no, code) for each fenced block under an `## Examples` heading."""
    with open(path, "r", encoding="utf-8") as fh:
        lines = fh.read().split("\n")
    blocks = []
    in_examples = False
    i = 0
    while i < len(lines):
        m = HEADING.match(lines[i])
        if m:
            in_examples = m.group(1).lower() == "examples"
            i += 1
            continue
        if in_examples and FENCE.match(lines[i]):
            start = i
            mfb = is_mfb_fence(lines[i])
            i += 1
            body = []
            while i < len(lines) and not FENCE.match(lines[i]):
                body.append(lines[i])
                i += 1
            if mfb:
                blocks.append((start + 1, "\n".join(body)))
        i += 1
    return blocks


MAIN_DECL = re.compile(r"(?mi)^\s*(EXPORT\s+)?(ISOLATED\s+)?(SUB|FUNC)\s+main\b")


def build(code):
    """Return (ok, diagnostic).

    A block that declares no `main` is a library fragment, not a whole program;
    a synthetic empty `main` is appended so such a block is still compile-checked
    rather than rejected with PROJECT_ENTRY_INVALID.
    """
    if not MAIN_DECL.search(code):
        code = code + "\n\nSUB main()\nEND SUB\n"
    d = tempfile.mkdtemp(prefix="manprobe.")
    try:
        os.makedirs(os.path.join(d, "src"))
        with open(os.path.join(d, "project.json"), "w") as fh:
            json.dump(PROJECT, fh)
        with open(os.path.join(d, "src", "main.mfb"), "w") as fh:
            fh.write(code + "\n")
        p = subprocess.run(
            [MFB, "build", "-q", d],
            capture_output=True, text=True, timeout=180, cwd=d,
        )
        if p.returncode == 0:
            return True, ""
        return False, (p.stdout + p.stderr).strip()
    except subprocess.TimeoutExpired:
        return False, "TIMEOUT"
    finally:
        shutil.rmtree(d, ignore_errors=True)


def main():
    filters = sys.argv[1:]
    pages = []
    for dirpath, _, names in os.walk(MAN):
        for n in sorted(names):
            if n.endswith(".md"):
                pages.append(os.path.join(dirpath, n))
    pages.sort()

    jobs = []
    for page in pages:
        rel = os.path.relpath(page, ROOT)
        if filters and not any(f in rel for f in filters):
            continue
        for line_no, code in extract(page):
            jobs.append((rel, line_no, code))

    failures = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as ex:
        futs = {ex.submit(build, code): (rel, ln, code) for rel, ln, code in jobs}
        for fut in concurrent.futures.as_completed(futs):
            rel, ln, code = futs[fut]
            ok, diag = fut.result()
            if not ok:
                failures.append((rel, ln, code, diag))

    failures.sort(key=lambda t: (t[0], t[1]))
    for rel, ln, code, diag in failures:
        print("=" * 78)
        print(f"FAIL {rel}:{ln}")
        print("--- code ---")
        print(code)
        print("--- diagnostic ---")
        print(diag)
    print("=" * 78)
    print(f"checked {len(jobs)} example blocks across {len(pages)} pages")
    print(f"failures: {len(failures)}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
