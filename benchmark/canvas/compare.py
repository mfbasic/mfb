#!/usr/bin/env python3
"""Compare a canvas benchmark run with the committed baseline.

    ./benchmark/canvas/compare.py canvas-<ts>.log            # against baseline/
    ./benchmark/canvas/compare.py NEW.log --base OTHER.log   # any two runs

Ratios are new / base on the `median` column. That is NOT the language suite's choice
(it ranks on `min`, benchmark/language/RANKING.md section 1.1), because here it is
measured to be the wrong one: across two identical full runs a row's `work` median
moved 1.09x at p90 and 1.22x at most, its `min` 1.40x and 1.74x, and `gpu` the same
way (1.04x/1.24x against 1.73x/2.19x). A row is ~119 frames, and its least catches the
odd frame the GPU ran at a higher clock. The headline is `work` -- the frame's fill,
encode, GPU wait and present -- with `gpu` beside it. `vsync` is the wait for the display and is
not compared. The matching .sums files are compared too: a changed pixel hash or draw
count is a change in what the renderer drew, whatever the times did.
"""

import argparse
import math
import os
import re
import sys

COMPARED = ["work", "gpu"]
# The log's four columns, in order; the comparison reads MEDIAN (see the docstring).
MEDIAN, AVERAGE, MIN, MAX = range(4)


def read_log(path):
    rows, row = {}, None
    for line in open(path, encoding="utf-8"):
        m = re.match(r"^(\S.*):\s*$", line)
        if m and not line.startswith("#"):
            row = m.group(1)
            rows[row] = {}
            continue
        m = re.match(r"^\s+(\w+)\s*:\s*([-\d.]+),\s*([-\d.]+),\s*([-\d.]+),\s*([-\d.]+)\s*$", line)
        if m and row:
            rows[row][m.group(1)] = float(m.group(2 + MEDIAN))
    return rows


def read_sums(path):
    sums = {}
    if os.path.exists(path):
        for line in open(path, encoding="utf-8"):
            m = re.match(r"(\w+) = (\S+)", line)
            if m:
                sums[m.group(1)] = m.group(2)
    return sums


def geomean(values):
    return math.exp(sum(map(math.log, values)) / len(values)) if values else float("nan")


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("new", help="a run's canvas-<ts>.log")
    ap.add_argument("--base", default=os.path.join(here, "baseline", "canvas.log"),
                    help="the log to compare against (default: baseline/canvas.log)")
    ap.add_argument("--threshold", type=float, default=1.25,
                    help="list rows that moved by at least this factor (default 1.25, just "
                         "above the largest move between two identical runs)")
    args = ap.parse_args()

    base, new = read_log(args.base), read_log(args.new)
    common = [r for r in new if r in base]
    print("%d rows compared; %d only in the new run, %d only in the base"
          % (len(common), len(set(new) - set(base)), len(set(base) - set(new))))
    for column in COMPARED:
        ratios = {r: new[r][column] / base[r][column] for r in common
                  if base[r].get(column, 0) > 0 and new[r].get(column, 0) > 0}
        slower = sorted(((v, r) for r, v in ratios.items() if v >= args.threshold), reverse=True)
        faster = sorted((v, r) for r, v in ratios.items() if v <= 1 / args.threshold)
        print("\n%s (median): geomean %.3f over %d rows, %d slower and %d faster by %.2fx or more"
              % (column, geomean(list(ratios.values())), len(ratios), len(slower), len(faster),
                 args.threshold))
        for v, r in slower + faster:
            print("  %6.2fx  %-31s %10.1f -> %10.1f us" % (v, r, base[r][column], new[r][column]))

    base_sums = read_sums(re.sub(r"\.log$", ".sums", args.base))
    new_sums = read_sums(re.sub(r"\.log$", ".sums", args.new))
    changed = sorted(k for k in new_sums if k in base_sums and new_sums[k] != base_sums[k])
    print("\nchecksums: %d compared, %d changed" % (len(set(new_sums) & set(base_sums)), len(changed)))
    for k in changed:
        print("  %-40s %s -> %s" % (k, base_sums[k], new_sums[k]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
