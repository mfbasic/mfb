#!/usr/bin/env python3
"""mfbgen — generate large batches of VALID, COMPILING MFBASIC programs to test
the runtime.

The point: you want to hammer the runtime with thousands of programs that
exercise ONE language feature at a time (FOR loops, DO/LOOP, WHILE, recursion),
in every combination of LET/MUT bindings, bounds, steps, and bodies — without
hand-writing any of them.

How correctness is checked WITHOUT a hand-written expected answer:
Every program is derived from a random *spec* (a small data structure). Two
pure functions consume that spec — `emit()` turns it into MFBASIC source, and
`simulate()` interprets it in Python to compute the exact stdout the program
must produce. Because both come from the same spec, the generator is its own
oracle. A generated program is a runtime BUG iff, after it compiles, its real
stdout differs from `simulate()`'s prediction (or it crashes / hangs).

Everything is kept inside i64 range by construction (values are simulated and
any spec that would overflow is rejected and regenerated), so generated
programs run to completion deterministically — no checked-overflow failures,
no float nondeterminism, no map-iteration-order dependence.

Usage:
  # generate 10k FOR-loop programs into ./out/for
  python3 mfbgen.py gen --category for --count 10000 --out ./out/for --seed 1

  # build + run + check every program, report failures
  python3 mfbgen.py run --out ./out/for --mfb ./target/debug/mfb --jobs 8

Categories: for, doloop, while, recursion, all
"""

import argparse
import concurrent.futures
import json
import os
import random
import subprocess
import sys
from pathlib import Path

I64_MIN = -(2**63)
I64_MAX = 2**63 - 1

# ---------------------------------------------------------------------------
# Overflow guard used by the simulator. Every arithmetic result a generated
# program can observe must stay in i64 (else the real runtime would raise a
# checked ErrOverflow and the program would fail — which we do NOT want here).
# ---------------------------------------------------------------------------


class Overflow(Exception):
    pass


def chk(v):
    if not (I64_MIN <= v <= I64_MAX):
        raise Overflow(v)
    return v


def idiv_trunc(a, b):
    # Integer `/` (Integer result) and toInt(a DIV b) both truncate the quotient
    # toward zero (verified against the runtime).
    if b == 0:
        raise Overflow("div by zero")
    q = abs(a) // abs(b)
    return q if (a < 0) == (b < 0) else -q


def mfb_mod(a, b):
    # MFBASIC MOD: remainder has the sign of `a`, quotient truncates toward 0.
    if b == 0:
        raise Overflow("mod by zero")
    return a - idiv_trunc(a, b) * b


# MFBASIC runtime error codes this generator predicts (spec §4.1 / §8).
ERR_OVERFLOW = 77050010          # +, -, *, unary - past i64 range
ERR_INVALID_ARGUMENT = 77050002  # non-Float divide/MOD by zero
ERR_FLOAT_NAN = 77050013         # NaN observed at a boundary (e.g. 0.0/0.0)
ERR_FLOAT_OVERFLOW = 77050015    # infinity observed at a boundary (x/0, overflow)


def format_fixed(hundredths):
    """Render an exact Fixed value (given in hundredths) the way toString(Fixed)
    does: sign, then integer part, then exactly two fractional digits. Valid
    ONLY for values exact at 2 decimals — toString(Fixed) TRUNCATES toward zero,
    so non-exact values (e.g. Fixed division results) must not be value-checked."""
    sign = "-" if hundredths < 0 else ""
    a = abs(hundredths)
    return f"{sign}{a // 100}.{a % 100:02d}"


def fixed_literal(hundredths):
    """A Fixed source literal for an exact-2dp value, e.g. 225 -> '2.25F'."""
    return format_fixed(hundredths) + "F"


def float_literal(hundredths):
    """A Float source literal for an exact-2dp value, e.g. 225 -> '2.25f'.
    toString(Float) defaults to 2-decimal precision, so exact-2dp values format
    identically to Fixed via format_fixed()."""
    return format_fixed(hundredths) + "f"


def money_literal(hundredths):
    """A Money source literal for an exact-2dp value, e.g. 225 -> '2.25m'.
    Money has no toString overload, so Money values are rendered for checking
    via toString(toFixed(m)); quarter-valued amounts convert to Fixed exactly."""
    return format_fixed(hundredths) + "m"


# ===========================================================================
# Shared expression / statement model (used by for / doloop / while).
#
# A spec is plain Python data. `emit_expr`/`emit_stmt` render MFBASIC; the
# `Env`-based interpreter evaluates the same nodes. Keeping the two in lockstep
# is what makes the generator a trustworthy oracle.
# ===========================================================================

# Expression nodes are tuples tagged by their first element:
#   ("int", n)             integer literal
#   ("var", name)          binding read
#   ("bin", op, a, b)      op in {+, -, *, MOD}
#   ("cmp", op, a, b)      op in {<, <=, >, >=, =, <>}  -> Boolean (conditions only)


def emit_expr(e):
    tag = e[0]
    if tag == "int":
        return str(e[1])
    if tag == "var":
        return e[1]
    if tag == "fix":
        return fixed_literal(e[1])
    if tag == "flt":
        return float_literal(e[1])
    if tag == "mny":
        return money_literal(e[1])
    if tag == "bin":
        _, op, a, b = e
        return f"({emit_expr(a)} {op} {emit_expr(b)})"
    if tag == "divwrap":
        # DIV returns Float; wrap in toInt to get an exact, printable Integer.
        _, a, b = e
        return f"toInt(({emit_expr(a)} DIV {emit_expr(b)}))"
    if tag == "cmp":
        _, op, a, b = e
        return f"({emit_expr(a)} {op} {emit_expr(b)})"
    raise ValueError(e)


