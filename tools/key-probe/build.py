#!/usr/bin/env python3
"""Assemble and build a key-probe program (tools/key-probe).

    python3 tools/key-probe/build.py console <dir> [mfb build args...]
    python3 tools/key-probe/build.py app <dir> [mfb build args...]

Creates the project `<dir>` (replacing its sources) from this directory's
`keys.mfb` plus `main_console.mfb` or `main_app.mfb`, copied byte-for-byte, and
runs `mfb build` on it with any extra arguments (`-target linux-aarch64`, ...).
`app` adds `-app`. The compiler is `$MFB`, else `target/release/mfb` of this
checkout. Prints the build directory.
"""
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
MFB = os.environ.get("MFB", os.path.join(ROOT, "target", "release", "mfb"))

PROJECT = """{
  "name": "keyprobe",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}],
  "entry": "main",
  "targets": ["native"]
}
"""


def main() -> int:
    if len(sys.argv) < 3 or sys.argv[1] not in ("console", "app"):
        print(__doc__, file=sys.stderr)
        return 2
    kind, project = sys.argv[1], os.path.abspath(sys.argv[2])
    src = os.path.join(project, "src")
    shutil.rmtree(src, ignore_errors=True)
    os.makedirs(src)
    with open(os.path.join(project, "project.json"), "w", encoding="utf-8") as f:
        f.write(PROJECT)
    shutil.copyfile(os.path.join(HERE, "keys.mfb"), os.path.join(src, "keys.mfb"))
    shutil.copyfile(os.path.join(HERE, f"main_{kind}.mfb"), os.path.join(src, "main.mfb"))
    args = [MFB, "build"] + (["-app"] if kind == "app" else []) + sys.argv[3:] + [project]
    subprocess.run(args, check=True)
    print(os.path.join(project, "build"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
