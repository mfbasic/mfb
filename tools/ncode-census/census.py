#!/usr/bin/env python3
"""Instruction-form census over `mfb build -ncode` dumps (plan-171-A).

Reads `.ncode` dumps (the `mfb-native-code-plan` JSON: one object per
instruction, `{ "op": ..., <fields> }`, after register allocation and frame
finalization) and counts instruction-shape patterns that the plan-171 family
turns into denser instruction forms, plus every op's occurrence count.

Modes:
  census.py [--label NAME] DUMP...            TSV of project + total counts
  census.py --pairs DUMP...                   "path instrs str_pairs ldr_pairs"
  census.py --top METRIC [N] DUMP...          the N functions highest in METRIC
  census.py --diff BEFORE.tsv AFTER.tsv [--all-scopes]
                                              per-metric before/after/delta

Python 3, stdlib only. See README.md for the metric definitions.
"""

import json
import sys
from collections import Counter

PAIR_OPS = ("str_u64", "ldr_u64")
# The signed, scaled imm7 window of a 64-bit LDP/STP: -64*8 ..= 63*8.
PAIR_MIN, PAIR_MAX = -512, 504
BRANCH_OPS = {
    "b.eq", "b.ne", "b.ge", "b.lt", "b.gt", "b.le", "b.vc", "b.vs", "b.hi",
    "b.lo", "b.mi", "b.ls",
}


def load(path):
    """The dump's functions as (name, [instruction dict]) pairs."""
    with open(path, encoding="utf-8") as handle:
        plan = json.load(handle)
    return [(f.get("symbol") or f.get("name"), f.get("instructions", []))
            for f in plan.get("functions", [])]


def numeric(value):
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def pair_candidate(a, b):
    """Two adjacent 8-byte accesses of one kind through one base, 8 bytes apart:
    the lower offset, or None when the two are not a candidate at all."""
    if a["op"] != b["op"] or a["op"] not in PAIR_OPS:
        return None
    if a.get("base") is None or a.get("base") != b.get("base"):
        return None
    oa, ob = numeric(a.get("offset")), numeric(b.get("offset"))
    if oa is None or ob is None or abs(oa - ob) != 8:
        return None
    return min(oa, ob)


def pair_rejections(a, b, lower):
    """Why a candidate cannot become one LDP/STP (the rules of
    `optimizer::opt2::pairing`); empty when it can."""
    reasons = []
    if lower < PAIR_MIN or lower > PAIR_MAX:
        reasons.append("range")
    if lower % 8 != 0:
        reasons.append("align")
    if a["op"] == "ldr_u64":
        if a.get("dst") == a.get("base"):
            reasons.append("dst_is_base")
        if a.get("dst") == b.get("dst"):
            reasons.append("same_dst")
    return reasons


def is_zero(value):
    return value in ("0", "false")


def is_one(value):
    return value in ("1", "true")


def power_of_two(value):
    number = numeric(value)
    return number is not None and number > 0 and number & (number - 1) == 0


def census_function(instructions):
    """Pattern counts for one function's instruction stream."""
    counts = Counter()
    counts["instructions"] = len(instructions)
    ops = [i.get("op") for i in instructions]
    for op in ops:
        counts["op:" + op] += 1
    n = len(instructions)
    for k in range(n - 1):
        a, b = instructions[k], instructions[k + 1]
        lower = pair_candidate(a, b)
        if lower is not None:
            kind = "str" if a["op"] == "str_u64" else "ldr"
            counts[kind + "_pair_candidates"] += 1
            reasons = pair_rejections(a, b, lower)
            if reasons:
                for reason in reasons:
                    counts["pair_reject:" + reason] += 1
            else:
                counts[kind + "_pair_legal"] += 1
        if a["op"] == "cmp_imm" and is_zero(a.get("rhs")):
            if b["op"] in ("b.eq", "b.ne"):
                counts["cmp0_eq_branch"] += 1
            elif b["op"] in ("b.lt", "b.ge"):
                counts["cmp0_sign_branch"] += 1
        if (a["op"] == "mov_imm" and b["op"] in ("and", "orr", "eor")
                and a.get("dst") in (b.get("lhs"), b.get("rhs"))):
            counts["movimm_feeds_logical"] += 1
    for k in range(n - 3):
        m, l, c, br = instructions[k:k + 4]
        if (m["op"] == "mov_imm" and power_of_two(m.get("value"))
                and l["op"] == "and" and m.get("dst") in (l.get("lhs"), l.get("rhs"))
                and c["op"] == "cmp_imm" and c.get("lhs") == l.get("dst")
                and is_zero(c.get("rhs")) and br["op"] in ("b.eq", "b.ne")):
            counts["single_bit_test"] += 1
    for k in range(n - 5):
        bc, m0, b, l1, m1, l2 = instructions[k:k + 6]
        if (bc["op"] in BRANCH_OPS and m0["op"] == "mov_imm" and b["op"] == "b"
                and l1["op"] == "label" and l1.get("name") == bc.get("target")
                and m1["op"] == "mov_imm" and m1.get("dst") == m0.get("dst")
                and l2["op"] == "label" and l2.get("name") == b.get("target")
                and ((is_zero(m0.get("value")) and is_one(m1.get("value")))
                     or (is_one(m0.get("value")) and is_zero(m1.get("value"))))):
            counts["bool_diamond"] += 1
    k = 0
    while k < n - 1:
        run, register = 0, None
        while (k + 1 < n and instructions[k]["op"] == "cmp_imm"
               and instructions[k + 1]["op"] == "b.eq"
               and (register is None or instructions[k].get("lhs") == register)):
            register = instructions[k].get("lhs")
            run += 1
            k += 2
        if run >= 3:
            counts["match_chains"] += 1
            counts["match_chain_compares"] += run
        if run == 0:
            k += 1
    return counts


