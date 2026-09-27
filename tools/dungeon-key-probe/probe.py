#!/usr/bin/env python3
"""bug-713 regression probe: examples/dungeon's key-reading helpers must not
conflate a malformed (non-UTF-8) input byte with genuine end of input.

`examples/dungeon` is a `--app` canvas program, so its own binary needs a
window and cannot be driven by piped stdin. Its key-reading functions
(`getChar`/`ready`/`readKey`/`badByteSentinel`) touch no canvas at all, though,
so bug-713 split them into their own file, `examples/dungeon/src/input.mfb`
(a normal source file in the real `examples/dungeon` project — it builds into
`dungeon.app` exactly as before; the split only isolates these functions from
the canvas-touching code around them).

This probe builds a separate, tiny console-mode driver project and copies
`examples/dungeon/src/input.mfb` into it **byte-for-byte** (`shutil.copy`) —
never a hand-retyped or regex-extracted duplicate that could silently drift
out of sync with what `examples/dungeon` actually ships. `mfb`'s project
loader requires every source root to resolve inside the project's own
directory (`path_within_project`, `src/ast/manifest.rs`), so a driver project
cannot point a `sources` root at a sibling example's directory directly; a
fresh copy of the exact file, taken immediately before every build, is the
closest equivalent available and is verified identical to the live file below
before every run.

Then it feeds the built driver piped stdin to prove:

1. A stray non-UTF-8 byte (`0xFF`) followed by good bytes does not read as
   "EOF" — the bad byte is dropped and the next real key is returned.
2. Genuine end of input (empty stdin) still reads as "EOF".
3. A clean, all-ASCII stream is read back unchanged.

Usage: python3 tools/dungeon-key-probe/probe.py
Exits 0 and prints PASS lines on success; exits 1 with a diagnostic on the
first failing case.
"""
import filecmp
import os
import shutil
import subprocess
import sys
import tempfile

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
MFB = os.path.join(REPO_ROOT, "target", "release", "mfb")
DUNGEON_INPUT_MFB = os.path.join(REPO_ROOT, "examples", "dungeon", "src", "input.mfb")

DRIVER_MAIN = """IMPORT io

FUNC main AS Integer
  LET key AS String = readKey()
  io::print("readKey() returned: [" & key & "]")
  RETURN 0
END FUNC
"""

PROJECT_JSON = """{
  "name": "dungeon-key-probe-driver",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "sources": [
    {
      "root": "src",
      "role": "main",
      "include": ["**/*.mfb"]
    }
  ],
  "entry": "main",
  "targets": ["native"]
}
"""


def build_driver(tmpdir: str) -> str:
    src_dir = os.path.join(tmpdir, "src")
    os.makedirs(src_dir, exist_ok=True)
    with open(os.path.join(tmpdir, "project.json"), "w", encoding="utf-8") as f:
        f.write(PROJECT_JSON)
    with open(os.path.join(src_dir, "main.mfb"), "w", encoding="utf-8") as f:
        f.write(DRIVER_MAIN)

    # Byte-for-byte copy of the REAL file — not a duplicate maintained here.
    copied_input = os.path.join(src_dir, "input.mfb")
    shutil.copy(DUNGEON_INPUT_MFB, copied_input)
    if not filecmp.cmp(DUNGEON_INPUT_MFB, copied_input, shallow=False):
        raise AssertionError(
            f"copy of {DUNGEON_INPUT_MFB} into the driver project is not "
            "byte-identical to the source — the probe would not be testing "
            "the real code")

    subprocess.run([MFB, "build", tmpdir], check=True, cwd=REPO_ROOT)
    return os.path.join(tmpdir, "build", "dungeon-key-probe-driver.out")


def run_with_stdin(binary: str, data: bytes):
    proc = subprocess.run([binary], input=data, capture_output=True, timeout=10)
    return proc.stdout.decode("utf-8", "replace"), proc.returncode


def main() -> int:
    if not os.path.isfile(DUNGEON_INPUT_MFB):
        print(f"FAIL: {DUNGEON_INPUT_MFB} does not exist — has "
              "examples/dungeon/src/input.mfb moved or been renamed?")
        return 1

    with tempfile.TemporaryDirectory(prefix="dungeon-key-probe-") as tmpdir:
        binary = build_driver(tmpdir)

        # Case 1: a leading bad byte must not read as EOF.
        out, code = run_with_stdin(binary, b"\xffabc")
        if "readKey() returned: [EOF]" in out:
            print(f"FAIL: a leading 0xFF byte was read as EOF (output: {out!r})")
            return 1
        if "readKey() returned: [a]" not in out:
            print(f"FAIL: expected the bad byte dropped and 'a' returned next "
                  f"(output: {out!r})")
            return 1
        print("PASS: a stray non-UTF-8 byte is dropped, not treated as EOF")

        # Case 2: genuine EOF (empty stdin) must still read as EOF.
        out, code = run_with_stdin(binary, b"")
        if "readKey() returned: [EOF]" not in out:
            print(f"FAIL: empty stdin should still read as EOF (output: {out!r})")
            return 1
        print("PASS: genuine end of input still reads as EOF")

        # Case 3: a clean stream is unaffected.
        out, code = run_with_stdin(binary, b"q")
        if "readKey() returned: [q]" not in out:
            print(f"FAIL: a clean 'q' byte should read back unchanged "
                  f"(output: {out!r})")
            return 1
        print("PASS: a clean UTF-8 stream reads back unchanged")

    return 0


if __name__ == "__main__":
    sys.exit(main())
