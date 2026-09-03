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
| A way to compare two `Float`s bit-exactly from MFBASIC exists | `mfb man bits` / `mfb man encoding` — grep for a float→bits or float→bytes member | **NOT MET as written, SATISFIED by construction — see Correction F-C3.** There is no float-reinterpret anywhere in the language; the affordance is built out of exact decimal rendering plus exact power-of-two scaling instead. |

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

   **Emission precedent located (plan-120-A execution):** `raw_data_object`
   (`src/codegen/memory/data/data_objects.rs:924-940`) builds an arbitrary
   rodata blob from `(symbol, layout, size, hex value, alignment)`, and the
   Unicode runtime tables are the working consumers — e.g. the u32 flattened
   casefold-sequence table at `:915-919`, emitted from
   `unicode::runtime_tables::casefold_sequences_hex()` with 4-byte alignment.
   The powers-of-ten table is the same shape with 16-byte entries. Note this
   is a *different* mechanism from `money`'s `CORDIC_ATAN_TABLE`
   (`gen_fixed_math.rs:1076`), which is a Rust `const [i64; N]` whose values
   are baked into instructions at emission time — that works only for a
   compile-time-known index, and Eisel–Lemire indexes by a runtime decimal
   exponent, so it needs the rodata object rather than baked immediates.
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
- [x] Land the pinned vector corpus as a RED-capable test: a host-run rt
      fixture asserting `toFloat` bit-exactness (~~via `bits`-level
      comparison~~ — no such affordance exists, see Correction F-C3; via exact
      decimal rendering plus exact 2^1000 scaling instead) over ~~~100~~ **27**
      vectors incl. the torture cases; currently-failing vectors marked and
      counted (the RED baseline).

      **LANDED** as `tests/rt-behavior/conversions/tofloat-correct-rounding-corpus-rt`
      with all four goldens (`build.log`, `.ast`, `.ir`, `.run`); scoped
      `test-accept.sh` → `acceptance tests passed (1 test(s) ran)`. The
      `build.log` golden pins the tally line `checked=27 wrong=5`, so Phase 2
      turning it to `wrong=0` is the visible gate.

      Count corrected from the plan's "~100" to **27**: the vectors are chosen
      for distinct failure MODES (exact dyadic, repeating fraction, half-ULP
      tie, 2^53 boundary, power-of-ten, normal/subnormal boundary, underflow
      half-way, signed zero, 30-digit accumulation), not for volume. Padding to
      100 would add rows that exercise nothing the 27 do not; Phase 2 can grow
      it if a new failure mode turns up.

      **Designed and its oracle generated** (the corpus table and expected
      renderings are in References above, produced from the exact
      `man × 2^e2` decomposition with ties-to-even, not from JS `toFixed` —
      F-C3 records the two traps that makes necessary). **Remaining: write the
      fixture files and its four goldens, and record the RED count.** Held
      back from the plan-120-B commit only so the two letters stay separable;
      it lands as F's own commit.

      **RED baseline measured: `checked=27 wrong=5`.** The fixture is written
      and runs (`tests/rt-behavior/conversions/tofloat-correct-rounding-corpus-rt`);
      only its four goldens remain. The five failing vectors are far more
      damning than the single `1e-7` the review reported:

      | Vector | Want | Got | What it means |
      |---|---|---|---|
      | `1e-7` | `…099999999999999995474811` | `…100000000000000021944591` | the known I3 1-ULP defect |
      | `123456789012345678901234567890` | `…677877719597056.00` | `…660285533552640.00` | **many** ULP out, not one — 30 significant digits accumulate error through 30 multiply steps |
      | `2.2250738585072014e-308` (`DBL_MIN`) | `…5625000000…` | `…5626588186…` | the smallest normal is misparsed |
      | `2.2250738585072011e-308` (largest subnormal) | `…5624470604…` | `…5626588186…` | misparsed — **and to the same value as `DBL_MIN` above**, so two distinct doubles collapse into one |
      | `2.4703282292062328e-324` | `…529395592033937712` | `0.000…0` | just-over-half rounds to **zero** instead of up to the min subnormal |

      Two of these are worse than the plan's "~1 ULP" framing. The
      normal/subnormal boundary pair collapsing to a single value means the
      parser cannot distinguish `DBL_MIN` from the largest subnormal at all,
      and the underflow row loses a representable value entirely. Both are in
      exactly the region Eisel–Lemire's fallback path exists to get right.

      Fixture shape, settled: an `rt-behavior/conversions/` fixture whose
      `main` calls two helpers — `check(text, places, expected)` for vectors
      the formatter can reach directly, and `checkTiny(...)` which applies
      `scale1000` (a `WHILE` doing 1000 exact `v = v * 2.0` doublings) before
      rendering. Both print `OK`/`WRONG` per vector and `main` prints a
      `checked=N wrong=M` tally, so the `.run` golden IS the RED baseline
      count and Phase 2 turning it to `wrong=0` is the visible gate. Per the
      "new rt fixture needs FOUR goldens" rule this needs `build.log`, `.ast`,
      `.ir` and `.run`, none of which `sync-goldens.sh` will create — they
      must be added by hand or the fixture silently reports `unexpected
      actual` and only a FULL `test-accept.sh` notices.