def eval_expr(e, env):
    tag = e[0]
    if tag == "int":
        return chk(e[1])
    if tag == "var":
        return env[e[1]]
    if tag == "bin":
        _, op, a, b = e
        x, y = eval_expr(a, env), eval_expr(b, env)
        if op == "+":
            return chk(x + y)
        if op == "-":
            return chk(x - y)
        if op == "*":
            return chk(x * y)
        if op == "/":
            return chk(idiv_trunc(x, y))
        if op == "MOD":
            return chk(mfb_mod(x, y))
    if tag == "divwrap":
        _, a, b = e
        x, y = eval_expr(a, env), eval_expr(b, env)
        return chk(idiv_trunc(x, y))
    if tag == "cmp":
        _, op, a, b = e
        x, y = eval_expr(a, env), eval_expr(b, env)
        return {
            "<": x < y,
            "<=": x <= y,
            ">": x > y,
            ">=": x >= y,
            "=": x == y,
            "<>": x != y,
        }[op]
    raise ValueError(e)


def type_of(e):
    """Static result type of an expression per the numeric promotion table
    (spec §4.1) — the oracle for typeName(...). Fixed dominates Integer; DIV
    always yields Float. All bindings this generator creates are Integer, so a
    bare `var` is Integer."""
    tag = e[0]
    if tag == "int":
        return "Integer"
    if tag == "fix":
        return "Fixed"
    if tag == "flt":
        return "Float"
    if tag == "mny":
        return "Money"
    if tag == "var":
        return "Integer"
    if tag == "divwrap":  # toInt(a DIV b) -> Integer
        return "Integer"
    if tag == "bin":
        _, op, a, b = e
        if op == "DIV":
            return "Float"
        # Promotion order is Fixed > Float > Integer (spec §4.1): Fixed dominates
        # even over Float, so Float op Fixed -> Fixed, but Float op Integer -> Float.
        ta, tb = type_of(a), type_of(b)
        if "Fixed" in (ta, tb):
            return "Fixed"
        if "Float" in (ta, tb):
            return "Float"
        return "Integer"
    raise ValueError(e)


# Statement nodes:
#   ("assign", name, expr)
#   ("print", expr)
#   ("if", cmp, stmt)                      inline IF ... THEN <stmt>
#   ("for", var, start, end, step, body)   body = list of stmts
#   ("dowhile", cond, body)                pre-test  DO WHILE cond ... LOOP
#   ("dountil", body, cond)                post-test DO ... LOOP UNTIL cond
#   ("while", cond, body)                  WHILE cond ... END WHILE


def emit_stmt(s, ind, out):
    pad = "  " * ind
    tag = s[0]
    if tag == "assign":
        out.append(f"{pad}{s[1]} = {emit_expr(s[2])}")
    elif tag == "print":
        out.append(f"{pad}io::print(toString({emit_expr(s[1])}))")
    elif tag == "if":
        inner = []
        emit_stmt(s[2], 0, inner)
        out.append(f"{pad}IF {emit_expr(s[1])} THEN {inner[0].strip()}")
    elif tag == "for":
        _, var, start, end, step, body = s
        step_txt = "" if step == ("int", 1) else f" STEP {emit_expr(step)}"
        out.append(f"{pad}FOR {var} = {emit_expr(start)} TO {emit_expr(end)}{step_txt}")
        for b in body:
            emit_stmt(b, ind + 1, out)
        out.append(f"{pad}NEXT")
    elif tag == "dowhile":
        _, cond, body = s
        out.append(f"{pad}DO WHILE {emit_expr(cond)}")
        for b in body:
            emit_stmt(b, ind + 1, out)
        out.append(f"{pad}LOOP")
    elif tag == "dountil":
        _, body, cond = s
        out.append(f"{pad}DO")
        for b in body:
            emit_stmt(b, ind + 1, out)
        out.append(f"{pad}LOOP UNTIL {emit_expr(cond)}")
    elif tag == "while":
        _, cond, body = s
        out.append(f"{pad}WHILE {emit_expr(cond)}")
        for b in body:
            emit_stmt(b, ind + 1, out)
        out.append(f"{pad}END WHILE")
    else:
        raise ValueError(s)


# A hard cap on interpreter iterations so a malformed spec can never make the
# GENERATOR hang (the generated program's termination is guaranteed by
# construction, but this protects us from our own bugs).
ITER_CAP = 5_000_000


def exec_stmt(s, env, out):
    tag = s[0]
    if tag == "assign":
        env[s[1]] = eval_expr(s[2], env)
    elif tag == "print":
        out.append(str(eval_expr(s[1], env)))
    elif tag == "if":
        if eval_expr(s[1], env):
            exec_stmt(s[2], env, out)
    elif tag == "for":
        _, var, start, end, step, body = s
        i = eval_expr(start, env)
        hi = eval_expr(end, env)
        st = eval_expr(step, env)
        n = 0
        while (st > 0 and i <= hi) or (st < 0 and i >= hi):
            env[var] = i
            for b in body:
                exec_stmt(b, env, out)
            i = chk(i + st)
            n += 1
            if n > ITER_CAP:
                raise Overflow("iter cap")
    elif tag in ("dowhile", "while"):
        cond, body = s[1], s[2]
        n = 0
        while eval_expr(cond, env):
            for b in body:
                exec_stmt(b, env, out)
            n += 1
            if n > ITER_CAP:
                raise Overflow("iter cap")
    elif tag == "dountil":
        _, body, cond = s
        n = 0
        while True:
            for b in body:
                exec_stmt(b, env, out)
            n += 1
            if n > ITER_CAP:
                raise Overflow("iter cap")
            if eval_expr(cond, env):
                break
    else:
        raise ValueError(s)


# ===========================================================================
# Program assembly (loop categories): bindings preamble + statement body.
# ===========================================================================


