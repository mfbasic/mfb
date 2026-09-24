#!/usr/bin/env python3
"""bug-686: pixel compare between two MFB_CANVAS_DUMP RGBA files.

Usage: cmp.py <file-a> <file-b>

Prints `bytes=<n> pixels=<n> maxdelta=<n>` where `pixels` is the count of
pixels differing by any channel and `maxdelta` is the largest single-channel
delta seen anywhere in the frame. Exits non-zero if the files are not the
same length (a size mismatch means the two dumps are not the same surface,
which is a bug in the caller, not a rendering difference).
"""
import sys


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: cmp.py <file-a> <file-b>", file=sys.stderr)
        return 2
    a = open(sys.argv[1], "rb").read()
    b = open(sys.argv[2], "rb").read()
    if len(a) != len(b):
        print(f"size mismatch: {len(a)} vs {len(b)}", file=sys.stderr)
        return 2
    bytes_ = sum(1 for x, y in zip(a, b) if x != y)
    px = 0
    mx = 0
    for i in range(0, len(a), 4):
        d = max(abs(a[i + k] - b[i + k]) for k in range(4))
        if d:
            px += 1
            mx = max(mx, d)
    print(f"bytes={bytes_} pixels={px} maxdelta={mx}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
