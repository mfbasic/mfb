# plan-120-F: Correctly-rounded toFloat(String) — the I1/I2/I3 root fix

Last updated: 2026-09-02
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
- ~~**I1**: `toFloat("5e-324")` and `"1e-30"` FAIL outright (spike X03/X04) —
  valid JSON documents are unreadable. Exact failure mechanism UNMEASURED
  (Phase 1 characterizes it).~~ **WITHDRAWN by plan-120-A's Correction C1 —
  measured false.** `toFloat` accepts both; the failure is in
  `json::stringify`'s 25-place search and belongs to plan-120-G. See the
  Corrections section below.
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

  **A first corpus was captured during plan-120-A's execution**, using Node
  v24.12.0's `Number()` (V8's correctly-rounded conversion) as the oracle and
  dumping the raw binary64 bit pattern via `DataView.setFloat64`. These are the
  bits `toFloat` must reproduce exactly; check them in as literals rather than
  computing them at test time.

  | Decimal text | Correct bits | Renders as |
  |---|---|---|
  | `1e-7` | `0x3e7ad7f29abcaf48` | `1e-7` |
  | `1e-30` | `0x39b4484bfeebc2a0` | `1e-30` |
  | `5e-324` | `0x0000000000000001` | `5e-324` |
  | `4.9406564584124654e-324` | `0x0000000000000001` | `5e-324` |
  | `2.4703282292062327e-324` | `0x0000000000000000` | `0` — half-of-denorm rounds DOWN |
  | `2.4703282292062328e-324` | `0x0000000000000001` | `5e-324` — just over half rounds UP |
  | `1e-323` | `0x0000000000000002` | `1e-323` |
  | `1e-322` | `0x0000000000000014` | `1e-322` |
  | `2.2250738585072011e-308` | `0x000fffffffffffff` | largest subnormal (the Java/PHP hang vector) |
  | `2.2250738585072014e-308` | `0x0010000000000000` | `DBL_MIN`, smallest normal |
  | `0.1` | `0x3fb999999999999a` | `0.1` |
  | `0.2` | `0x3fc999999999999a` | `0.2` |
  | `0.3` | `0x3fd3333333333333` | `0.3` |
  | `0.7` | `0x3fe6666666666666` | `0.7` |
  | `0.5` / `0.25` / `0.125` / `0.0625` | `0x3fe0…` / `0x3fd0…` / `0x3fc0…` / `0x3fb0…` | exact dyadics |
  | `1.5` / `2.5` / `1024` | `0x3ff8…` / `0x4004…` / `0x4090…` | exact |
  | `1.0000000000000002` | `0x3ff0000000000001` | 1 + 1 ULP |
  | `1.000000000000000011102230246251565404236316680908203125` | `0x3ff0000000000000` | exact half-ULP, ties-to-even → `1` |
  | `9007199254740992` (2^53) | `0x4340000000000000` | exact |
  | `9007199254740993` (2^53+1) | `0x4340000000000000` | not representable, ties-to-even |
  | `1e21` | `0x444b1ae4d6e2ef50` | `1e+21` |
  | `1e23` | `0x44b52d02c7e14af6` | `1e+23` |
  | `8.98846567431158e307` | `0x7fe0000000000000` | exact power of two |
  | `1.7976931348623157e308` | `0x7fefffffffffffff` | `DBL_MAX` |
  | `123456789012345678901234567890` | `0x45f8ee90ff6c373e` | 30 digits, rounds to `1.2345678901234568e+29` |
  | `3.141592653589793238462643383279` | `0x400921fb54442d18` | π to 31 digits |
  | `-1e-30` | `0xb9b4484bfeebc2a0` | sign is just the top bit |
  | `-0.0` | `0x8000000000000000` | negative zero is preserved by `toFloat` |
  | `0.0` | `0x0000000000000000` | |

  **Three rows are overflow, and must stay ERRORS in MFB, not values**:
  `1e400` and `1e309` give Node `Infinity` (`0x7ff0000000000000`), and
  `1.7976931348623158e308` — which is *past* `DBL_MAX` in decimal — rounds back
  DOWN to `DBL_MAX` (`0x7fefffffffffffff`) rather than overflowing. That last
  one is a genuine trap for the new implementation: a naive "decimal exponent >
  308 ⇒ overflow" range check would wrongly reject it. The boundary is decided
  by the rounded VALUE, not by the exponent.

  Sign note for `-0.0`: Node's `JSON.stringify` renders it `0` (plan-120-C's
  rule) but the *bits* keep the sign, so an F corpus assertion must compare bits
  rather than rendered text, or it will not distinguish `-0.0` from `0.0`.