def render_program(bindings, body_stmts, trap=False):
    """bindings: list of (kind, name, init_expr). kind in {'LET','MUT'}.

    When `trap` is set, a function-level TRAP is appended that prints the caught
    error's code and returns — used by the deliberately-failing arithmetic
    programs so the printed output is the runtime error code.
    """
    lines = ["IMPORT io", "", "FUNC main AS Integer"]
    for kind, name, init in bindings:
        lines.append(f"  {kind} {name} = {emit_expr(init)}")
    tmp = []
    for s in body_stmts:
        emit_stmt(s, 1, tmp)
    lines.extend(tmp)
    lines.append("  RETURN 0")
    if trap:
        lines += ["", "  TRAP(e)", "    io::print(toString(e.code))", "    RETURN 0", "  END TRAP"]
    lines.append("END FUNC")
    return "\n".join(lines) + "\n"


def wrap_program(body_lines, trap=False):
    """Wrap already-rendered body source lines in a FUNC main. Used by the
    arithmetic generators, which build their source and expected output line by
    line (rather than through the statement model) because they interleave
    toString(...) value prints with typeName(...) type prints."""
    lines = ["IMPORT io", "", "FUNC main AS Integer"]
    lines += ["  " + line for line in body_lines]
    lines.append("  RETURN 0")
    if trap:
        lines += ["", "  TRAP(e)", "    io::print(toString(e.code))", "    RETURN 0", "  END TRAP"]
    lines.append("END FUNC")
    return "\n".join(lines) + "\n"


def simulate_program(bindings, body_stmts):
    env = {}
    for _kind, name, init in bindings:
        env[name] = eval_expr(init, env)
    out = []
    for s in body_stmts:
        exec_stmt(s, env, out)
    return "\n".join(out) + ("\n" if out else "")


# ===========================================================================
# Category generators. Each returns (source_text, expected_stdout) or raises
# Overflow to be retried with a fresh spec.
# ===========================================================================


def gen_loop_body(rng, counter_names, const_names):
    """A body that accumulates into MUT `total` (and sometimes `total2`)."""
    stmts = []
    acc = "total"
    choice = rng.randrange(4)
    if choice == 0:  # sum the loop/counter variable
        v = rng.choice(counter_names)
        stmts.append(("assign", acc, ("bin", "+", ("var", acc), ("var", v))))
    elif choice == 1:  # add a LET constant
        c = rng.choice(const_names)
        stmts.append(("assign", acc, ("bin", "+", ("var", acc), ("var", c))))
    elif choice == 2:  # count iterations
        stmts.append(("assign", acc, ("bin", "+", ("var", acc), ("int", 1))))
    else:  # conditional add: IF (v MOD k)=0 THEN total = total + v
        v = rng.choice(counter_names)
        k = rng.randint(2, 5)
        cond = ("cmp", "=", ("bin", "MOD", ("var", v), ("int", k)), ("int", 0))
        stmts.append(("if", cond, ("assign", acc, ("bin", "+", ("var", acc), ("var", v)))))
    return stmts


def gen_for(rng):
    bindings = [("MUT", "total", ("int", 0))]
    const_names = ["total"]  # gen_loop_body always has at least this name
    # Optionally introduce a LET constant used as an addend.
    if rng.random() < 0.6:
        c = rng.randint(1, 50)
        bindings.append(("LET", "c", ("int", c)))
        const_names = ["c"]
    # Loop bounds. Positive step with lo<=hi, or negative step with lo>=hi.
    if rng.random() < 0.25:
        hi = rng.randint(0, 30)
        lo = hi + rng.randint(0, 40)
        step = -rng.randint(1, 4)
    else:
        lo = rng.randint(-10, 20)
        hi = lo + rng.randint(0, 60)
        step = rng.randint(1, 4)

    # Bounds sometimes come from LET bindings (exercises LET + FOR header).
    if rng.random() < 0.5:
        bindings.append(("LET", "lo", ("int", lo)))
        bindings.append(("LET", "hi", ("int", hi)))
        start_e, end_e = ("var", "lo"), ("var", "hi")
    else:
        start_e, end_e = ("int", lo), ("int", hi)

    body = gen_loop_body(rng, ["i"], const_names)

    # Optional inner nested FOR (multiplies the work; exercises nesting).
    if rng.random() < 0.35:
        inner_hi = rng.randint(0, 8)
        inner = gen_loop_body(rng, ["i", "j"], const_names)
        body = body + [("for", "j", ("int", 0), ("int", inner_hi), ("int", 1), inner)]

    stmts = [("for", "i", start_e, end_e, ("int", step), body), ("print", ("var", "total"))]
    src = render_program(bindings, stmts)
    exp = simulate_program(bindings, stmts)
    return src, exp


def gen_counter_loop(rng, kind):
    """kind in {'dowhile','dountil','while'} — counter-driven, guaranteed to
    terminate, with a MUT counter and a MUT accumulator."""
    limit = rng.randint(1, 60)
    inc = rng.randint(1, 4)
    bindings = [("MUT", "n", ("int", 0)), ("MUT", "total", ("int", 0))]
    const_names = []
    if rng.random() < 0.6:
        c = rng.randint(1, 40)
        bindings.append(("LET", "c", ("int", c)))
        const_names.append("c")

    # body: accumulate then advance the counter (advance last so termination
    # is obvious and matches the simulator).
    body = gen_loop_body(rng, ["n"], const_names if const_names else ["total"])
    body = body + [("assign", "n", ("bin", "+", ("var", "n"), ("int", inc)))]

    if kind == "dowhile":
        loop = ("dowhile", ("cmp", "<", ("var", "n"), ("int", limit)), body)
    elif kind == "while":
        loop = ("while", ("cmp", "<", ("var", "n"), ("int", limit)), body)
    else:  # dountil (post-test): runs at least once, exits when n >= limit
        loop = ("dountil", body, ("cmp", ">=", ("var", "n"), ("int", limit)))

    stmts = [loop, ("print", ("var", "total")), ("print", ("var", "n"))]
    src = render_program(bindings, stmts)
    exp = simulate_program(bindings, stmts)
    return src, exp


