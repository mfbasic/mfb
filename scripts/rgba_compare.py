#!/usr/bin/env python3
"""rgba_compare.py — compare two raw RGBA8 frames under Tolerance::GPU_DEFAULT (plan-131-D).

    python3 scripts/rgba_compare.py <reference.rgba> <candidate.rgba> [width]

Tolerance::GPU_DEFAULT: no pixel may differ by more than 2 steps in any channel, and no more
than 2% of pixels may differ at all. Prints one line:

    ok worst=<n> differing=<pct>%                           within tolerance
    worst=<n> differing=<pct>% first-beyond-tolerance=...   outside it (x, y, a, b)
    frame sizes differ (<a> vs <b>) — a harness bug        unusable input

Callers test for the `ok` prefix. `width` (default 900) only names the coordinate of the first
beyond-tolerance pixel. Shared by test-canvas-vulkan.sh and test-winapp.sh.
"""
import sys


def main():
    if len(sys.argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    reference = open(sys.argv[1], "rb").read()
    candidate = open(sys.argv[2], "rb").read()
    width = int(sys.argv[3]) if len(sys.argv) > 3 else 900
    if len(reference) != len(candidate) or not reference:
        print(f"frame sizes differ ({len(reference)} vs {len(candidate)}) — a harness bug")
        return 0
    worst = 0
    differing = 0
    first = None
    total = len(reference) // 4
    for i in range(0, len(reference), 4):
        a = reference[i:i + 4]
        b = candidate[i:i + 4]
        if a == b:
            continue
        differing += 1
        delta = max(abs(x - y) for x, y in zip(a, b))
        if delta > worst:
            worst = delta
        if first is None and delta > 2:
            pixel = i // 4
            first = (pixel % width, pixel // width, a.hex(), b.hex())
    fraction = differing / total
    if worst <= 2 and fraction <= 0.02:
        print(f"ok worst={worst} differing={fraction * 100:.4f}%")
    else:
        print(f"worst={worst} differing={fraction * 100:.4f}% first-beyond-tolerance={first}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