- plan-118-C (out-of-lining) — NOT a prerequisite; but this letter should
  emit the new parser as one shared `_mfb_rt_string_to_float` runtime
  helper rather than inline per call site (precedent `runtime.mapProbe`),
  both for size and because the fallback path is large.

## Prerequisites

Family gate in plan-120-A, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-120-A landed (codes propagate) | `grep -n "err.code" src/codegen/builtins/json/helper_to_number.rs` | **MET** — re-run 2026-09-02 on `worktree-P-120`: line 21 is `FAIL error(err.code, "JSON number " & value & " is not representable: " & err.message)`, i.e. the TRAP now re-raises `toFloat`'s own code instead of the generic 77050003. |

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
- ~~UNMEASURED (Phase 1): why `1e-30`/`5e-324` error today (suspected an
  exponent-magnitude cap or the overflow check misfiring on subnormals)~~ —
  **ANSWERED: they do not error.** Neither suspicion was right; there is no
  exponent cap and the overflow check is not involved. See Corrections F-C1
  (the measurement) and F-C2 (the emitter read showing neither loop can raise).
- Golden-drift blast radius, **first cut measured** (plan-120-A execution):
  `grep -rl "toFloat(" tests/{rt-behavior,byte-identity,rt-error} | grep src/main.mfb`
  → **16 fixtures**. Split by whether a 1-ULP shift could reach their goldens:

  | Group | Fixtures | Drift risk |
  |---|---|---|
  | `rt-error/general/toFloat_{invalid_format,overflow,exponent_overflow}` | 3 | none — they pin the ERROR, and F keeps both codes |
  | `rt-error/arithmetic/*` (`float-overflow`, `float-nan`, `fma-observed`), `rt-error/operators/unary-float-negation-invalid-format` | 4 | none — same, error-pinning |
  | `rt-error/json/func_json_stringify_invalid_runtime` | 1 | none — pins a stringify failure |
  | `rt-behavior/{operators/unary-numeric-negation,trap/inline-trap-default-able-types,money/money_operations,arithmetic/float-fma-fusion,collections/list-order-invariant,collections/list-payload-order,vector/vector-normalize-inline}` | 7 | **candidate** — each prints or stores a parsed Float |
  | `byte-identity/general` | 1 | `.ncode` only; drifts iff the emitted instruction sequence changes, which it will (inline emitter → `bl`) |

  So ~7 behavior fixtures are the ones to inspect value-by-value against the
  Node/Python oracle, plus `byte-identity/general`'s sums which drift for a
  structural reason rather than a value one. This is a first cut from the call
  site alone; Phase 1 still owes the *actual* diff list from a real run, since a
  fixture only drifts if its particular literals are among the ~1-ULP-wrong set.

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

- [x] ~~Root-cause the `1e-30`/`5e-324` rejections (read the emitter's
      exponent/overflow paths; record the mechanism here).~~ — **moot: there is
      no rejection.** Measured in plan-120-A (Correction C1, reproduced in F-C1
      above): `toFloat("1e-30")` and `toFloat("5e-324")` both return normally.
      The rejection the review saw came from `json::stringify`'s 25-place search
      (`helper_stringify_number.rs:49`), which plan-120-G replaces. Both values
      stay in this letter's corpus as correctness vectors.
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

**F-C1 — the I1 range claim is false; this letter is about ROUNDING only.**
Inherited from plan-120-A's Correction C1, measured there with a probe that
separates `toFloat` from `json::stringify`:

```
1e-30  toFloat: OK      5e-324 toFloat: OK
1e-30  json::parse: OK  1e-30 parse+stringify: FAIL 77050003
```

`toFloat` rejects neither value. The "valid JSON documents are unreadable" class
is a *rendering* failure in `helper_stringify_number.rs:49` and is plan-120-G's
to fix. Scope changes applied to this letter:

- §1 Goal keeps `1e-7` and the half-ULP boundary cases (I3, the real defect) and
  keeps `5e-324`/`1e-30` as **correctness** vectors — the question for them is
  whether the returned double is the correctly-rounded one, not whether the call
  succeeds.