# ---- recursion -------------------------------------------------------------
# A handful of classic recursive shapes. Args are kept small so results stay in
# i64. The Python reference computes the expected output directly.

RECURSION_TEMPLATES = {
    "sumTo": {
        "src": (
            "FUNC sumTo(n AS Integer) AS Integer\n"
            "  IF n <= 0 THEN RETURN 0\n"
            "  RETURN n + sumTo(n - 1)\n"
            "END FUNC\n"
        ),
        "ref": lambda n: n * (n + 1) // 2 if n > 0 else 0,
        "arg": lambda rng: rng.randint(0, 5000),  # max 5000*5001/2 ~ 1.25e7
    },
    "factorial": {
        "src": (
            "FUNC factorial(n AS Integer) AS Integer\n"
            "  IF n <= 1 THEN RETURN 1\n"
            "  RETURN n * factorial(n - 1)\n"
            "END FUNC\n"
        ),
        "ref": lambda n: _fact(n),
        "arg": lambda rng: rng.randint(0, 20),  # 20! < i64max, 21! overflows
    },
    "fib": {
        "src": (
            "FUNC fib(n AS Integer) AS Integer\n"
            "  IF n < 2 THEN RETURN n\n"
            "  RETURN fib(n - 1) + fib(n - 2)\n"
            "END FUNC\n"
        ),
        "ref": lambda n: _fib(n),
        "arg": lambda rng: rng.randint(0, 27),  # keep call tree cheap to run
    },
    "gcd": {
        "src": (
            "FUNC gcd(a AS Integer, b AS Integer) AS Integer\n"
            "  IF b = 0 THEN RETURN a\n"
            "  RETURN gcd(b, a MOD b)\n"
            "END FUNC\n"
        ),
        "ref": None,  # two-arg; handled specially below
        "arg": None,
    },
    "power": {
        "src": (
            "FUNC power(base AS Integer, exp AS Integer) AS Integer\n"
            "  IF exp <= 0 THEN RETURN 1\n"
            "  RETURN base * power(base, exp - 1)\n"
            "END FUNC\n"
        ),
        "ref": None,  # two-arg
        "arg": None,
    },
}


def _fact(n):
    r = 1
    for k in range(2, n + 1):
        r *= k
    return r


def _fib(n):
    a, b = 0, 1
    for _ in range(n):
        a, b = b, a + b
    return a


def gen_recursion(rng):
    name = rng.choice(list(RECURSION_TEMPLATES))
    t = RECURSION_TEMPLATES[name]
    # Pick 1..4 distinct calls to exercise the same function repeatedly.
    calls = []
    expected = []
    ncalls = rng.randint(1, 4)
    for _ in range(ncalls):
        if name == "gcd":
            a = rng.randint(0, 100000)
            b = rng.randint(0, 100000)
            import math

            val = math.gcd(a, b)
            calls.append(f"  io::print(toString(gcd({a}, {b})))")
            expected.append(str(val))
        elif name == "power":
            base = rng.randint(0, 6)
            exp = rng.randint(0, 22)
            val = base ** exp
            chk(val)  # reject via Overflow if out of range -> retried
            calls.append(f"  io::print(toString(power({base}, {exp})))")
            expected.append(str(val))
        else:
            n = t["arg"](rng)
            val = t["ref"](n)
            chk(val)
            calls.append(f"  io::print(toString({name}({n})))")
            expected.append(str(val))
    src = "IMPORT io\n\n" + t["src"] + "\nFUNC main AS Integer\n" + "\n".join(calls) + "\n  RETURN 0\nEND FUNC\n"
    exp = "\n".join(expected) + "\n"
    return src, exp


# ---- integer arithmetic ----------------------------------------------------
# Expression trees over + - * / MOD DIV with parentheses. `/` (Integer result)
# and MOD divide by a NONZERO literal so non-failing programs never fault; DIV
# is wrapped in toInt(...) so its Float result becomes an exact printable
# Integer. Deliberately-failing programs (overflow, /0, MOD 0) are generated
# separately and checked against the predicted runtime error code.

ARITH_OPS = ["+", "-", "*", "/", "MOD", "DIV"]


def gen_arith_expr(rng, depth, var_names):
    if depth <= 0 or rng.random() < 0.35:
        if var_names and rng.random() < 0.35:
            return ("var", rng.choice(var_names))
        return ("int", rng.randint(-20, 20))
    op = rng.choice(ARITH_OPS)
    if op in ("+", "-", "*"):
        a = gen_arith_expr(rng, depth - 1, var_names)
        b = gen_arith_expr(rng, depth - 1, var_names)
        return ("bin", op, a, b)
    # Division/mod: keep the divisor a nonzero literal so validity is guaranteed.
    a = gen_arith_expr(rng, depth - 1, var_names)
    d = rng.choice([x for x in range(-12, 13) if x != 0])
    den = ("int", d)
    if op == "DIV":
        return ("divwrap", a, den)
    return ("bin", op, a, den)


