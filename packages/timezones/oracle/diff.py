#!/usr/bin/env python3
"""Compare the probe's answers with the oracle's for one mode.

    python3 diff.py offsets

Reads jobs/<mode>.txt, jobs/<mode>.expected (oracle) and jobs/<mode>.actual
(probe). A line differing is a mismatch unless divergences.json declares it.
Each declaration has a `reason` and one of:
  "job":     an exact job line;
  "pattern": a regular expression searched in the job line, with "mode";
  "nameOnly": true, with "mode" — both sides accepted with the same instant and
             offset and differ only in the zone name they report.
Prints the first 20 remaining mismatches, then `<n> mismatches`, and exits 1 if
n > 0.
"""

import json
import os
import re
import sys


def declared_reason(declarations, mode, job, want, got):
    for entry in declarations:
        if "job" in entry and entry["job"] == job:
            return entry["reason"]
        if entry.get("mode") != mode:
            continue
        if "pattern" in entry and re.search(entry["pattern"], job):
            return entry["reason"]
        if entry.get("nameOnly"):
            a, b = want.split(" "), got.split(" ")
            if a[0] == b[0] == "accept" and len(a) == len(b) == 5 and a[1:4] == b[1:4]:
                return entry["reason"]
    return None

HERE = os.path.dirname(os.path.abspath(__file__))


def read_lines(path):
    with open(path, encoding="utf-8") as f:
        return f.read().splitlines()


def main():
    if len(sys.argv) != 2:
        sys.stderr.write("usage: diff.py <mode>\n")
        sys.exit(2)
    mode = sys.argv[1]
    jobs = read_lines(os.path.join(HERE, "jobs", mode + ".txt"))
    expected = read_lines(os.path.join(HERE, "jobs", mode + ".expected"))
    actual = read_lines(os.path.join(HERE, "jobs", mode + ".actual"))
    with open(os.path.join(HERE, "divergences.json"), encoding="utf-8") as f:
        declarations = json.load(f)

    if len(expected) != len(jobs) or len(actual) != len(jobs):
        print("%s: %d jobs but %d oracle answers and %d probe answers"
              % (mode, len(jobs), len(expected), len(actual)))
        sys.exit(1)

    mismatches = []
    excused = 0
    for job, want, got in zip(jobs, expected, actual):
        if want == got:
            continue
        if declared_reason(declarations, mode, job, want, got) is not None:
            excused += 1
            continue
        mismatches.append((job, want, got))

    for job, want, got in mismatches[:20]:
        print("MISMATCH %s\n  oracle: %s\n  probe:  %s" % (job, want, got))
    print("%s: %d jobs, %d declared divergences, %d mismatches"
          % (mode, len(jobs), excused, len(mismatches)))
    sys.exit(1 if mismatches else 0)


if __name__ == "__main__":
    main()