- §2's UNMEASURED item "why `1e-30`/`5e-324` error today" is answered: they do
  not error. Phase 1's root-cause task is discharged by C1's measurement rather
  than by new work.
- The header's framing that the three number defects "share one root" is half
  right: I3 (1-ULP) and I2 (overflow reporting) are `toFloat`'s; I1 is not.
- Nothing about the Eisel–Lemire design, the corpus, or the remote-box proof
  changes — those all serve I3, which is untouched by this correction.

**F-C1b — the I3 defect is REAL and reproduces; this letter's core premise
stands.** Having falsified the I1 range claim above, the rounding claim was
checked too rather than assumed. The native formatter is exact, so a 30-place
rendering distinguishes doubles one ULP apart; expected strings are Node
v24.12.0's `Number(s).toFixed(30)`:

```
WRONG 1e-7
  want 0.000000099999999999999995474811     <- correctly rounded
  got  0.000000100000000000000021944591     <- what toFloat returns
OK    0.1
OK    0.5
OK    1e-30
OK    2.5
OK    3.141592653589793
```

So `toFloat("1e-7")` is off by one ULP, exactly as review I3 reported, and the
json cross-interop corruption that follows from it is genuine. **This letter is
worth doing.**

One nuance that sharpens the scope: the defect is *not* universal. `0.1`
(a repeating binary fraction), `1e-30` (30 divide steps), `2.5`/`0.5` (exact
dyadics) and π to 16 digits all come back correctly rounded. The repeated
multiply/divide loop happens to land on the right double for most inputs and
misses only where the accumulated error crosses a rounding boundary — which is
precisely why the bug survived: it is invisible to spot checks and needs the
pinned corpus in Phase 1 to be caught systematically. It also means the
golden-drift blast radius (measured above) is likely much smaller than the
call-site count suggests.

Aside, for whoever curates the memory note "toFloat is ~1 ULP off even for exact
dyadics": on this build exact dyadics are fine (`0.5`, `2.5` both `OK`). The
wording overstates the defect; `1e-7` is the honest example.

**F-C2 — the emitter read, confirming the header's algorithm claim and pinning
where each verdict comes from** (`builder_conversions.rs:1519-1556`, read during
plan-120-A's execution):

```rust
self.emit(abi::label(&exponent_multiply_loop));
self.emit(abi::compare_immediate(exponent, "0"));
self.emit(abi::branch_eq(&exponent_apply_done));
self.emit(abi::float_multiply_d(FP_SCRATCH[0], FP_SCRATCH[0], FP_SCRATCH[1])); // ×10.0
self.emit(abi::subtract_immediate(exponent, exponent, 1));
```

— and the mirror `exponent_divide_loop` doing `float_divide_d` by the same
`10.0`. So the plan's header is exactly right: the decimal exponent is applied by
`|exp10|` successive binary64 multiplies or divides, each one rounding. `1e-30`
takes 30 roundings, `5e-324` takes 324; that is the I3 mechanism, and it is why
even an exactly-representable dyadic can land 1 ULP off.

Two structural facts this settles, both supporting F-C1:

1. **Neither loop can raise.** They contain no range check — they can only
   saturate to `±Inf` or flush to `0`. There is no path by which
   `emit_parse_decimal_string_to_double` rejects `1e-30` or `5e-324`, which is
   the mechanical reason the "toFloat FAILs" claim was false.
2. **The two verdicts come from different places.** `ErrInvalidFormat` is raised
   at the emitter's `invalid_label` (a grammar fault), while `ErrOverflow` comes
   from the *caller*: `lower_to_float` (`:897-905`) runs
   `emit_double_overflow_check` on the finished double and branches to a separate
   `overflow` label. So `1e400` is diagnosed **after** the loops produce `Inf`,
   not by them. Phase 2 must keep that split when the loops are replaced —
   `_mfb_rt_string_to_float` has to distinguish "not a number" from "out of
   range" itself, since the post-hoc `Inf` test disappears along with the
   saturating loop.

## Summary

The family's highest-value letter: one algorithm swap at the language
primitive closes silent cross-system value corruption and the
valid-JSON-rejected class, with an explicit, measured rejection of the
Fixed-based alternative and a pinned torture corpus as the permanent gate.