def gen_arith_int(rng):
    """Integer expression trees: exact value AND typeName (always Integer)."""
    bindings = []
    var_names = []
    for name in ("a", "b"):
        if rng.random() < 0.6:
            bindings.append(("LET", name, ("int", rng.randint(-20, 20))))
            var_names.append(name)
    env = {n: eval_expr(init, {}) for (_k, n, init) in bindings}
    body = [f"{k} {n} = {emit_expr(init)}" for (k, n, init) in bindings]
    expected = []
    for _ in range(rng.randint(3, 7)):
        e = gen_arith_expr(rng, rng.randint(1, 3), var_names)
        val = eval_expr(e, env)  # raises Overflow -> retried by one_program
        text = emit_expr(e)
        body.append(f"io::print(toString({text}))")
        expected.append(str(val))
        body.append(f"io::print(typeName({text}))")
        expected.append(type_of(e))  # "Integer"
    return wrap_program(body), "\n".join(expected) + "\n"


# ---- Fixed / Float value programs (exact) ---------------------------------
# Both toString(Fixed) and the runtime toString(Float) emit exactly 2 decimals,
# so only values exact at 2 dp round-trip. We restrict the Fixed/Float literals
# to quarter multiples (dyadic, hence exactly representable) and the arithmetic
# to +, -, and (Fixed|Float)*Integer — all of which keep the value an exact
# quarter. Fixed*Fixed / Float*Float and any division are left to the
# typeName-only programs below. `kind` is "Fixed" or "Float"; the leaf tag
# ("fix"/"flt") and result type name differ but the value math is identical.

NUM_OVF = 2_000_000_00  # |value| well under the 2^31 Fixed bound, in hundredths
KIND_LEAF = {"Fixed": "fix", "Float": "flt"}


def _num_build(rng, depth, kind):
    """Return (node, type, value). value is an int for Integer, or hundredths
    (a multiple of 25) for the Fixed/Float `kind`. Raises Overflow out of range."""
    leaf = KIND_LEAF[kind]
    if depth <= 0 or rng.random() < 0.45:
        if rng.random() < 0.5:
            n = rng.randint(-9, 9)
            return ("int", n), "Integer", n
        h = rng.randint(-12, 12) * 25  # -3.00 .. 3.00 in quarter steps
        return (leaf, h), kind, h
    op = rng.choice(["+", "-", "*"])
    na, ta, va = _num_build(rng, depth - 1, kind)
    nb, tb, vb = _num_build(rng, depth - 1, kind)
    if op == "*" and ta == kind and tb == kind:
        op = rng.choice(["+", "-"])  # avoid kind*kind (non-exact 2dp toString)
    if op == "*":
        val = va * vb  # kind*Integer keeps hundredths a multiple of 25
        typ = kind if kind in (ta, tb) else "Integer"
    else:
        if kind in (ta, tb):
            ha = va if ta == kind else va * 100
            hb = vb if tb == kind else vb * 100
            val = ha + hb if op == "+" else ha - hb
            typ = kind
        else:
            val = va + vb if op == "+" else va - vb
            typ = "Integer"
    if typ == kind:
        if abs(val) > NUM_OVF:
            raise Overflow("range")
    else:
        chk(val)
    return ("bin", op, na, nb), typ, val


def gen_num_value(rng, kind):
    """One expression guaranteed to be `kind`-typed with an exact 2dp value.
    Returns (node, hundredths)."""
    node, typ, val = _num_build(rng, rng.randint(1, 3), kind)
    # Force a `kind` root by adding a quarter-valued `kind` leaf.
    q = rng.randint(-8, 8) * 25
    root_val = q + (val if typ == kind else val * 100)
    if abs(root_val) > NUM_OVF:
        raise Overflow("root range")
    return ("bin", "+", (KIND_LEAF[kind], q), node), root_val


def gen_arith_fixed(rng):
    """Fixed & mixed Integer/Fixed arithmetic: exact value AND typeName (Fixed)."""
    return _gen_num_program(rng, "Fixed")


def gen_arith_float(rng):
    """Float & mixed Integer/Float arithmetic: exact value AND typeName (Float)."""
    return _gen_num_program(rng, "Float")


def _gen_num_program(rng, kind):
    body = []
    expected = []
    for _ in range(rng.randint(3, 6)):
        expr, hundredths = gen_num_value(rng, kind)  # raises Overflow -> retried
        text = emit_expr(expr)
        body.append(f"io::print(toString({text}))")
        expected.append(format_fixed(hundredths))
        body.append(f"io::print(typeName({text}))")
        expected.append(kind)  # == type_of(expr)
    return wrap_program(body), "\n".join(expected) + "\n"


# ---- Money (dimensional; NOT like the other primitives) -------------------
# Money is base-10 exact (5 decimals) and dimensioned: it combines only with
# another Money under +, -, MOD; scales by a dimensionless k under * and /; and
# a Money/Money or Money DIV ratio EXITS the dimension to Float. Every off-table
# pairing (M+k, k/M, M*M, M vs k, ...) is a COMPILE error, so the generators
# below only ever emit dimensionally-valid expressions. Money has no toString
# overload, so values are rendered via toString(toFixed(m)) — exact for the
# quarter-valued amounts used here. `k` (dimensionless) is an Integer here so
# Money*k and Money/k stay exact.


def _money_build(rng, depth):
    """Return (node, type, value) for an EXACT Money value program. type is
    "Money" (value = hundredths, a multiple of 25) or "Integer" (value = int).
    Only exact, dimensionally-valid ops: M+M, M-M, M*Integer, Integer*M."""
    if depth <= 0 or rng.random() < 0.5:
        if rng.random() < 0.6:
            h = rng.randint(-12, 12) * 25
            return ("mny", h), "Money", h
        n = rng.randint(-6, 6)
        return ("int", n), "Integer", n
    a, ta, va = _money_build(rng, depth - 1)
    b, tb, vb = _money_build(rng, depth - 1)
    am, bm = ta == "Money", tb == "Money"
    if am and bm:
        op = rng.choice(["+", "-"])  # M,M -> Money (exact); avoid / (Float) & MOD
        val = va + vb if op == "+" else va - vb
        typ = "Money"
    elif am != bm:
        op = "*"                     # M*Integer / Integer*M -> Money (exact)
        val = va * vb
        typ = "Money"
    else:
        op = rng.choice(["+", "-", "*"])  # dimensionless Integer arithmetic
        val = va + vb if op == "+" else (va - vb if op == "-" else va * vb)
        typ = "Integer"
    if typ == "Money":
        if abs(val) > NUM_OVF:
            raise Overflow("money range")
    else:
        chk(val)
    return ("bin", op, a, b), typ, val


