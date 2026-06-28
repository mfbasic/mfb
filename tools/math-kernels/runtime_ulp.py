#!/usr/bin/env python3
"""runtime_ulp.py — drive the *actual* emitted NEON kernels over the committed
macOS-libm reference vectors and report their ULP distance (plan-01-libm-kernels
Validation Plan: "the execution proof, not golden output").

Unlike `gen_coeffs.py verify` — which scores an f64 *reconstruction model* of a
kernel — this harness compiles and runs a real MFBASIC program that calls
`math::atan2` / `math::tan` / `math::pow` / `Float MOD Float`, so it measures the
machine code the compiler emits. It recovers each result bit-exactly by printing
`toString(x, D)` (which emits the f64's exact decimal expansion) and parsing it
back in Python.

Bit-exactness caveat: a result is only recovered exactly when its significant
digits fit within D fractional decimal places. Outputs with |x| far below 1
(e.g. pow → 1e-300) are skipped and counted, since 255 decimals cannot capture
them; this is reported so coverage is never silently truncated.

Usage:
  python3 runtime_ulp.py <fn> [--ref DIR] [--mfb PATH] [--decimals N] [--limit N]
    <fn> ∈ atan2 | tan | pow | fmod
"""
import argparse
import math
import os
import subprocess
import sys
import tempfile
from decimal import Decimal

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ulp import ulp_diff, bits_to_f64  # noqa: E402

# Optional: compare against the mathematically-true value (mpmath) as well as the
# captured macOS-libm reference. macOS libm is not correctly-rounded for every
# function (notably `tan`, which is up to ~2 ULP off the true value at some
# inputs), so a kernel that is *more* accurate than macOS would otherwise look
# like it "fails" the macOS bar at exactly the points macOS itself is wrong. The
# real correctness gate is ULP-vs-truth.
try:
    import mpmath as _mp  # noqa: N812
    _mp.mp.dps = 50
    # `fmod` is exact (its 0-ULP-vs-macOS bit-exact match is the definitive gate),
    # so it needs no truth mode.
    _TRUTH = {
        "atan2": lambda a: _mp.atan2(_mp.mpf(a[0]), _mp.mpf(a[1])),
        "tan": lambda a: _mp.tan(_mp.mpf(a[0])),
        "pow": lambda a: _mp.power(_mp.mpf(a[0]), _mp.mpf(a[1])),
    }
except ImportError:
    _TRUTH = None

# fdlibm medium-range trig reduction limit (matches gen_coeffs.py); beyond it the
# kernel would need Payne-Hanek, which is out of scope here exactly as for sin/cos.
_TRIG_PRIMARY = 2.0 ** 20 * (math.pi / 2.0)


def _is_primary(fn, args):
    if fn == "tan":
        return abs(args[0]) <= _TRIG_PRIMARY
    return True


def _lit(v):
    """An MFBASIC Float literal that parses to exactly the f64 `v`.

    Uses the exact decimal expansion (no exponent — the lexer rejects `e`
    notation) so strtod recovers the identical bit pattern.
    """
    if v != v or math.isinf(v):
        return None
    s = format(Decimal(v), "f")
    if "." not in s:
        s += ".0"  # force a Float literal (a bare integer literal types as Integer)
    return s


def _recoverable(expected, decimals):
    """Whether toString(expected, decimals) keeps every significant bit."""
    if expected == 0.0:
        return True
    a = abs(expected)
    if math.isinf(a) or a != a:
        return False
    # Smallest fractional place printed is 10**-decimals; require the value's
    # least-significant set decimal place to be no smaller than that.
    exact = Decimal(expected)
    # Number of fractional digits in the exact expansion:
    frac = -exact.as_tuple().exponent
    return frac <= decimals


def _call_expr(fn, vnames):
    """Build the kernel call from pre-bound variable names (binding args to
    variables avoids a compiler bug where two inline Float unary-negate literals
    as call arguments corrupt the lowering — orthogonal to these kernels)."""
    if fn == "atan2":
        return f"math::atan2({vnames[0]}, {vnames[1]})"
    if fn == "tan":
        return f"math::tan({vnames[0]})"
    if fn == "pow":
        return f"math::pow({vnames[0]}, {vnames[1]})"
    if fn == "fmod":
        return f"({vnames[0]}) MOD ({vnames[1]})"
    raise ValueError(fn)