- [x] Census fixtures whose goldens will drift (grep float-printing fixtures
      using parsed floats).

      Measured:
      `grep -rl "toFloat(" tests/{rt-behavior,byte-identity,rt-error} | grep src/main.mfb`
      → **16 fixtures**, classified in §2 above into 8 that pin an ERROR (no
      value to drift), 7 behaviour fixtures that print or store a parsed Float
      (the real candidates), and `byte-identity/general` whose `.ncode` drifts
      structurally when the inline emitter becomes a `bl`.

      That is the census this task asks for — a grep over call sites. The
      *actual* drifted-golden list cannot be produced until the new parser
      exists, since a fixture only drifts if its particular literals are among
      the wrong-rounded set; that list is Phase 2's `inspect + regenerate every
      drifted golden` step, and §3's direction rule (every drifted value must
      move TOWARD the correctly-rounded one) is how each is judged.

Acceptance: corpus test in-tree with the failing set enumerated; no
compiler change yet (`artifact-gate` 0 diffs).

**MET.** The corpus fixture is in-tree at
`tests/rt-behavior/conversions/tofloat-correct-rounding-corpus-rt` with all four
goldens, and a scoped `test-accept.sh` reports
`acceptance tests passed (1 test(s) ran)`. The failing set is enumerated by the
fixture's own output, pinned in its `build.log` golden, ending
`checked=27 wrong=5`; the five vectors and what each proves are tabulated in the
Phase 1 task above.

No compiler change: this phase touched only `tests/` and `planning/`.
`scripts/artifact-gate.sh all` → **1828 goldens checked, 0 diffs** (the run
reports 1328 tests rather than the previous 1327 — that is this new fixture
being picked up, and it passes).

One mechanic worth recording for the next person adding an rt fixture: the four
goldens are not independent. The first harness run produced a `build.log`
containing ONLY the `-ast -ir` stage, because with no `.run` marker present the
harness never executes the program. Creating the empty `.run` marker made it
execute, which changed `build.log` — so the `build.log` captured before the
marker existed was immediately stale and had to be re-captured. Create the `.run`
marker FIRST, then capture `build.log`.
Commit: a979053ce

### Phase 2 — the helper

- [x] Emit `_mfb_rt_string_to_float` (scan + ~~fast path~~ + Eisel–Lemire +
      limb fallback + range routing) with the powers-of-ten table as a data
      object; wire `lower_to_float`/`lower_to_fixed` String arms to call it.

      Six symbols in `src/codegen/string/format/float_parse.rs`: the entry
      point, `_mfb_rt_f2s_lemire`, and four bignum primitives (`mul_small`,
      `shl`, `cmp`, and `cmp_scaled`, which cross-multiplies `D * 10^q` against
      a candidate's midpoint `m * 2^e` so the fallback needs no division and has
      no error to bound). The table is `float_parse_table.rs`, built with exact
      big-integer arithmetic rather than pasted, and cross-checked against the
      same construction in Python.

      **The Clinger fast path is deliberately absent — see Correction F-C4.**
      It is the only part of the algorithm needing f64 arithmetic, and a
      hand-written NIR helper may not name a physical FP register; it was
      measured to be a pure optimization first.

      Both String arms are unchanged beyond the emission: the helper still
      leaves its result in `FP_SCRATCH[0]` and still saturates rather than
      reporting overflow, so `toFloat`/`toFixed` raise exactly the codes they
      raised before. A 39-shape grammar probe confirms the accepted and rejected
      sets are identical, including `1e400` still raising and `1e-400` still
      returning zero.
- [x] Corpus test goes fully green; inspect + regenerate every drifted
      golden per §3's direction rule. **`checked=27 wrong=5` → `wrong=0`**, and
      the corpus was then extended to 34 (Correction F-C5).

      Drift was far smaller than §3 anticipated: `test-accept` reported exactly
      **one** mismatch across 1350 tests — the corpus fixture itself — and every
      changed line in its golden is a `WRONG` becoming `OK` against a `want`
      value computed independently from the exact `man * 2^e2` decomposition, so
      the direction rule is satisfied by inspection rather than by assertion. No
      other fixture's output depended on the 1-ULP difference.

      `artifact-gate` showed 10 diffs, all `.ncodesum`, confined to the
      `general` and `json` cover fixtures — the only two that call `toFloat`.
      **No `.ir` moved anywhere**, which is the signature this change should
      have: it is codegen only. Regenerated; gate then reported
      `1832 golden(s) checked, 0 diff(s)`.
- [x] Cross-interop proof: rebuild the review's jsoncmp probes; N02-class
      check (Node parses MFB's output back to the bit-identical double) for
      the corpus values. **`same=26 different=0 render-fail=4`.**

      Each vector is parsed by `toFloat`, rendered by `json::stringify`, and
      that text handed to Node v24.12.0, which must land on the bit-identical
      double it gets from the original literal. The review's headline case is
      closed: `1e-7` now renders `0.0000001` and round-trips exactly, where N02
      reported DIFFERENT.

      The four RENDER-FAILs are `1e-30`, `-1e-30`, `2.2250738585072014e-308` and
      `1e-45` — all below the ~1e-25 floor of `json::stringify`'s fixed-place
      search. That is plan-120-G's defect, exactly as F-C1 established, and the
      probe reports them separately so they can never be mistaken for a value
      disagreement.
- [x] Cross-arch: run the corpus fixture on the remote boxes (x86-64 Linux
      2227/2228, Windows 2230 via a console PE + ssh) — the 128-bit multiply
      path is per-arch codegen and needs runtime proof per the
      emission-is-not-proof lesson. **All green, on more boxes than asked:**

      | box | target | result |
      |---|---|---|
      | — | macos-aarch64 (host) | `checked=34 wrong=0` |
      | 2223 | linux-aarch64 glibc | 1 passed, 0 failed |
      | 2228 | linux-x86_64 glibc | 1 passed, 0 failed |
      | 2227 | linux-x86_64 musl | 1 passed, 0 failed |
      | 2229 | linux-riscv64 musl | 1 passed, 0 failed |
      | 2230 | windows-x86_64 | `checked=34 wrong=0` |

      Linux used the existing `scripts/linux-runtime-proof.sh` with
      `FILTER=tofloat-correct-rounding`; Windows was a cross-built console PE
      shipped by `scp` and run over ssh, since no script covers a plain console
      fixture.

      Windows is the load-bearing one: `umulh` clobbers the register `c_arg(1)`
      maps to there (it *is* `rdx`), so it is the box that would have caught a
      missing vreg copy at entry — and the helper does that copy for exactly
      this reason.
- [x] Doc sync: `mfb man general toFloat` (now correctly rounded; overflow
      contract restated); retire the stale memory note wording via the
      repo docs (~~`.ai/codegen-invariants.md` if it records the defect`~~ —
      moot: grepped, no `.ai/` doc records it).

      `func_to_float.rs` DESC gains the correctly-rounded guarantee, what
      follows from it (exact text converts exactly; a value with enough digits
      survives a trip through another correctly-rounded reader), and the
      asymmetry that too-small converts to zero while too-large raises.

      Added beyond the task: **the embedded spec never stated the conversion's
      rounding contract at all**, and it is now a language-level guarantee a
      reimplementation must reproduce, so `spec language types` §4.1 gains a
      `Float` paragraph with a citation to the emitter.

      Gates: `man-census.sh --memory-scope` 0 unclassified;
      `man-run-examples.sh general --run` 34 built / 34 ran / 0 failed;
      `spec_citations_resolve` and all 26 `docs::` tests green.