def gen_money_value(rng):
    """One expression guaranteed to be Money-typed with an exact 2dp value."""
    node, typ, val = _money_build(rng, rng.randint(1, 3))
    q = rng.choice([x for x in range(-8, 9) if x != 0]) * 25
    if typ == "Money":
        root, root_val = ("bin", "+", ("mny", q), node), q + val   # M + M
    else:
        root, root_val = ("bin", "*", ("mny", q), node), q * val   # M * Integer
    if abs(root_val) > NUM_OVF:
        raise Overflow("money root range")
    return root, root_val


def gen_arith_money(rng):
    """Money & Money*Integer arithmetic: exact value (via toFixed) AND typeName."""
    body = []
    expected = []
    for _ in range(rng.randint(3, 6)):
        expr, hundredths = gen_money_value(rng)  # raises Overflow -> retried
        text = emit_expr(expr)
        body.append(f"io::print(toString(toFixed({text})))")
        expected.append(format_fixed(hundredths))
        body.append(f"io::print(typeName({text}))")
        expected.append("Money")
    return wrap_program(body), "\n".join(expected) + "\n"


def _promote_k(ta, tb):
    """Dimensionless numeric promotion: Fixed > Float > Integer."""
    if "Fixed" in (ta, tb):
        return "Fixed"
    if "Float" in (ta, tb):
        return "Float"
    return "Integer"


def _has_money(e):
    if e[0] == "mny":
        return True
    if e[0] == "bin":
        return _has_money(e[2]) or _has_money(e[3])
    return False


def gen_money_typecheck_expr(rng, depth):
    """Build a dimensionally-VALID Money/mixed expression and its result type,
    exercising the full Money algebra table (spec §4.1)."""
    if depth <= 0 or rng.random() < 0.4:
        r = rng.random()
        if r < 0.5:
            return ("mny", rng.randint(-12, 12) * 25), "Money"
        if r < 0.67:
            return ("int", rng.randint(-15, 15)), "Integer"
        if r < 0.84:
            return ("fix", rng.randint(-16, 16) * 25), "Fixed"
        return ("flt", rng.randint(-16, 16) * 25), "Float"
    a, ta = gen_money_typecheck_expr(rng, depth - 1)
    b, tb = gen_money_typecheck_expr(rng, depth - 1)
    am, bm = ta == "Money", tb == "Money"
    if am and bm:
        op, rtyp = rng.choice(
            [("+", "Money"), ("-", "Money"), ("MOD", "Money"), ("/", "Float"), ("DIV", "Float")]
        )
    elif am and not bm:
        op, rtyp = rng.choice([("*", "Money"), ("/", "Money"), ("DIV", "Float")])
    elif bm and not am:
        op, rtyp = ("*", "Money")  # k,M is valid only for *
    else:
        op = rng.choice(["+", "-", "*", "/", "MOD", "DIV"])
        rtyp = "Float" if op == "DIV" else _promote_k(ta, tb)
    return ("bin", op, a, b), rtyp


def gen_arith_money_typecheck(rng):
    body = []
    expected = []
    for _ in range(rng.randint(4, 8)):
        for _try in range(20):
            e, typ = gen_money_typecheck_expr(rng, rng.randint(1, 3))
            if _has_money(e):
                break
        body.append(f"io::print(typeName({emit_expr(e)}))")
        expected.append(typ)
    return wrap_program(body), "\n".join(expected) + "\n"


# ---- typeName-only programs (full promotion coverage) ---------------------
# typeName does NOT evaluate its argument, so these may use ANY operands and
# every operator (including DIV, which yields Float). We verify only the result
# type against the promotion table.


def _typecheck_leaf(rng):
    r = rng.random()
    if r < 0.34:
        return ("int", rng.randint(-15, 15))
    if r < 0.67:
        return ("fix", rng.randint(-16, 16) * 25)
    return ("flt", rng.randint(-16, 16) * 25)


def gen_typecheck_expr(rng, depth):
    # Mixed Integer/Fixed/Float leaves cover every operand pairing, so typeName
    # exercises the full promotion lattice (Fixed > Float > Integer; DIV->Float).
    if depth <= 0 or rng.random() < 0.4:
        return _typecheck_leaf(rng)
    op = rng.choice(["+", "-", "*", "/", "MOD", "DIV"])
    a = gen_typecheck_expr(rng, depth - 1)
    if op in ("/", "MOD", "DIV"):
        # A nonzero literal divisor keeps the program realistic (though typeName
        # would tolerate a zero here since it never evaluates).
        b = _typecheck_leaf(rng)
        while b[1] == 0:  # every leaf tag stores its numeric value at index 1
            b = _typecheck_leaf(rng)
        return ("bin", op, a, b)
    return ("bin", op, a, gen_typecheck_expr(rng, depth - 1))


def gen_arith_typecheck(rng):
    body = []
    expected = []
    for _ in range(rng.randint(4, 8)):
        e = gen_typecheck_expr(rng, rng.randint(1, 3))
        body.append(f"io::print(typeName({emit_expr(e)}))")
        expected.append(type_of(e))  # Integer / Fixed / Float
    return wrap_program(body), "\n".join(expected) + "\n"