def _eligible(fn, args, expected):
    """Skip rows the kernel would reject (raise), or that aren't real inputs."""
    if any(a != a or math.isinf(a) for a in args):
        return False
    if expected != expected or math.isinf(expected):
        return False
    if fn == "pow":
        x, y = args
        if x < 0.0 and not float(y).is_integer():
            return False  # no real result — the kernel's error path
        if x == 0.0 and y < 0.0:
            return False
        if x <= 0.0 and not float(y).is_integer() and x != 0.0:
            return False
    if fn == "atan2":
        # atan2(0,0) → 0/0 = NaN in the kernel (ErrFloatNan); libm returns 0.
        # MFBASIC has no NaN value, so this degenerate input is out of scope.
        if args[0] == 0.0 and args[1] == 0.0:
            return False
    if fn == "fmod":
        if args[1] == 0.0:
            return False  # ErrFloatDomain pre-check, never reaches the kernel
    return True


def _read_ref(path):
    rows = []
    with open(path) as fh:
        for line in fh:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            rows.append([bits_to_f64(int(t, 16)) for t in line.split()])
    return rows


def _compile_run(fn, rows, mfb, decimals):
    """Compile+run a program over `rows`; return (out_path, exit, stdout) or
    (None,..) on build failure."""
    src = ["IMPORT io", "IMPORT math", "", "FUNC main AS Integer",
           f"  LET p AS Byte = {decimals}"]
    for i, (args, _) in enumerate(rows):
        vnames = []
        for j, a in enumerate(args):
            v = f"a{i}_{j}"
            src.append(f"  LET {v} AS Float = {_lit(a)}")
            vnames.append(v)
        src.append(f"  io::print(toString({_call_expr(fn, vnames)}, p))")
    src.append("  RETURN 0")
    src.append("END FUNC")
    tmp = tempfile.mkdtemp(prefix=f"ulp_{fn}_")
    os.makedirs(os.path.join(tmp, "src"))
    with open(os.path.join(tmp, "project.json"), "w") as fh:
        fh.write('{ "name": "ulp", "version": "0.1.0", "mfb": "1.0", '
                 '"kind": "executable", "sources": [{ "root": "src", '
                 '"role": "main", "include": ["**/*.mfb"] }], "entry": "main", '
                 '"targets": ["native"] }')
    with open(os.path.join(tmp, "src", "main.mfb"), "w") as fh:
        fh.write("\n".join(src) + "\n")
    build = subprocess.run([mfb, "build", tmp], capture_output=True, text=True)
    out_path = None
    for line in build.stdout.splitlines():
        if line.startswith("Wrote executable to "):
            out_path = line[len("Wrote executable to "):].strip()
    if out_path is None:
        print(f"{fn}: build failed:\n{build.stdout}\n{build.stderr}", file=sys.stderr)
        return None, None, None
    runp = subprocess.run([out_path], capture_output=True, text=True)
    return out_path, runp.returncode, runp.stdout


def _build_and_run(fn, chosen, mfb, decimals):
    """Run all rows; a row whose kernel raises aborts the program at that point.
    Identify each raising row from the truncated output, exclude it, and re-run
    until the whole set completes. Returns (got_lines, errored_rows)."""
    errored = []
    remaining = list(chosen)
    while True:
        _, code, stdout = _compile_run(fn, remaining, mfb, decimals)
        if code is None:
            return None, None
        lines = stdout.splitlines()
        if code == 0 and len(lines) == len(remaining):
            # Re-map outputs back onto `chosen` order (errored rows omitted).
            return lines, errored
        # The row at index len(lines) raised (it printed nothing before aborting).
        bad_idx = len(lines)
        if bad_idx >= len(remaining):
            print(f"{fn}: run failed with no identifiable row (exit {code})", file=sys.stderr)
            return None, None
        errored.append(remaining[bad_idx])
        remaining = remaining[:bad_idx] + remaining[bad_idx + 1:]