Acceptance: corpus green on macOS + both remote arches; interop check SAME
for all vectors; full `cargo test --no-fail-fast` +
`scripts/test-accept.sh` + regenerated `artifact-gate.sh all`; fmt + check
`--all-targets`.
Commit: —

**MET.** Corpus green on macOS and all five remote targets (the table above);
interop check `same=26 different=0`; `artifact-gate` 1832 goldens / 0 diffs;
`cargo test --no-fail-fast` 95 binaries / 4444 passed / 0 failed;
`test-accept.sh` 1350 tests, 0 mismatches.

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

**F-C4 — the Clinger fast path is omitted, and that was measured before it was
decided.** §3 step 2 specifies an exactly-representable fast path (mantissa
< 2^53 and |exp10| <= 22, one f64 multiply or divide). It is not emitted, for a
reason the plan could not have known: it is the only part of the algorithm that
needs f64 arithmetic, and `finalize_vreg_body_with_locals` refuses a
hand-written helper that names a physical register — FP scratch included. The
options were to find a float-vreg convention for helper bodies, or to drop the
shortcut.

Dropping it is only safe if it is an optimization rather than a correctness
crutch, so that was tested rather than assumed:
`lemire_alone_is_sufficient` runs every input both ways and requires identical
bits — 20,000 random doubles rendered and reparsed, plus the exactly-
representable range the shortcut exists to serve (`1e22`, `1e-22`,
`9007199254740992`, `0.5`, `0.1`, …). Zero differences. Eisel–Lemire certifies
those cases itself.

