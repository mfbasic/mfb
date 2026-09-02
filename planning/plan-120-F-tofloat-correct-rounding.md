# plan-120-F: Correctly-rounded toFloat(String) — the I1/I2/I3 root fix

Last updated: 2026-09-01
Effort: large (3h–1d)
Depends on: plan-120-A (its error-code propagation defines what json reports when this letter changes what converts)

The review's three number defects share one root: `toFloat(String)`'s parser
(`emit_parse_decimal_string_to_double`,
`src/codegen/engine/convert/builder_conversions.rs:1326`) accumulates digits
in binary64 and applies the exponent by **repeated multiply/divide by 10.0**
(`exponent_multiply_loop` / `exponent_divide_loop`) — the classic
double-rounding algorithm. Measured consequences:

- **I3**: `toFloat("1e-7")` stores an off-by-1-ULP double; json then emits
  digits Node parses to a *different value* (probe N02 `DIFFERENT`). Silent
  cross-system value corruption, and the long-standing memory note
  ("toFloat is not correctly rounded, ~1 ULP off even for exact dyadics").
- **I1**: `toFloat("5e-324")` and `"1e-30"` FAIL outright (spike X03/X04) —
  valid JSON documents are unreadable. Exact failure mechanism UNMEASURED
  (Phase 1 characterizes it).
- **I2**: `toFloat("1e400")` raises rather than saturating (kept — see
  Non-goals — but the code it raises became honest in plan-120-A).

**Recommendation (the user asked for one): fix `toFloat` itself with an
in-tree Eisel–Lemire implementation; do NOT add a Fixed-based mode.**
Reasoning:

- *Fix at the source*: every consumer (`toFloat`, `toFixed(String)`'s float
  path, json, csv, future parsers) inherits correctness; a json-local
  workaround leaves the language primitive wrong.
- *Eisel–Lemire over libc `strtod`*: the runtime's formatter deliberately
  imports no libc ("no libc math or formatting is imported anywhere",
  `float_format.rs:1-5`), and Windows would need a CRT import that the PE
  runtime today avoids entirely. Eisel–Lemire is the modern standard (Rust
  core, Go, .NET), needs one 64×64→128 multiply against a static powers-of-
  ten table, and its rare ambiguous cases can fall back to an exact big-
  decimal comparison built on the SAME limb machinery `float_format.rs`
  already emits. In-tree, no new platform seams.
- *Against a `toFixed`/decimal option* (the user's floated alternative):
  `Fixed` is 32.32 binary fixed-point — integer range ±2³¹, resolution 2⁻³²
  (`emit_float_bits_to_fixed_value`, `builder_conversions.rs:1307-1316`
  multiplies by 2³²). Most JSON numbers (any |x| ≥ 2.1e9, anything needing
  finer than ~2.3e-10) cannot be represented, so a "parse into Fixed" mode
  would fail or silently clamp far more often than today's 1-ULP defect —
  strictly worse. A future *decimal-exact* JSON mode is only worth doing on
  top of a real decimal type; recorded as out of scope, not designed here.

References:

- `builder_conversions.rs:856-912` (`lower_to_float`), `:1326+` (the parser
  emitter), `:922-965` (`lower_to_fixed`'s String arm — same emitter).
- `src/codegen/string/format/float_format.rs` — the limb machinery the
  slow path reuses; also the proof that big static tables + limb loops are
  an established emission style.
- Test-vector sources: the classic strtod torture set (Verified pinned
  vectors checked into the test, not fetched at test time).
- plan-118-C (out-of-lining) — NOT a prerequisite; but this letter should
  emit the new parser as one shared `_mfb_rt_string_to_float` runtime
  helper rather than inline per call site (precedent `runtime.mapProbe`),
  both for size and because the fallback path is large.

## Prerequisites

Family gate in plan-120-A, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-120-A landed (codes propagate) | `grep -n "err.code" src/codegen/builtins/json/helper_to_number.rs` | NOT MET |

## 1. Goal

- `toFloat(s)` returns the correctly-rounded (round-to-nearest-even) binary64
  for every decimal string in the pinned torture corpus — including
  `1e-7`, `5e-324` (min subnormal), `1e-30`, and the classic half-ULP
  boundary cases — and json cross-interop holds: for every corpus value,
  Node parsing MFB's stringify output yields the bit-identical double
  (closing review I3/I1).

### Non-goals (explicit constraints)

- **Overflow still raises** (`ErrOverflow`): MFB deliberately has no
  observable non-finite Float (`observe_float` traps program-wide), so
  `toFloat("1e400")` must not mint an Infinity. The Node divergence
  (Infinity) stays, documented — I2 is closed as *coded + documented*, not
  as Node-parity.
- Underflow to subnormals and to 0 is value-correct, never an error.
- `toFloat`'s accepted GRAMMAR is unchanged (this letter fixes rounding and
  range, not syntax).
- No libc imports; no behavior change for `toFloat(Integer/Money)`.

## 2. Current State

- The emitter is inline per call site (315 instructions for a lone
  `toString(toFloat(x))` fixture measured in the plan-118 research) and
  algorithmically naive per the header above.
- `toFixed(String)` routes through the same emitter (`:958`), so it inherits
  the fix automatically.
- UNMEASURED (Phase 1): why `1e-30`/`5e-324` error today (suspected an
  exponent-magnitude cap or the overflow check misfiring on subnormals);
  the count of `toFloat(String)` call sites in the tree's fixtures whose
  values change by 1 ULP (golden-drift blast radius).

### Verified properties

- The limb machinery needed by the exact fallback exists and is emitted
  today (`float_format.rs` — 35-limb base-2³² divmod loops).
- json will surface this letter's codes untouched (plan-120-A's
  propagation).