def gen_arith_fail(rng):
    """A single deliberately-failing expression, wrapped in a function-level
    TRAP that prints e.code. Covers Integer overflow / divide / MOD by zero, and
    Float NaN / infinity observed at the toString boundary."""
    kind = rng.choice(
        ["mul", "add", "sub", "div0", "mod0", "nan", "inf", "ninf", "fbigmul",
         "movf", "mdiv0", "mmod0"]
    )
    if kind == "mul":
        a = rng.randint(3_100_000_000, 5_000_000_000)
        b = rng.randint(3_100_000_000, 5_000_000_000)  # product > i64max
        line, code = f"io::print(toString(({a} * {b})))", ERR_OVERFLOW
    elif kind == "add":
        line = f"io::print(toString(({I64_MAX} + {rng.randint(1, 1000)})))"
        code = ERR_OVERFLOW
    elif kind == "sub":
        # -(i64max) - b underflows past i64min -> ErrOverflow (Integer has no
        # separate underflow code; that is Byte-only).
        line = f"io::print(toString(({-I64_MAX} - {rng.randint(2, 1000)})))"
        code = ERR_OVERFLOW
    elif kind == "div0":
        line = f"io::print(toString(({rng.randint(-99, 99)} / 0)))"
        code = ERR_INVALID_ARGUMENT
    elif kind == "mod0":
        line = f"io::print(toString(({rng.randint(-99, 99)} MOD 0)))"
        code = ERR_INVALID_ARGUMENT
    elif kind == "nan":  # 0.0 / 0.0 -> NaN, observed by toString
        line, code = "io::print(toString((0f / 0f)))", ERR_FLOAT_NAN
    elif kind == "inf":  # x / 0.0 -> +Inf
        line, code = f"io::print(toString(({rng.randint(1, 99)}f / 0f)))", ERR_FLOAT_OVERFLOW
    elif kind == "ninf":  # x / 0.0 -> -Inf
        line, code = f"io::print(toString((-{rng.randint(1, 99)}f / 0f)))", ERR_FLOAT_OVERFLOW
    elif kind == "movf":  # Money add past the i64 raw range
        line = "io::print(toString(toFixed((90000000000000.00m + 90000000000000.00m))))"
        code = ERR_OVERFLOW
    elif kind == "mdiv0":  # Money / 0 (non-Float result) -> ErrInvalidArgument
        line = f"io::print(toString(toFixed(({rng.randint(1, 99)}.00m / 0))))"
        code = ERR_INVALID_ARGUMENT
    elif kind == "mmod0":  # Money MOD 0.00m (zero divisor)
        line = f"io::print(toString(toFixed(({rng.randint(1, 99)}.00m MOD 0.00m))))"
        code = ERR_INVALID_ARGUMENT
    else:  # fbigmul: runtime overflow to infinity (non-foldable via toFloat)
        return (
            wrap_program(['LET a = toFloat("1e200")', "io::print(toString((a * a)))"], trap=True),
            f"{ERR_FLOAT_OVERFLOW}\n",
        )
    return wrap_program([line], trap=True), f"{code}\n"


def gen_arith(rng):
    r = rng.random()
    if r < 0.14:
        return gen_arith_int(rng)             # Integer value + typeName
    if r < 0.28:
        return gen_arith_fixed(rng)           # Fixed/mixed value + typeName
    if r < 0.42:
        return gen_arith_float(rng)           # Float/mixed value + typeName
    if r < 0.54:
        return gen_arith_money(rng)           # Money value (via toFixed) + typeName
    if r < 0.68:
        return gen_arith_typecheck(rng)       # typeName, all Int/Fixed/Float pairings
    if r < 0.82:
        return gen_arith_money_typecheck(rng)  # typeName, Money dimensional algebra
    return gen_arith_fail(rng)                # overflow / div0 / NaN / Inf / Money traps


CATEGORY_FUNCS = {
    "for": gen_for,
    "doloop": lambda rng: gen_counter_loop(rng, rng.choice(["dowhile", "dountil"])),
    "while": lambda rng: gen_counter_loop(rng, "while"),
    "recursion": gen_recursion,
    "arith": gen_arith,
}


def one_program(category, rng):
    """Generate a single (src, expected), retrying on Overflow."""
    fn = CATEGORY_FUNCS[category]
    for _ in range(200):
        try:
            return fn(rng)
        except Overflow:
            continue
    raise RuntimeError(f"could not generate an in-range {category} program")


# ===========================================================================
# Project scaffolding
# ===========================================================================

PROJECT_JSON = {
    "name": "gen",
    "version": "0.1.0",
    "mfb": "1.0",
    "kind": "executable",
    "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}],
    "entry": "main",
    "targets": ["native"],
}


def write_program(prog_dir, src, expected, meta):
    (prog_dir / "src").mkdir(parents=True, exist_ok=True)
    (prog_dir / "project.json").write_text(json.dumps(PROJECT_JSON, indent=2) + "\n")
    (prog_dir / "src" / "main.mfb").write_text(src)
    (prog_dir / "expected.txt").write_text(expected)
    (prog_dir / "meta.json").write_text(json.dumps(meta) + "\n")


def cmd_gen(args):
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    categories = list(CATEGORY_FUNCS) if args.category == "all" else [args.category]
    rng = random.Random(args.seed)
    width = max(5, len(str(args.count)))
    made = 0
    for i in range(args.count):
        category = categories[i % len(categories)] if args.category == "all" else args.category
        src, expected = one_program(category, rng)
        prog_dir = out / f"p{str(i).zfill(width)}_{category}"
        write_program(prog_dir, src, expected, {"index": i, "category": category, "seed": args.seed})
        made += 1
        if made % 500 == 0:
            print(f"  generated {made}/{args.count}", file=sys.stderr)
    (out / "batch.json").write_text(
        json.dumps({"count": args.count, "category": args.category, "seed": args.seed}, indent=2) + "\n"
    )
    print(f"Generated {made} programs into {out}")