def write_tsv(rows, out):
    out.write("project\tfunction\tmetric\tvalue\n")
    for project, function, metric, value in rows:
        out.write(f"{project}\t{function}\t{metric}\t{value}\n")


def census(paths, labels):
    rows, total = [], Counter()
    for path, label in zip(paths, labels):
        project = Counter()
        for _, instructions in load(path):
            project += census_function(instructions)
            project["functions"] += 1
        total += project
        rows.extend((label, "*", metric, project[metric]) for metric in sorted(project))
    rows.extend(("*", "*", metric, total[metric]) for metric in sorted(total))
    return rows


def pairs_table(paths):
    """The prototype's table (plan-171-A section 2.2): whole-dump instruction count
    (labels included), store pairs, and load pairs whose first destination is
    neither the base nor the second destination. No offset-range filter."""
    for path in paths:
        instructions = [i for _, body in load(path) for i in body]
        stores = loads = 0
        for a, b in zip(instructions, instructions[1:]):
            if pair_candidate(a, b) is None:
                continue
            if a["op"] == "str_u64":
                stores += 1
            elif a.get("dst") not in (a.get("base"), b.get("dst")):
                loads += 1
        print(path, len(instructions), stores, loads)


def top(metric, limit, paths):
    scored = []
    for path in paths:
        for name, instructions in load(path):
            value = census_function(instructions)[metric]
            if value:
                scored.append((value, path, name))
    scored.sort(key=lambda row: (-row[0], row[1], row[2]))
    for value, path, name in scored[:limit]:
        print(f"{value}\t{path}\t{name}")


def read_tsv(path):
    table = {}
    with open(path, encoding="utf-8") as handle:
        next(handle)
        for line in handle:
            project, function, metric, value = line.rstrip("\n").split("\t")
            table[(project, function, metric)] = int(value)
    return table


def diff(before_path, after_path, all_scopes):
    before, after = read_tsv(before_path), read_tsv(after_path)
    keys = sorted(set(before) | set(after),
                  key=lambda key: (key[0] != "*", key[0], key[1], key[2]))
    print("project\tmetric\tbefore\tafter\tdelta")
    for key in keys:
        if key[0] != "*" and not all_scopes:
            continue
        old, new = before.get(key, 0), after.get(key, 0)
        if old == new and not key[2].startswith(("str_pair", "ldr_pair", "op:stp", "op:ldp")):
            continue
        print(f"{key[0]}\t{key[2]}\t{old}\t{new}\t{new - old:+d}")


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0 if argv else 2
    if argv[0] == "--pairs":
        pairs_table(argv[1:])
        return 0
    if argv[0] == "--top":
        metric = argv[1]
        rest = argv[2:]
        limit = 20
        if rest and rest[0].isdigit():
            limit, rest = int(rest[0]), rest[1:]
        top(metric, limit, rest)
        return 0
    if argv[0] == "--diff":
        diff(argv[1], argv[2], "--all-scopes" in argv[3:])
        return 0
    labels, paths = [], []
    args = iter(argv)
    pending_label = None
    for arg in args:
        if arg == "--label":
            pending_label = next(args)
            continue
        paths.append(arg)
        labels.append(pending_label or arg)
        pending_label = None
    write_tsv(census(paths, labels), sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
