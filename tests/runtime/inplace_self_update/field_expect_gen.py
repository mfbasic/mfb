#!/usr/bin/env python3
"""plan-145-A: generate `field_expect.tsv` from plan-144's findings cell tables.

    python3 tests/runtime/inplace_self_update/field_expect_gen.py > \
        tests/runtime/inplace_self_update/field_expect.tsv

One output line per (cases.tsv line, field site): `signature \t site \t expect`.
The findings' §1 (record sites S3–S10) and §2 (`STATE` sites T1–T8) cell tables
are the input, read from
`planning/plan-144-findings/record-state-self-update-audit.md`:

    finding cell        expect
    y                   arm
    n (…)               copy:<letter>   the plan-145 letter that lands it
    n/a (…)             na:<diagnostic>

A cases.tsv line may run several statements (`x = append(x, 1) ; x = distinct(x)`).
Each statement is matched to its own findings row, and the line's expectation is
the worst of its statements': any `na` wins, then any `rebuild`, then the latest
letter; `arm` only when every statement is `arm`.

Which letter closes an `n` cell (plan-145-A §"plan-145 as a whole"):

  * the arm's letter — the 10 overloads with a record arm today are served by B's
    seam; D lands the cannot-reallocate arms, E the reallocating ones;
  * the site's letter — C for the mixed `WITH` (S10/T5), F nested (S6/T6), G the
    global (S5), H the loop and the capture (S7/T7/S9);
  * the later of the two. A not-last site (S3/T1/T3) is D's for a
    cannot-reallocate arm (Open Decision 2) and `rebuild:not-last-grow` for a
    reallocating one (letter E's non-goal: a grow would shift the next sibling).

Two families are not arms at any site:

  * plan-142's `exempt` lines -> `rebuild:new-value` (the result is a new value of
    unrelated size; plan-145-A §3 "Expectations");
  * `String` (`&`) -> `deferred:string` (Open Decision 1).

The 11 bug-671 rows (`n/a (does not compile: bug-67x)` in the findings' §2) were
re-measured against the fixed compiler in plan-145-A Phase 1: every such cell is
`n (no arm)`, and is treated so here.
"""
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
FINDINGS = os.path.join(ROOT, "planning", "plan-144-findings", "record-state-self-update-audit.md")
CASES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cases.tsv")

RECORD_SITES = ["S3", "S4", "S5", "S6", "S7", "S9", "S10"]
STATE_SITES = ["T1", "T2", "T3", "T4", "T5", "T6", "T7", "T8"]
SITES = RECORD_SITES + STATE_SITES

# The overloads a record/`STATE` arm serves today (findings Appendix B.1, B.6).
EXISTING = {"add", "append", "insert", "prepend", "remove", "removeAt", "removeKey", "set"}
# plan-145-D: arms whose lowering never stores a new block pointer (for the element
# kinds cases.tsv uses: Integer, Float, Fixed — all fixed-width).
NO_REALLOC_NEW = {"filter", "take", "drop", "mid", "distinct", "sort", "sortBy",
                  "intersection", "difference", "replace", "transform", "mapValues"}
# plan-145-E: arms that can reallocate.
REALLOC_NEW = {"union", "symmetricDifference", "merge"}
# Of the existing arms, the ones that grow (InlineGrow) — they keep `G17` at a
# not-last field.
EXISTING_GROW = {"add", "append", "insert", "prepend"}

ORDER = "BCDEFGH"
# The plan-145 letters that have landed: a pair whose closing letter is here is
# `arm`, not `copy:<letter>`. A letter adds itself when it flips its lines.
LANDED = {"B", "C", "D", "E", "F"}


def later(a, b):
    return a if ORDER.index(a) >= ORDER.index(b) else b


def table_rows(text, start, stop, sites):
    """{label: {site: cell}} for the table between two headings."""
    lines = text.splitlines()
    i = next(k for k, l in enumerate(lines) if l.startswith(start))
    j = next(k for k, l in enumerate(lines) if k > i and l.startswith(stop))
    out = {}
    for line in lines[i:j]:
        if not (line.startswith("| F1 ") or line.startswith("| F3 ")):
            continue
        cells = [c.strip() for c in line.split("|")[1:-1]]
        label = cells[0][3:]  # drop the `F1 ` family tag
        out[label] = dict(zip(sites, cells[2:2 + len(sites)]))
    return out


def label_signature(label):
    """`collections::add(...) AS Set OF T` from a findings row label."""
    m = re.match(r"`([^`]+)`", label)
    return m.group(1) if m else label


