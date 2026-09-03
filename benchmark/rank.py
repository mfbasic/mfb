#!/usr/bin/env python3
"""Rank every benchmark row by how far MFBASIC is from its peers.

Reads the committed logs in benchmark/baseline (or a directory given as the
first argument) and emits the grade table, the section rollup and the
priority-ordered work queue defined in benchmark/RANKING.md.

    ./benchmark/rank.py                       # full report from the baseline
    ./benchmark/rank.py --dir bench-logs      # some other run's logs
    ./benchmark/rank.py --csv > rows.csv      # one line per row, for a diff

The reference target is `mfb-O1` (the default level) against `c-O0` (the
default level) with `python` as the second, independent reference.
"""

import argparse
import csv
import math
import os
import re
import sys
from collections import defaultdict

TARGETS = ["mfb-O1", "mfb-O2", "mfb-O3", "c-O0", "c-O2", "python"]

# --- the ranking constants (see benchmark/RANKING.md for the derivation) ------

GRADES = [  # (letter, upper bound on mfb/c-O0, one-line meaning)
    ("S", 1.0, "at or better than C -O0"),
    ("A", 2.5, "same order as C -O0"),
    ("B", 6.0, "visibly slower, no structural defect implied"),
    ("C", 15.0, "a slow path"),
    ("D", 40.0, "a wrong-shape implementation"),
    ("F", math.inf, "an algorithmic defect"),
]

RESOLUTION_MS = 0.001  # the logs print 3 decimals: 1 us is the smallest tick
FLOOR_MS = 0.010   # below this the row is timer noise, not a measurement
SMALL_MS = 0.050   # above FLOOR but too small to be worth an optimisation
NATIVE_LIB = 1.5   # python/c-O0 at or under this => the Python row is C code
INTERPRETED = 5.0  # python/c-O0 at or over this => Python is really interpreting

# Container matrices: the Record-/State-/key- variants have no C or Python peer,
# so they borrow the baseline of the same operation in the scalar section.
VARIANT_RE = re.compile(r"^(list|map|set) \((?:Record-|State-|key-)?(Fixed|Dynamic)\)$")


def base_section(section):
    m = VARIANT_RE.match(section)
    return "%s (%s)" % (m.group(1), m.group(2)) if m else None


# --- parsing -----------------------------------------------------------------

ROW_RE = re.compile(r"^\s+(\S+)\s*:\s*(.+)$")
SEC_RE = re.compile(r"^(\S.*?):\s*$")


