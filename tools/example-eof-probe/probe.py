#!/usr/bin/env python3
"""bug-715 regression probe: four examples must not crash with the compiler's
raw error text and exit 255 when stdin ends before a read is satisfied.

`hello_input`, `wide-demo`, `hangman`, and `audio` each call `io::input`/
`io::readChar` at least once with no `TRAP` on a path reachable during normal
use. When stdin runs out, `ErrEndOfFile` propagates uncaught: the process
exits 255 and prints the compiler's raw `Error: 7-702-0003` text. For the two
alternate-screen apps (`wide-demo`, `hangman`), that text is also written
*before* the terminal is restored (`?1049l`), so it never reaches the user on
a real terminal.

This probe builds each example (via `target/release/mfb build`, no pty needed
— every case here is reproduced with plain piped/redirected stdin, matching
the bug doc's own reproduction) and asserts, for each:

1. Exit code is not 255.
2. Output does not contain the raw `Error: 7-702-0003` compiler text.
3. For `wide-demo`/`hangman` only: the terminal-restore sequence (`?1049l`)
   appears before the example's own error message in the byte stream.

Usage: python3 tools/example-eof-probe/probe.py
Exits 0 and prints PASS lines on success; exits 1 with a diagnostic on the
first failing case.
"""
import os
import subprocess
import sys

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
MFB = os.path.join(REPO_ROOT, "target", "release", "mfb")

RAW_ERROR = b"Error: 7-702-0003"
RESTORE_SEQ = b"\x1b[?1049l"


def build(example: str) -> str:
    example_dir = os.path.join(REPO_ROOT, "examples", example)
    subprocess.run([MFB, "build", example_dir], check=True, cwd=REPO_ROOT)
    # Console (non --app) examples build to <example>/build/<example>.out.
    return os.path.join(example_dir, "build", f"{example}.out")


def run(binary: str, stdin_data: bytes):
    proc = subprocess.run([binary], input=stdin_data, capture_output=True, timeout=10)
    return proc.returncode, proc.stdout + proc.stderr


def check_no_raw_crash(name: str, binary: str, stdin_data: bytes, prefix: bytes,
                        alt_screen: bool) -> None:
    code, output = run(binary, stdin_data)
    if code == 255:
        raise AssertionError(
            f"{name}: exited 255 (the runtime's default uncaught-error exit) "
            f"after stdin ended — output: {output!r}")
    if RAW_ERROR in output:
        raise AssertionError(
            f"{name}: raw compiler error text {RAW_ERROR!r} leaked into "
            f"output instead of an example-authored message — output: {output!r}")
    if prefix not in output:
        raise AssertionError(
            f"{name}: expected an example-authored error message starting "
            f"with {prefix!r} — output: {output!r}")
    if alt_screen:
        restore_at = output.find(RESTORE_SEQ)
        message_at = output.find(prefix)
        if restore_at == -1:
            raise AssertionError(
                f"{name}: expected the terminal-restore sequence {RESTORE_SEQ!r} "
                f"in the output — output: {output!r}")
        if restore_at > message_at:
            raise AssertionError(
                f"{name}: the error message was written before the terminal "
                f"was restored (restore at {restore_at}, message at "
                f"{message_at}) — a real terminal would never show it")
    print(f"PASS: {name} — exit {code}, no raw error text"
          + (", restored before reporting" if alt_screen else ""))


def main() -> int:
    cases = [
        ("hello_input", b"Bob\nw", b"hello_input:", False),
        ("wide-demo", b"", b"wide-demo:", True),
        ("hangman", b"", b"hangman:", True),
        ("audio", b"", b"audio:", False),
    ]
    for name, stdin_data, prefix, alt_screen in cases:
        binary = build(name)
        try:
            check_no_raw_crash(name, binary, stdin_data, prefix, alt_screen)
        except AssertionError as exc:
            print(f"FAIL: {exc}")
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