def main():
    text = open(FINDINGS).read()
    rec = table_rows(text, "## 1. Record sites", "### 1b.", RECORD_SITES)
    st = table_rows(text, "## 2. `STATE` sites", "### 2b.", STATE_SITES)
    rows = {}
    for label, cells in rec.items():
        rows.setdefault(label_signature(label), {}).update(cells)
    for label, cells in st.items():
        rows.setdefault(label_signature(label), {}).update(cells)

    cases = []
    for line in open(CASES):
        if not line.strip() or line.startswith("#"):
            continue
        cols = line.rstrip("\n").split("\t")
        cases.append((cols[0], cols[1], cols[2].split(" ; ")[0], cols[3].split(" ; ")))
    sigs = [c[0] for c in cases]

    def resolve(statement, decl_type):
        """The cases.tsv signature a statement `x = f(x, …)` calls."""
        rhs = statement.split(" = ", 1)[1]
        if rhs.startswith("x & "):
            return "& (value AS String, other AS String) AS String"
        fn = rhs.split("(", 1)[0]
        cands = [s for s in sigs if s.startswith(fn + "(")]
        if len(cands) > 1:
            # Disambiguate by the first parameter's type and the item's shape.
            elem = decl_type.split()[-1] if decl_type.startswith("List OF") else None
            first = decl_type.split()[0]
            cands = [s for s in cands if f"value AS {first}" in s or f"a AS {first}" in s
                     or f"({first}" in s or "value AS List OF T" in s and first == "List"
                     or "value AS Map OF K" in s and first == "Map"
                     or "value AS Set OF T" in s and first == "Set"]
            if elem and len(cands) > 1:
                typed = [s for s in cands if f"List OF {elem})" in s or f"List OF {elem}," in s]
                if typed:
                    cands = typed
            if len(cands) > 1 and fn == "collections::append":
                list_item = "ys" in rhs
                cands = [s for s in cands if ("item AS List OF T" in s) == list_item]
        if len(cands) != 1:
            sys.exit(f"cannot resolve `{statement}` ({decl_type}): {cands}")
        return cands[0]

    def op_name(sig):
        return sig.split("(", 1)[0].split("::")[-1]

    def cell_expect(sig, site, cell):
        cell = re.sub(r"n/a \(does not compile: bug-67\d\)", "n (no arm)", cell)
        if cell == "y":
            return ("arm", None)
        if cell.startswith("n/a"):
            return ("na", "TYPE_FOR_EACH_REQUIRES_COLLECTION" if "FOR EACH" in cell else cell)
        assert cell.startswith("n ("), (sig, site, cell)
        op = op_name(sig)
        if sig.startswith("& "):
            return ("deferred", "string")
        if op in EXISTING:
            arm, grows = "B", op in EXISTING_GROW or (op == "set" and "Map" in sig)
        elif op in NO_REALLOC_NEW or sig.startswith("math::"):
            arm, grows = "D", False
        elif op in REALLOC_NEW:
            arm, grows = "E", True
        else:
            sys.exit(f"{sig}: no arm class")
        if site in ("S3", "T1", "T3"):
            return ("rebuild", "not-last-grow") if grows else ("copy", later("D", arm))
        site_letter = {"S4": "B", "T2": "B", "T4": "B", "T8": "B", "S10": "C", "T5": "C",
                       "S6": "F", "T6": "F", "S5": "G", "S7": "H", "T7": "H", "S9": "H"}[site]
        return ("copy", later(site_letter, arm))

    print("# plan-145-A: the expected outcome of every cases.tsv line at every field site.")
    print("# Generated by field_expect_gen.py from plan-144's findings; do not edit by hand")
    print("# except to flip a line the letter that lands it names (arm / rebuild / deferred / na).")
    print("# Columns: signature \\t site \\t expect")
    for sig, status, decl, statements in cases:
        decl_type = decl.split(" = ")[0]
        for site in SITES:
            if status == "exempt":
                expects = [("rebuild", "new-value")]
            else:
                expects = []
                for s in statements:
                    target = resolve(s, decl_type)
                    # The findings name the operator row by its shape, not a signature.
                    row = rows.get(target) or rows.get({"& (value AS String, other AS String) AS String": "s & t"}.get(target))
                    if row is None:
                        sys.exit(f"{target}: no findings row")
                    expects.append(cell_expect(target, site, row[site]))
            kinds = [k for k, _ in expects]
            if "na" in kinds:
                out = "na:" + next(v for k, v in expects if k == "na")
            elif "deferred" in kinds:
                out = "deferred:" + next(v for k, v in expects if k == "deferred")
            elif "rebuild" in kinds:
                out = "rebuild:" + next(v for k, v in expects if k == "rebuild")
            elif "copy" in kinds:
                letter = "B"
                for k, v in expects:
                    if k == "copy":
                        letter = later(letter, v)
                out = "arm" if letter in LANDED else "copy:" + letter
            else:
                out = "arm"
            print(f"{sig}\t{site}\t{out}")


main()