The emitted helper is therefore entirely integer, which also removes any
question about FP register state across the call. The cost is a few extra
instructions on the common path; the benefit is that the whole parser is one
arithmetic domain.

**F-C5 — the corpus did not exercise the exact fallback, so the cross-arch box
would have proved half the helper.** The plan's acceptance is "run the corpus
fixture on the remote boxes". Before doing that it was worth asking what the
corpus actually reaches — and the answer, measured by
`the_original_corpus_never_reached_the_exact_fallback`, is that **every one of
the original 27 vectors is settled by Eisel–Lemire alone**. The big-integer
comparison behind it is a large part of the new code, shares none of Lemire's
arithmetic, and had no coverage at all; a green run on five architectures would
have proved the 128-bit multiply and nothing else.

Seven vectors were added, each the exact decimal midpoint between two adjacent
doubles. A midpoint is dyadic, so its expansion is finite but far longer than
the 19 significant digits the mantissa window keeps — which is precisely the
input the fallback exists for. Each also lands exactly on the tie, making these
the only vectors that exercise round-half-to-even. Two are subnormal-range
midpoints whose literals run past 1000 digits, which additionally drives the
significant-digit cap and the sticky flag that breaks a tie the cap would
otherwise hide.

Expectations come from the `man * 2^e2` decomposition with round-half-to-even —
the construction the rest of the corpus used — and each was cross-checked
against an independent correctly-rounded parser before being committed, so the
fixture does not merely assert that we agree with whatever produced it. Corpus
is now `checked=34 wrong=0` on all six targets.

**F-C6 — a probe reported 168 failures that were not failures.** The generated
557-vector round-trip probe first printed `wrong=168`. Every one was a value
rendered by the probe generator in E-notation and by the formatter under test in
plain decimal — the same number, different notation. Recorded because the
failure mode is a convincing one: a long list of WRONG lines whose `in` and
`out` differ in every character reads exactly like a broken parser, and the
temptation is to start debugging the parser rather than the harness. The fix was
one format specifier; the probe then reported `wrong=0`.


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

**F-C3 — "bit-exactness via `bits`-level comparison" has no affordance; build
one.** Phase 1 says to assert bit-exactness "via `bits`-level comparison". There
is no such thing: `mfb man bits` is integer-only (`band`/`bor`/`sl`/`sra`/…, no
reinterpret), and `mfb man encoding` has no float→bytes member either. Nothing in
the language turns a `Float` into its IEEE bit pattern. Per the skill's rule, a
provability gap is a **missing prerequisite** — added to the table above — and
the continuation is to build the affordance, not to weaken the assertion.

The affordance, in two parts, needing no compiler change:

1. **Exact decimal rendering.** `toString(v, toByte(p))` is documented exact
   (`float_format.rs` header — it is the classical exact fixed-format
   algorithm), so two doubles one ULP apart render differently at enough
   places. This already worked as a discriminator in F-C1b's probe: `1e-7`
   showed `…0000021944591` against the oracle's `…9999995474811` at 30 places.
   The cap is 255 fraction digits (`float_format.rs:38-48`).

2. **Exact power-of-two scaling, for the subnormal tail.** 255 places cannot
   reach `5e-324` (which needs 1074). Multiplying a finite double by 2 is
   EXACT in IEEE — it only increments the exponent — so repeated doubling
   moves a subnormal into the normal range with its mantissa bit-for-bit
   intact, and a 1-ULP difference survives the scaling unchanged. Scale by
   2^1000 (≈1.07e301, comfortably in range; `5e-324 × 2^1000 ≈ 5.35e-23`)
   and then render. Build the scale factor by repeated doubling in MFBASIC
   rather than with `^`, so the fixture does not depend on `pow` returning an
   exactly-representable power of two.

Verified before committing to the approach (scaling the min subnormal and the
value two ULP above it by 2^1000, then rendering):