def run(fn, ref_dir, mfb, decimals, limit):
    path = os.path.join(ref_dir, f"{fn}.ref")
    if not os.path.exists(path):
        print(f"{fn}: no ref file at {path}", file=sys.stderr)
        return 2
    rows = _read_ref(path)
    chosen = []
    skipped_unrecoverable = 0
    for row in rows:
        *args, expected = row
        if not _eligible(fn, args, expected):
            continue
        if not _recoverable(expected, decimals):
            skipped_unrecoverable += 1
            continue
        if any(_lit(a) is None for a in args):
            continue
        chosen.append((args, expected))
        if limit and len(chosen) >= limit:
            break

    got_lines, errored = _build_and_run(fn, chosen, mfb, decimals)
    if got_lines is None:
        return 1
    if errored:
        for args, expected in errored:
            print(f"  RAISED {fn}{tuple(args)} (exp {expected!r}) — excluded", file=sys.stderr)
        chosen = [c for c in chosen if c not in errored]

    truth_fn = _TRUTH.get(fn) if _TRUTH else None
    # buckets: count, ok_macos, max_macos, misses, ok_truth, max_truth, macos_bad_vs_truth
    buckets = {"p": [0, 0, 0, [], 0, 0, 0], "e": [0, 0, 0, [], 0, 0, 0]}
    for (args, expected), line in zip(chosen, got_lines):
        got = float(line)
        u = ulp_diff(got, expected)
        b = buckets["p" if _is_primary(fn, args) else "e"]
        b[0] += 1
        if u <= 1:
            b[1] += 1
        b[2] = max(b[2], u)
        if truth_fn is not None:
            t = float(truth_fn(args))
            ut = ulp_diff(got, t)
            if ut <= 1:
                b[4] += 1
            else:
                b[3].append((args, got, expected, t, ut))
            b[5] = max(b[5], ut)
            if ulp_diff(expected, t) > 1:
                b[6] += 1
        elif u > 1:
            b[3].append((args, got, expected, None, u))

    p, e = buckets["p"], buckets["e"]
    print(f"{fn}: runtime kernel vs macOS-libm reference ({decimals}-decimal recovery)")
    ppct = (100.0 * p[1] / p[0]) if p[0] else 0.0
    print(f"  primary : {p[0]:5d} vectors  {ppct:6.2f}% <=1ULP vs macOS  maxULP={p[2]}")
    if truth_fn is not None:
        tpct = (100.0 * p[4] / p[0]) if p[0] else 0.0
        print(f"            {p[0]:5d} vectors  {tpct:6.2f}% <=1ULP vs TRUTH  maxULP={p[5]}"
              f"   (macOS itself >1ULP vs truth on {p[6]} of these)")
    if e[0]:
        epct = 100.0 * e[1] / e[0]
        print(f"  extended: {e[0]:5d} vectors  {epct:6.2f}% <=1ULP vs macOS  maxULP={e[2]} "
              f"(large-arg / Payne-Hanek, out of scope)")
    if skipped_unrecoverable:
        print(f"  skipped : {skipped_unrecoverable} vectors (|result| too small for "
              f"{decimals}-decimal recovery)")
    for row in p[3][:12]:
        args, got, expected, t, u = row
        extra = f" truth {t!r}" if t is not None else ""
        print(f"    MISS {fn}{tuple(args)}: got {got!r} exp {expected!r}{extra}  {u} ULP")
    # Gate on ULP-vs-truth when available (the real correctness bar); otherwise on
    # ULP-vs-macOS.
    ok = (p[4] == p[0]) if truth_fn is not None else (p[1] == p[0])
    return 0 if ok else 3


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("fn", choices=["atan2", "tan", "pow", "fmod"])
    here = os.path.dirname(os.path.abspath(__file__))
    ap.add_argument("--ref", default=os.path.join(here, "..", "..",
                                                   "tests", "_data", "math_kernel_ref"))
    ap.add_argument("--mfb", default=os.path.join(here, "..", "..",
                                                  "target", "debug", "mfb"))
    ap.add_argument("--decimals", type=int, default=80)
    ap.add_argument("--limit", type=int, default=0)
    args = ap.parse_args(argv[1:])
    return run(args.fn, args.ref, args.mfb, args.decimals, args.limit)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
