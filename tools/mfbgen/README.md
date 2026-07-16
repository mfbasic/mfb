# mfbgen — valid-program generator for runtime testing

Generates large batches of **valid, compiling** MFBASIC programs that exercise
one language feature at a time (FOR loops, DO/LOOP, WHILE, recursion) in every
combination of LET/MUT bindings, bounds, steps, and bodies — so you can hammer
the **runtime / compiled code** without hand-writing any programs.

It is *not* a parser fuzzer. Every program compiles and runs to completion by
construction.

## How it knows the right answer (the oracle)

Each program is derived from a random **spec** (a small data structure). Two
pure functions consume that spec: one emits the MFBASIC source, the other
interprets the spec in Python to compute the exact stdout the program must
produce. Same spec → same behavior, so the generator is its own oracle. A
generated program is a **runtime bug** iff, after it compiles, its real stdout
differs from the prediction (or it crashes / hangs).

All arithmetic is kept inside i64 by construction: the simulator rejects any
spec that would overflow (which would otherwise raise a checked `ErrOverflow`
at runtime) and retries. Programs are deterministic — no floats, no
map-iteration-order dependence — so the comparison is exact.

## Usage

```sh
# Generate 10k FOR-loop programs
python3 tools/mfbgen/mfbgen.py gen --category for --count 10000 --out /tmp/for --seed 1

# Build + run + check them all; buckets failures, writes /tmp/for/failures.txt
python3 tools/mfbgen/mfbgen.py run --out /tmp/for --mfb ./target/debug/mfb --jobs 8
```

`--category`: `for`, `doloop`, `while`, `recursion`, `arith`, or `all`.
`gen` is deterministic given `--seed`, so any failure reproduces exactly.

### Categories

- **for / doloop / while** — counted and counter-driven loops accumulating into
  MUT bindings, with LET constants, varied bounds/steps, and nesting.
- **recursion** — classic recursive FUNCs (sumTo, factorial, fib, gcd, power)
  with a Python reference for the expected value.
- **arith** — expression trees over `+ - * / MOD DIV` with parentheses, across
  Integer, Fixed, Float, Money, and every valid mixed pairing. Flavors:
  - *Integer value* — exact Integer result **and** `typeName` (always Integer).
    `/` (Integer result) and `MOD` truncate toward zero / take the dividend's
    sign; `DIV` returns Float so it is wrapped as `toInt(a DIV b)` to stay an
    exact printable Integer.
  - *Fixed value* — Fixed and mixed Integer/Fixed arithmetic with exact value
    **and** `typeName` (Fixed).
  - *Float value* — Float and mixed Integer/Float arithmetic with exact value
    **and** `typeName` (Float).
  - *Money value* — Money and Money×Integer arithmetic with exact value **and**
    `typeName` (Money). Money has no `toString` overload, so values are rendered
    via `toString(toFixed(m))` (exact for the quarter-valued amounts used).
  - *typeName-only (Int/Fixed/Float)* — arbitrary trees with every operator
    including `DIV`, verifying the promoted result type against the promotion
    lattice (`Fixed > Float > Integer`; so `Float op Fixed → Fixed`, `Float op
    Integer → Float`, `DIV → Float`). `typeName` does not evaluate its argument,
    so these need no range or divisor guards.
  - *typeName-only (Money)* — dimensionally-valid Money expressions verifying
    the Money algebra: `M±M → M`, `M MOD M → M`, `M*k`/`k*M → M`, `M/k → M`,
    `M/M`/`M DIV _ → Float`. The generator never emits an off-table pairing
    (`M+k`, `k/M`, `M*M`, `M` vs `k`, …) because those are *compile* errors.
  - *failing* — deliberately trigger a checked failure, wrapped in a
    function-level `TRAP` that prints `e.code`, checked against the predicted
    runtime code:
    - Integer overflow (`*`, `+`, `-`) → 77050010
    - non-Float divide / `MOD` by zero → 77050002
    - Float `0.0/0.0` → NaN observed at the toString boundary → 77050013
    - Float `x/0.0` and overflow-to-infinity (`1e200 * 1e200`) → 77050015
    - Money overflow → 77050010; Money `/0` and `MOD 0m` → 77050002

  `typeName` verifies the compiler's numeric type promotion (and Money's
  dimensional algebra); the value checks verify the runtime arithmetic itself;
  the failing programs verify the checked-arithmetic and Float-finiteness error
  paths.

  **Exact-value caveat (Fixed / Float / Money).** `toString(Fixed)` and
  `toString(Float)` take a `precision` param that **defaults to 2**, so runtime
  output is 2 decimals (this is documented, not a bug — though note that
  *compile-time constant folding* prints shortest round-trip instead, e.g.
  `toString(1f)` → `"1"` while a runtime `toFloat("1")` → `"1.00"`; that
  inconsistency may be worth filing). Money renders through `toFixed`, also
  2 dp. Because of the 2-dp output, the value-checked Fixed/Float/Money programs
  use only quarter-valued (dyadic, exactly representable) literals under `+`,
  `-`, and ×Integer — all exact at 2 dp with no rounding. `X×X` (same-kind
  multiply) and any division are covered by the type-only programs, not
  value-checked.

Each program lives in its own project dir (`p<NNN>_<category>/`) with
`project.json`, `src/main.mfb`, `expected.txt`, and `meta.json`. `run`
classifies each as PASS / BUILD_FAIL / CRASH (nonzero exit or signal) / HANG
(timeout) / MISMATCH (compiled+ran but wrong output), and writes every failing
case to `failures.txt`.

## Adding a category / more surface

Categories are small generator functions in `mfbgen.py` returning
`(source, expected_stdout)`. The shared statement/expression model
(`emit_stmt` + `exec_stmt`, kept in lockstep) already covers assignments,
inline IF, FOR, DO WHILE/LOOP, DO/LOOP UNTIL, and WHILE, so new loop shapes are
cheap. Recursion uses a table of classic templates with a Python reference.

To grow coverage next: nested mixed loops, EXIT/CONTINUE, more binding shapes,
then String/List/Map bodies and MATCH (add the corresponding nodes to the model
and their interpreter cases so the oracle stays exact).

## Scope note

The generator currently produces the well-understood, unambiguous cases (e.g.
positive STEP with `lo <= hi`, counter-driven loops that provably terminate).
This is deliberate: the oracle must match the runtime exactly, so edge-case
semantics are added only after they're confirmed against the real compiler —
otherwise a generator assumption shows up as a false MISMATCH. Expand the ranges
in each `gen_*` function to probe edgier territory once you've validated it.