```
5e-324 * 2^1000 = 0.0000000000000000000000529395592033937712
1e-323 * 2^1000 = 0.0000000000000000000001058791184067875424
distinguishable: true    finite: true
```

Together these give a total, exact discriminator over the whole corpus without
adding language surface.

**Two traps found while generating the expected strings — do not produce the
oracle with JavaScript's `toFixed`:**

1. **`Number.prototype.toFixed` is not fixed-point for `|x| ≥ 1e21`** — it falls
   back to exponential, so it emitted `1e+21`, `1e+23`, `1.2345678901234568e+29`
   and `1.7976931348623157e+308` where MFB's exact formatter prints every digit.
   The large-magnitude vectors' expectations must come from an exact BigInt
   expansion of the double (the same `man × 2^e2` decomposition the corpus-bits
   table above was built from), not from `toFixed`.
2. **`-0.0` renders differently on the two sides.** Node's `(-0).toFixed(4)` is
   `"0.0000"`; MFB's formatter deliberately keeps the sign (`float_format.rs`
   header: "`-0.0` renders with the sign"), so the expectation is `-0.0000`.
   Take the sign from the bit pattern rather than from the rendered oracle.

Neither is a defect in MFB — both are JS output quirks that would silently bake
a wrong expectation into the fixture. Flagged here because the corpus is meant
to be the permanent gate, and a wrong expectation in it is worse than no gate.

**The corrected oracle**, generated from the exact `man × 2^e2` decomposition
with round-half-to-even and MFB's sign convention (no `toFixed` anywhere). These
are the strings the Phase 1 fixture asserts — `S` marks a vector scaled by
2^1000 first:

| Vector | places | | Expected `toString(toFloat(v), places)` |
|---|---|---|---|
| `1e-7` | 30 | | `0.000000099999999999999995474811` |
| `0.1` | 30 | | `0.100000000000000005551115123126` |
| `0.2` | 30 | | `0.200000000000000011102230246252` |
| `0.3` | 30 | | `0.299999999999999988897769753748` |
| `0.7` | 30 | | `0.699999999999999955591079014994` |
| `0.5` | 20 | | `0.50000000000000000000` |
| `2.5` | 20 | | `2.50000000000000000000` |
| `1024` | 6 | | `1024.000000` |
| `1.0000000000000002` | 20 | | `1.00000000000000022204` |
| `1.000000000000000011102230246251565404236316680908203125` | 20 | | `1.00000000000000000000` (exact half-ULP → ties-to-even → 1) |
| `3.141592653589793238462643383279` | 30 | | `3.141592653589793115997963468544` |
| `9007199254740992` | 4 | | `9007199254740992.0000` |
| `9007199254740993` | 4 | | `9007199254740992.0000` (2^53+1 is not representable) |
| `1e21` | 4 | | `1000000000000000000000.0000` |
| `1e23` | 4 | | `99999999999999991611392.0000` |
| `123456789012345678901234567890` | 2 | | `123456789012345677877719597056.00` |
| `1e-30` | 45 | | `0.000000000000000000000000000001000000000000000` |
| `1e-21` | 36 | | `0.000000000000000000001000000000000000` |
| `2.2250738585072014e-308` | 40 | S | `0.0000002384185791015625000000000000000000` |
| `2.2250738585072011e-308` | 40 | S | `0.0000002384185791015624470604407966062288` |
| `5e-324` | 40 | S | `0.0000000000000000000000529395592033937712` |
| `2.4703282292062327e-324` | 40 | S | `0.0000000000000000000000000000000000000000` (rounds to 0) |
| `2.4703282292062328e-324` | 40 | S | `0.0000000000000000000000529395592033937712` |
| `1e-323` | 40 | S | `0.0000000000000000000001058791184067875424` |
| `-1e-30` | 45 | | `-0.000000000000000000000000000001000000000000000` |
| `-0.0` | 4 | | `-0.0000` (MFB keeps the sign) |
| `0.0` | 4 | | `0.0000` |
| `8.98846567431158e307` | 0 | | a 308-digit integer |
| `1.7976931348623157e308` | 0 | | a 309-digit integer |

Two of these are worth keeping for their own sake even after F lands: `1e23`
shows the nearest double is `99999999999999991611392`, not `1e23`, and
`9007199254740993` shows 2^53+1 collapsing onto 2^53 — both are exactly the
kind of value a "looks right" spot check waves through. The corpus table above therefore records the correct
BITS (for provenance and for whoever implements the Eisel–Lemire path) while the
fixture asserts the corresponding exact decimal renderings.

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