# ===========================================================================
# Runner: build + run + check each program.
# ===========================================================================


def check_one(prog_dir, mfb, timeout):
    prog_dir = Path(prog_dir)
    expected = (prog_dir / "expected.txt").read_text()
    # Build.
    b = subprocess.run(
        [mfb, "build", str(prog_dir)],
        capture_output=True,
        text=True,
    )
    if b.returncode != 0:
        return ("BUILD_FAIL", prog_dir.name, (b.stdout + b.stderr).strip()[:800])
    exe = prog_dir / "gen.out"
    if not exe.exists():
        return ("BUILD_FAIL", prog_dir.name, "no gen.out produced")
    # Run.
    try:
        r = subprocess.run([str(exe)], capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return ("HANG", prog_dir.name, f"timeout after {timeout}s")
    if r.returncode < 0:
        return ("CRASH", prog_dir.name, f"killed by signal {-r.returncode}")
    if r.returncode != 0:
        return ("CRASH", prog_dir.name, f"exit {r.returncode}; stderr={r.stderr.strip()[:300]}")
    if r.stdout != expected:
        detail = f"expected {expected!r}\n     got {r.stdout!r}"
        return ("MISMATCH", prog_dir.name, detail)
    return ("PASS", prog_dir.name, "")


STATUSES = ("PASS", "BUILD_FAIL", "CRASH", "HANG", "MISMATCH")


def _category_of(name):
    """Category is the program-dir-name suffix after the first '_'
    (e.g. 'p00042_arith' -> 'arith'). Category tokens contain no '_'."""
    return name.split("_", 1)[1] if "_" in name else "?"


def _print_category_table(counts):
    """counts: {category: {status: n}}. Print a category x status table."""
    cats = sorted(counts)
    headers = ["category", *STATUSES, "TOTAL"]
    rows = []
    totals = {s: 0 for s in STATUSES}
    for cat in cats:
        row = [cat]
        cat_total = 0
        for s in STATUSES:
            n = counts[cat].get(s, 0)
            totals[s] += n
            cat_total += n
            row.append(str(n))
        row.append(str(cat_total))
        rows.append(row)
    grand = sum(totals.values())
    rows.append(["TOTAL", *[str(totals[s]) for s in STATUSES], str(grand)])

    widths = [len(h) for h in headers]
    for row in rows:
        for i, cell in enumerate(row):
            widths[i] = max(widths[i], len(cell))

    def fmt(cells):
        return "  " + "  ".join(c.ljust(widths[i]) if i == 0 else c.rjust(widths[i]) for i, c in enumerate(cells))

    sep = "  " + "  ".join("-" * w for w in widths)
    print(fmt(headers))
    print(sep)
    for row in rows[:-1]:
        print(fmt(row))
    print(sep)
    print(fmt(rows[-1]))


def cmd_run(args):
    out = Path(args.out)
    progs = sorted(p for p in out.iterdir() if p.is_dir() and (p / "project.json").exists())
    if args.limit:
        progs = progs[: args.limit]
    total = len(progs)
    print(f"Checking {total} programs with {args.jobs} workers (mfb={args.mfb})")
    buckets = {k: [] for k in STATUSES}
    # Per-category counts: {category: {status: n}}.
    cat_counts = {}
    done = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as ex:
        futs = {ex.submit(check_one, p, args.mfb, args.timeout): p for p in progs}
        for fut in concurrent.futures.as_completed(futs):
            status, name, detail = fut.result()
            buckets[status].append((name, detail))
            cat = _category_of(name)
            cat_counts.setdefault(cat, {})[status] = cat_counts.setdefault(cat, {}).get(status, 0) + 1
            done += 1
            if done % 250 == 0 or done == total:
                print(f"  {done}/{total} checked", file=sys.stderr)

    print("\n===== SUMMARY (by category) =====")
    _print_category_table(cat_counts)
    failures = {k: v for k, v in buckets.items() if k != "PASS" and v}
    if failures:
        report = out / "failures.txt"
        with report.open("w") as f:
            for k, items in failures.items():
                for name, detail in items:
                    f.write(f"[{k}] {name}\n{detail}\n\n")
        print(f"\nWrote failing cases to {report}")
        # Echo a few to the console for immediate visibility.
        shown = 0
        for k, items in failures.items():
            for name, detail in items[:3]:
                print(f"\n[{k}] {name}\n{detail}")
                shown += 1
                if shown >= 6:
                    break
            if shown >= 6:
                break
    return 1 if failures else 0


def main():
    ap = argparse.ArgumentParser(description="Generate and test valid MFBASIC programs.")
    sub = ap.add_subparsers(dest="cmd", required=True)

    g = sub.add_parser("gen", help="generate programs")
    g.add_argument("--category", default="for", choices=list(CATEGORY_FUNCS) + ["all"])
    g.add_argument("--count", type=int, default=10000)
    g.add_argument("--out", required=True)
    g.add_argument("--seed", type=int, default=1)
    g.set_defaults(func=cmd_gen)

    r = sub.add_parser("run", help="build + run + check programs")
    r.add_argument("--out", required=True, help="directory a `gen` run produced")
    r.add_argument("--mfb", default="./target/debug/mfb")
    r.add_argument("--jobs", type=int, default=os.cpu_count() or 4)
    r.add_argument("--timeout", type=float, default=15.0)
    r.add_argument("--limit", type=int, default=0)
    r.set_defaults(func=cmd_run)

    args = ap.parse_args()
    rc = args.func(args)
    sys.exit(rc or 0)


if __name__ == "__main__":
    main()