def read_log(path):
    """-> {(section, row): (median, average, min, max)}; None for a `--` row."""
    out, section = {}, None
    with open(path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            line = line.rstrip("\n")
            if not line.strip() or line.startswith("#"):
                continue
            m = SEC_RE.match(line)
            if m:
                section = m.group(1).strip()
                continue
            m = ROW_RE.match(line)
            if m and section is not None:
                parts = [p.strip() for p in m.group(2).split(",")]
                out[(section, m.group(1))] = (
                    None if parts[0] == "--" else tuple(float(p) for p in parts)
                )
    return out


def load(dirpath):
    data, order = {}, []
    for t in TARGETS:
        path = os.path.join(dirpath, t + ".log")
        if not os.path.exists(path):
            sys.exit("error: %s not found" % path)
        data[t] = read_log(path)
    for key, val in read_log(os.path.join(dirpath, "mfb-O1.log")).items():
        if val is not None:
            order.append(key)
    return data, order


# --- ranking -----------------------------------------------------------------

MIN = 2  # index of the `min` column, the estimator this system ranks on


def grade_for(ratio):
    for letter, bound, _ in GRADES:
        if ratio <= bound:
            return letter
    return "F"


def rank(data, order):
    rows = []
    for key in order:
        mfb = data["mfb-O1"].get(key)
        if mfb is None:
            continue
        section, name = key
        c0 = data["c-O0"].get(key)
        py = data["python"].get(key)
        proxy = False
        if c0 is None or py is None:
            base = base_section(section)
            if base is None:
                continue  # mfb-exclusive feature: no peer, not rankable
            c0 = data["c-O0"].get((base, name))
            py = data["python"].get((base, name))
            if c0 is None or py is None:
                continue
            proxy = True

        m, c, p = mfb[MIN], c0[MIN], py[MIN]

        # Two independent qualities, deliberately not folded into one label:
        # `baseline` says whose C/Python time the ratio was taken against,
        # `confidence` says whether the row is big enough to have been measured.
        baseline = "proxy" if proxy else "direct"
        scale = max(m, c)
        if scale < FLOOR_MS:
            confidence = "noise"
        elif scale < SMALL_MS:
            confidence = "small"
        else:
            confidence = "firm"

        # Clamp every denominator at the log's print resolution: a peer that
        # rounded to 0.000 is "at most 1 us", not "infinitely fast".
        c_d, p_d = max(c, RESOLUTION_MS), max(p, RESOLUTION_MS)
        r_c, r_p, p_c = m / c_d, m / p_d, p / c_d
        ref = "native-lib" if p_c <= NATIVE_LIB else ("interpreted" if p_c >= INTERPRETED else "mixed")

        flag = ""
        if m > p:
            flag = "RED" if ref == "interpreted" else ("LIB" if ref == "native-lib" else "red")

        # Variant overhead: what the Record-/State-/key- element type costs
        # relative to the scalar sibling. Pure mfb-vs-mfb, so no borrowed
        # baseline and no cross-language caveat.
        overhead = None
        base = base_section(section)
        if base is not None and base != section:
            sib = data["mfb-O1"].get((base, name))
            if sib and sib[MIN] > 0:
                overhead = m / sib[MIN]

        # -O headroom: does the mfb optimiser already close this gap?
        best = min(v[MIN] for v in (data[t].get(key) for t in ("mfb-O1", "mfb-O2", "mfb-O3")) if v)
        rows.append(dict(
            section=section, name=name, mfb=m, c0=c, py=p,
            r_c=r_c, r_p=r_p, p_c=p_c, ref=ref,
            grade=grade_for(r_c), flag=flag,
            confidence=confidence, baseline=baseline, overhead=overhead,
            opt_gain=m / best if best > 0 else 1.0,
        ))
    return rows


GRADE_ORDER = {letter: i for i, (letter, _, _) in enumerate(GRADES)}


def clusters(rows, worst="C"):
    """Group the failing rows by operation name: one cluster ~ one root cause."""
    cut = GRADE_ORDER[worst]
    bad = [r for r in rows if GRADE_ORDER[r["grade"]] >= cut and r["confidence"] != "noise"]
    by_op = defaultdict(list)
    for r in bad:
        by_op[r["name"]].append(r)
    # How much a row's headroom counts toward its cluster's score: octaves of
    # gap (log2 is the natural unit -- a 4x row is worth twice a 2x row, not
    # four times), discounted by how much the measurement can be trusted.
    weight = {"firm": 1.0, "small": 0.25}
    out = []
    for op, rs in by_op.items():
        firm = [r for r in rs if r["confidence"] == "firm" and r["baseline"] == "direct"]
        score = sum(weight[r["confidence"]] * (0.5 if r["baseline"] == "proxy" else 1.0)
                    * math.log2(max(r["r_c"], 1.0)) for r in rs)
        out.append(dict(
            op=op, rows=rs, n=len(rs), n_firm=len(firm), score=score,
            worst=min(GRADE_ORDER[r["grade"]] for r in rs),
            max_ratio=max(r["r_c"] for r in rs),
            reds=sum(1 for r in rs if r["flag"] == "RED"),
            sections=sorted({r["section"] for r in rs}),
            # A cluster with no firm row rests entirely on borrowed baselines:
            # still worth reading, but it has not been measured against a peer.
            actionable=bool(firm),
        ))
    out.sort(key=lambda c: (-c["score"], -c["max_ratio"]))
    return out


# --- reporting ---------------------------------------------------------------

def report(rows):
    n = len(rows)
    print("MFBASIC benchmark ranking  --  mfb -O1 vs c -O0, min column, %d rows\n" % n)

    print("GRADE DISTRIBUTION")
    print("  %-2s %-9s %5s %6s   %s" % ("", "mfb/c-O0", "rows", "%", "meaning"))
    lo = 0.0
    for letter, bound, meaning in GRADES:
        got = [r for r in rows if r["grade"] == letter]
        span = "<= %.1fx" % bound if bound != math.inf else "> %.0fx" % lo
        print("  %-2s %-9s %5d %5.1f%%   %s" % (letter, span, len(got), 100 * len(got) / n, meaning))
        lo = bound
    firm = [r for r in rows if r["confidence"] == "firm" and r["baseline"] == "direct"]
    print("\n  baseline:   %d direct (real C/Python peer), %d proxy (borrowed from the scalar sibling)" % (
        sum(1 for r in rows if r["baseline"] == "direct"),
        sum(1 for r in rows if r["baseline"] == "proxy")))
    print("  confidence: %d firm, %d small (<%.3f ms), %d noise (<%.3f ms)" % (
        sum(1 for r in rows if r["confidence"] == "firm"),
        sum(1 for r in rows if r["confidence"] == "small"), SMALL_MS,
        sum(1 for r in rows if r["confidence"] == "noise"), FLOOR_MS))
    print("  rankable:   %d direct+firm rows carry the weight of the ranking" % len(firm))
    print("  vs CPython: %d RED (lose to the interpreter), %d LIB (lose to a C library)" % (
        sum(1 for r in rows if r["flag"] == "RED"),
        sum(1 for r in rows if r["flag"] == "LIB")))

    print("\n\nSECTION ROLLUP  (worst first, by share of rows at C or worse)")
    by_sec = defaultdict(list)
    for r in rows:
        by_sec[r["section"]].append(r)
    tbl = []
    for sec, rs in by_sec.items():
        bad = [r for r in rs if GRADE_ORDER[r["grade"]] >= GRADE_ORDER["C"]]
        med = sorted(r["r_c"] for r in rs)[len(rs) // 2]
        tbl.append((len(bad) / len(rs), len(bad), sec, len(rs), med,
                    sum(1 for r in rs if r["flag"] == "RED")))
    print("  %-24s %5s %6s %9s %5s" % ("section", "rows", ">=C", "median xC", "RED"))
    for share, nbad, sec, tot, med, reds in sorted(tbl, reverse=True):
        if nbad == 0:
            continue
        print("  %-24s %5d %3d/%-2d %8.1fx %5d" % (sec, tot, nbad, tot, med, reds))

    print("\n\nWORK QUEUE  (clusters of C-or-worse rows sharing an operation)")
    print("  one cluster is one likely root cause; score = confidence-weighted")
    print("  octaves of headroom summed over its rows (see RANKING.md)\n")
    print("  %-3s %-16s %7s %5s %5s %5s %9s  %s" % (
        "#", "operation", "score", "rows", "firm", "RED", "worst xC", "sections"))
    for i, cl in enumerate(clusters(rows), 1):
        secs = ", ".join(cl["sections"])
        if len(secs) > 52:
            secs = secs[:49] + "..."
        print("  %-3d %-16s %7.1f %5d %5d %5d %8.0fx  %s%s" % (
            i, cl["op"], cl["score"], cl["n"], cl["n_firm"], cl["reds"],
            cl["max_ratio"], secs, "" if cl["actionable"] else "  [proxy only]"))

    print("\n\nWORST 30 ROWS BY GRADE THEN RATIO (firm confidence only)")
    print("  %-2s %-4s %-22s %-16s %9s %9s %9s %8s %7s" % (
        "G", "flag", "section", "row", "mfb ms", "c-O0 ms", "py ms", "xC", "xPy"))
    ranked = sorted(firm, key=lambda r: (-GRADE_ORDER[r["grade"]], -r["r_c"]))
    for r in ranked[:30]:
        print("  %-2s %-4s %-22s %-16s %9.3f %9.3f %9.3f %7.1fx %6.1fx" % (
            r["grade"], r["flag"], r["section"], r["name"],
            r["mfb"], r["c0"], r["py"], r["r_c"], r["r_p"]))

    print("\n\nELEMENT-TYPE OVERHEAD  (mfb vs mfb: no borrowed baseline, no cross-language caveat)")
    print("  what a Record/State/key-typed element costs over the scalar sibling\n")
    ov = [r for r in rows if r["overhead"] is not None and r["mfb"] >= SMALL_MS]
    print("  rows measured: %d   >=2x: %d   >=10x: %d" % (
        len(ov), sum(1 for r in ov if r["overhead"] >= 2), sum(1 for r in ov if r["overhead"] >= 10)))
    print("  %-24s %-16s %9s %9s %9s" % ("section", "row", "mfb ms", "scalar ms", "overhead"))
    for r in sorted(ov, key=lambda r: -r["overhead"])[:15]:
        print("  %-24s %-16s %9.3f %9.3f %8.1fx" % (
            r["section"], r["name"], r["mfb"], r["mfb"] / r["overhead"], r["overhead"]))

    print("\n\nALREADY SOLVED BY -O2/-O3  (grade C+ at -O1, >=1.5x faster at a higher level)")
    opt = [r for r in firm if GRADE_ORDER[r["grade"]] >= GRADE_ORDER["C"] and r["opt_gain"] >= 1.5]
    if not opt:
        print("  none")
    for r in sorted(opt, key=lambda r: -r["opt_gain"]):
        print("  %-2s %-22s %-16s xC=%7.1f  -O gain %.1fx" % (
            r["grade"], r["section"], r["name"], r["r_c"], r["opt_gain"]))


def calibrate(data, order):
    """Re-derive the evidence behind the constants in RANKING.md section 1.

    The three mfb logs are three independent process runs of a nearly identical
    program (the median -O3/-O1 speedup is 1.05x), so the spread between them
    measures how stable each output column is.
    """
    def pct(v, ps=(50, 75, 90, 95, 100)):
        v = sorted(v)
        return [v[min(len(v) - 1, int(round(p / 100 * (len(v) - 1))))] for p in ps]

    mfbs = ("mfb-O1", "mfb-O2", "mfb-O3")
    print("1.1  ESTIMATOR STABILITY -- dispersion of mfb -O1/-O2/-O3 (max/min)")
    print("     %-8s %5s %6s %6s %6s %6s %6s" % ("column", "n", "p50", "p75", "p90", "p95", "max"))
    for idx, lab in ((0, "median"), (1, "average"), (2, "min")):
        d = []
        for k in order:
            vs = [data[t].get(k) for t in mfbs]
            if any(v is None for v in vs):
                continue
            col = [v[idx] for v in vs]
            if min(col) <= RESOLUTION_MS:
                continue
            d.append(max(col) / min(col))
        print("     %-8s %5d %6.2f %6.2f %6.2f %6.2f %6.2f" % ((lab, len(d)) + tuple(pct(d))))

    inert = [k for k in order
             if data["mfb-O2"].get(k) and data["mfb-O3"].get(k) and data["mfb-O2"][k][0] > 0.5
             and abs(data["mfb-O2"][k][0] - data["mfb-O3"][k][0]) / data["mfb-O2"][k][0] < 0.02]
    print("\n     restricted to the %d rows where the optimiser is provably inert" % len(inert))
    print("     (-O2 and -O3 medians agree within 2%, both over 0.5 ms) -- pure noise:")
    for idx, lab in ((0, "median"), (2, "min")):
        d = [max(data[t][k][idx] for t in mfbs) / max(RESOLUTION_MS, min(data[t][k][idx] for t in mfbs))
             for k in inert]
        print("     %-8s %5d %6.2f %6.2f %6.2f %6.2f %6.2f" % ((lab, len(d)) + tuple(pct(d))))

    print("\n1.2  BAND WIDTH -- rows that change grade when the same run is graded")
    print("     on `min` versus on `median` (a stand-in for re-running the suite)")
    direct = [k for k in order if data["c-O0"].get(k) and data["python"].get(k)]
    def band(r, bounds):
        return next((i for i, b in enumerate(bounds) if r <= b), len(bounds))
    for lab, bounds in (("2x   [1,2,4,8,16,32]", [1, 2, 4, 8, 16, 32]),
                        ("2.5x [1,2.5,6,15,40]", [1, 2.5, 6, 15, 40]),
                        ("3x   [1,3,9,27,81]", [1, 3, 9, 27, 81])):
        flips = pops = 0
        counts = defaultdict(int)
        for k in direct:
            m, c = data["mfb-O1"][k], data["c-O0"][k]
            a = band(m[2] / max(c[2], RESOLUTION_MS), bounds)
            b = band(m[0] / max(c[0], RESOLUTION_MS), bounds)
            flips += a != b
            counts[a] += 1
            pops += 1
        print("     %-22s %3d/%d flip (%2.0f%%)   population %s"
              % (lab, flips, pops, 100 * flips / pops,
                 [counts[i] for i in range(len(bounds) + 1)]))

    print("\n1.3  WHY NOT ABSOLUTE MILLISECONDS -- c -O0 `min` over the %d direct rows"
          % len(direct))
    cs = sorted(data["c-O0"][k][2] for k in direct)
    print("     p25=%.3f  p50=%.3f  p75=%.3f  max=%.3f ms;  %d rows under 1 ms"
          % (cs[len(cs) // 4], cs[len(cs) // 2], cs[3 * len(cs) // 4], cs[-1],
             sum(1 for x in cs if x < 1)))
    gate = [k for k in direct if data["mfb-O1"][k][2] <= data["c-O0"][k][2] + 10.0]
    worst = max(gate, key=lambda k: data["mfb-O1"][k][2] / max(data["c-O0"][k][2], RESOLUTION_MS))
    print("     a `mfb <= c-O0 + 10 ms` gate passes %d/%d rows, including %s %s at %.0fx"
          % (len(gate), len(direct), worst[0], worst[1],
             data["mfb-O1"][worst][2] / max(data["c-O0"][worst][2], RESOLUTION_MS)))

    print("\n     proxy-baseline sanity: mfb(variant)/mfb(scalar) for the same op")
    v = []
    for k in order:
        b = base_section(k[0])
        if b and b != k[0] and data["mfb-O1"].get(k) and data["mfb-O1"].get((b, k[1])):
            sib = data["mfb-O1"][(b, k[1])][2]
            if sib > RESOLUTION_MS:
                v.append(data["mfb-O1"][k][2] / sib)
    v.sort()
    print("     n=%d  p10=%.2f  p50=%.2f  p90=%.2f  (a proxy ratio is an over-estimate"
          % (len(v), v[len(v) // 10], v[len(v) // 2], v[9 * len(v) // 10]))
    print("     of unknown size -- direction is trustworthy, magnitude is not)")


def emit_csv(rows):
    w = csv.writer(sys.stdout)
    w.writerow(["section", "row", "grade", "flag", "baseline", "confidence", "ref_class",
                "mfb_min_ms", "c0_min_ms", "py_min_ms", "x_c", "x_py", "py_over_c", "opt_gain", "elem_overhead"])
    for r in rows:
        w.writerow([r["section"], r["name"], r["grade"], r["flag"], r["baseline"], r["confidence"], r["ref"],
                    "%.4f" % r["mfb"], "%.4f" % r["c0"], "%.4f" % r["py"],
                    "%.3f" % r["r_c"], "%.3f" % r["r_p"], "%.3f" % r["p_c"], "%.3f" % r["opt_gain"],
                    "" if r["overhead"] is None else "%.3f" % r["overhead"]])


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dir", default=os.path.join(here, "baseline"),
                    help="directory holding <target>.log (default: benchmark/baseline)")
    ap.add_argument("--csv", action="store_true", help="emit one CSV line per row instead of the report")
    ap.add_argument("--calibrate", action="store_true",
                    help="re-derive the evidence behind RANKING.md section 1")
    args = ap.parse_args()
    data, order = load(args.dir)
    if args.calibrate:
        calibrate(data, order)
        return
    rows = rank(data, order)
    (emit_csv if args.csv else report)(rows)


if __name__ == "__main__":
    main()