## 3. Design Overview

One new runtime helper `_mfb_rt_string_to_float` (emitted when referenced,
like the map helpers) implementing:

1. **Scan** sign/digits/dot/exponent into (mantissa ≤ 19 digits as u64,
   decimal exponent, truncated-bit) — grammar identical to today.
2. **Fast path**: exact cases (mantissa < 2⁵³ and |exp10| ≤ 22) via one
   multiply/divide by an exact power of ten — provably correctly rounded.
3. **Eisel–Lemire**: 128-bit product against the static powers-of-ten table
   (rodata data object, ~[−342, 308] range × 16 bytes); accept when the
   rounding is unambiguous.
4. **Fallback**: exact big-decimal comparison using the limb loops to decide
   the boundary bit (rare inputs only).
5. Overflow → the existing `ErrOverflow` route; underflow → correct
   subnormal/0.

`lower_to_float`/`lower_to_fixed` String arms shrink to marshal + `bl` +
result/ error check (also deleting ~300 inline instructions per call site —
a small plan-118 dividend, not a goal here).

**Risk concentrates in step 4's exactness and in golden drift**: existing
programs' floats can shift by 1 ULP *toward correctness*, which moves
`.run`/build.log goldens wherever a parsed float is printed. Every drifted
golden is inspected (the new value must be the correctly-rounded one — spot
check against Node/Python) before regeneration; a drift in any *other*
direction is a bug in the new parser.

Byte-identity is NOT a gate (the point is changing values by ≤1 ULP to the
correct ones); the gate is the pinned vector corpus + cross-interop.

Rejected: libc strtod (platform principle, Windows CRT); json-local
reimplementation (leaves the primitive wrong); Fixed mode (range analysis
above); arbitrary-precision decimal parse into a new number variant (M5
territory, deferred).

## Phases

### Phase 1 — characterize + corpus (no behavior change)

- [ ] Root-cause the `1e-30`/`5e-324` rejections (read the emitter's
      exponent/overflow paths; record the mechanism here).
- [ ] Land the pinned vector corpus as a RED-capable test: a host-run rt
      fixture asserting `toFloat` bit-exactness (via
      `bits`-level comparison or stringify round-trip once G lands — use
      float bit compare now) over ~100 vectors incl. the torture cases;
      currently-failing vectors marked and counted (the RED baseline).
- [ ] Census fixtures whose goldens will drift (grep float-printing fixtures
      using parsed floats).

Acceptance: corpus test in-tree with the failing set enumerated; no
compiler change yet (`artifact-gate` 0 diffs).
Commit: —

### Phase 2 — the helper

- [ ] Emit `_mfb_rt_string_to_float` (scan + fast path + Eisel–Lemire +
      limb fallback + range routing) with the powers-of-ten table as a data
      object; wire `lower_to_float`/`lower_to_fixed` String arms to call it.
- [ ] Corpus test goes fully green; inspect + regenerate every drifted
      golden per §3's direction rule.
- [ ] Cross-interop proof: rebuild the review's jsoncmp probes; N02-class
      check (Node parses MFB's output back to the bit-identical double) for
      the corpus values.
- [ ] Cross-arch: run the corpus fixture on the remote boxes (x86-64 Linux
      2227/2228, Windows 2230 via a console PE + ssh) — the 128-bit multiply
      path is per-arch codegen and needs runtime proof per the
      emission-is-not-proof lesson.
- [ ] Doc sync: `mfb man general toFloat` (now correctly rounded; overflow
      contract restated); retire the stale memory note wording via the
      repo docs (`.ai/codegen-invariants.md` if it records the defect).

Acceptance: corpus green on macOS + both remote arches; interop check SAME
for all vectors; full `cargo test --no-fail-fast` +
`scripts/test-accept.sh` + regenerated `artifact-gate.sh all`; fmt + check
`--all-targets`.
Commit: —

## Validation Plan

- Tests: the vector corpus (kept permanently); existing toFloat/toFixed
  suites; json acceptance (codes from A unchanged).
- Runtime proof: remote-box corpus runs; the Node interop transcript.
- Doc sync: Phase 2 list.
- Acceptance: family standard.

## Open Decisions

- Helper granularity — one `_mfb_rt_string_to_float` (recommended) vs
  keeping inline emission with the new algorithm (bigger code, no sharing).
  §3 recommends the helper; revisit only if the helper's arg marshalling
  proves awkward for `toFixed`'s in-place consumption.

## Corrections

*(fill during execution)*

## Summary

The family's highest-value letter: one algorithm swap at the language
primitive closes silent cross-system value corruption and the
valid-JSON-rejected class, with an explicit, measured rejection of the
Fixed-based alternative and a pinned torture corpus as the permanent gate.
