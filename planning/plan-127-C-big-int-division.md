# plan-127-C: `big::Int` division and the members built on it

Last updated: 2026-09-06
Effort: large (3h–1d)
Depends on: plan-127-B

Full multi-byte division, and the six members that need it. This is the highest-risk
letter in the feature and it is deliberately last: every algorithm here is one where a
subtle error produces a plausible wrong answer rather than a failure.

Behavioral outcome: `big::divMod(a, b)` returns a quotient and remainder satisfying
`a = b × quotient + remainder` with `|remainder| < |b|`, for every `a` and every
non-zero `b`, at any magnitude.

References — read these first:

- plan-127-A §4.2 (pinned runtime layout), §4.3 (shared emitters).
- plan-127-B §4.1 (the magnitude emitters this letter composes).
- Knuth, *TAOCP* Vol. 2 §4.3.1 Algorithm D — the normalization step and the
  quotient-digit correction are the two places implementations go wrong.
- `.ai/codegen-invariants.md` — register lifetimes and vreg allocation order.

## Prerequisites

Stated once in plan-127-A and unchanged. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-127-B is complete: Phases 1–4 ticked and their commits recorded | `grep -c '^- \[ \]' planning/completed/plan-127-B-big-int-arithmetic.md` → `0` | MET (2026-09-13, worktree-P-127 @ 92cff30cc: `0`; plan archived to `planning/completed/`) |
| The magnitude emitters exist | `grep -c "fn emit_add_magnitude\|fn emit_sub_magnitude\|fn emit_mul_magnitude" src/codegen/builtins/big/gen_big.rs` → `3` | MET (2026-09-13, @ 92cff30cc: `3`) |

If plan-127-B is not complete, this letter cannot start, full stop. Algorithm D is
built from magnitude compare, subtract and multiply; there is no partial mode.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. **If you stop, report the current status of *all* prerequisites.**

## 1. Goal

- For every `a` and every non-zero `b`: `big::divMod(a, b)` returns `{quotient,
  remainder}` with `a = big::add(big::multiply(b, quotient), remainder)` and
  `big::compare(big::abs(remainder), big::abs(b)) < 0`.
- `big::divide` and `big::remainder` agree with the corresponding field of
  `big::divMod` for the same operands.
- `big::gcd(a, b)` is non-negative and divides both operands exactly.
- Division by zero raises `ErrInvalidArgument` on all three division members and
  nothing else fails.

### Non-goals

Every non-goal in plan-127-A §1 applies unchanged. Additionally:

- **No `modInverse`, no `isProbablePrime`.** Both are key-generation shaped with no
  non-cryptographic customer in this feature, and shipping them invites exactly the
  misuse §4.5 warns against. Not shipped, not stubbed.
- **No constant-time guarantee anywhere.** `big::modPow` in particular is data-dependent
  in both time and memory access. §4.5 makes this a documented, advisory-carrying
  property.
- **No sub-quadratic division.** Algorithm D only.

## 2. Current State

`big::Int` supports addition, subtraction, multiplication, aggregates, bit operations
and base-10 text (plan-127-B). `emit_div_small` exists but divides only by a
single-byte divisor and serves `toString` alone — it cannot divide by another
`big::Int`. There is no way to compute a quotient, a remainder, a GCD, or a power.

### Measured populations

| What | Count | Command |
|---|---|---|
| Members this letter adds | 7 | §4.4 |
| Records this letter adds | 1 (`DivResult`) | §4.3 |
| Members existing after plan-127-B | 22 | plan-127-A §4.4 (10) + plan-127-B §4.3 (12) — was 23, plan-127-A Corrections C5 |

### Verified properties

- **`emit_div_small` does not generalize.** VERIFIED by reading plan-127-B §4.1: it
  walks from the high byte carrying a remainder that must stay below the divisor, which
  only holds when the divisor fits in one byte. Multi-byte division needs quotient-digit
  estimation and correction — a different algorithm, not a loop bound change.
- **UNVERIFIED — the sign convention of MFBASIC's `MOD` for negative operands.**
  Carried forward from plan-127-A. Phase 1 measures it; §4.2's convention must match
  whatever it reports, and the measurement is recorded in Corrections.

## 3. Design Overview

One emitter, then seven members.

**Where correctness risk concentrates: here, and specifically in Algorithm D's quotient
digit estimation.** The estimate is correct or one too large, and the correction step is
the part implementations omit — it fires on a small fraction of inputs, so a missing
correction passes casual testing and fails on particular operand pairs. This is the
reason the letter is last and the reason Phase 1's acceptance is a property test over a
wide random spread rather than a fixed vector list.

**Where design uncertainty concentrates: the sign conventions**, which are a choice, not
a derivation, and which must agree with the language's own `MOD`. Phase 1 measures
before specifying.

**Byte-identity is not this letter's gate** — same narrow non-disturbance role as
plan-127-A §3.

### Rejected alternatives

- **Binary long division (shift-and-subtract, one bit at a time).** Simpler and much
  easier to get right, but O(bits × limbs) rather than O(limbs²) — for a 4096-bit
  divisor that is 4096 subtract passes instead of 512 estimated digits. Rejected on
  cost, having considered it precisely because the risk here is correctness. If
  Algorithm D proves unstable in Phase 1, landing binary long division and recording the
  measured slowdown is a better outcome than an unreliable Algorithm D — but it is a
  decision to record in Corrections, not a silent fallback.
- **Deriving `remainder` as `subtract(a, multiply(b, divide(a, b)))`.** Rejected: it
  runs the division twice for callers who want both, which is what `divMod` exists to
  avoid, and it compounds any error in `divide` instead of surfacing it.

## 4. Detailed Design

### 4.1 `emit_div_mod_magnitude` (`gen_big.rs`)

Knuth Algorithm D over byte limbs. Three parts, all of which must be present:

1. **Normalize.** Left-shift both operands so the divisor's high byte is ≥ 128. This is
   what bounds the quotient-digit estimate error to at most 1. Skipping it does not make
   the algorithm wrong on most inputs, which is precisely why it gets skipped.
2. **Estimate and correct.** For each quotient digit, estimate from the top two bytes of
   the running remainder over the divisor's high byte, then **correct downward while the
   trial product exceeds the remainder**. The correction loop runs at most twice; it is
   not optional.
3. **Denormalize.** Right-shift the remainder by the same amount. The quotient is
   unaffected.

Returns `(quotientData, quotientCount, remainderData, remainderCount)`; both results go
through plan-127-A's `emit_build_int`, so normalization of the *record* stays
single-sited and separate from Algorithm D's operand normalization. The two senses of
"normalize" are easy to conflate — the code comment must distinguish them.

### 4.2 Sign conventions

Measured, not assumed (Phase 1). The intended convention, to be confirmed against the
language's `MOD`:

- **Quotient truncates toward zero.** `divide(-7, 2)` is `-3`, not `-4`.
- **Remainder takes the sign of the dividend.** `remainder(-7, 2)` is `-1`;
  `remainder(7, -2)` is `1`.
- The identity `a = b × quotient + remainder` holds under both clauses together.

If Phase 1 finds `MOD` uses floored semantics instead, §4.2 changes to match and the
change is recorded in Corrections — matching the language beats matching this
document.

### 4.3 `DivResult`

```basic
EXPORT TYPE DivResult
  quotient  AS big::Int
  remainder AS big::Int
END TYPE
```

Field order is contract, as for `Int` (plan-127-A §4.1); the `add_record` call carries
the comment. Both fields are flat composites, so the whole `DivResult` inlines
recursively into one block (`src/docs/spec/memory/03_heap-values.md:63-66`).

### 4.4 Members (7)

```basic
big::divide(a AS big::Int, b AS big::Int) AS big::Int              ' ErrInvalidArgument (b = 0)
big::remainder(a AS big::Int, b AS big::Int) AS big::Int           ' ErrInvalidArgument (b = 0)
big::divMod(a AS big::Int, b AS big::Int) AS big::DivResult        ' ErrInvalidArgument (b = 0)
big::pow(base AS big::Int, exponent AS Integer) AS big::Int        ' ErrInvalidArgument (exponent < 0)
big::gcd(a AS big::Int, b AS big::Int) AS big::Int                 ' total
big::factorial(n AS Integer) AS big::Int                           ' ErrInvalidArgument (n < 0)
big::modPow(base AS big::Int, exponent AS big::Int,
            modulus AS big::Int) AS big::Int                       ' ErrInvalidArgument
```

`divMod` earns its place: quotient and remainder come from one division, so computing
both separately does the work twice.

`pow` takes an `Integer` exponent — a genuinely big exponent produces a result no
machine can hold. `modPow` takes a `big::Int` exponent, because that is the case where
the result stays bounded.

`gcd` is total: it is defined for every pair including `(0, 0)`, where it returns `0`.
It returns a non-negative result regardless of operand signs. Both facts go in the
descriptor, because the `(0, 0)` case and the sign rule are where implementations
differ.

`factorial` takes an `Integer` and raises `ErrInvalidArgument` on a negative. It exists
for the same reason `sum` and `product` do: one native call instead of N.

### 4.5 `modPow` carries a mandatory advisory

`big::modPow` is not constant-time. Its running time and memory access pattern depend on
the exponent's bits, so using it with a secret exponent leaks that exponent through
timing. Its `desc` must state this explicitly and direct the reader to `crypto::` for
anything cryptographic — `crypto` implements its curve and scalar arithmetic with
constant-time primitives for exactly this reason.

This is not a caveat to soften. It is the difference between a useful arbitrary-precision
utility and a footgun that looks like a crypto primitive. The same advisory applies to
`big::compare` and `big::equals`, which short-circuit; their descriptors point at
`crypto::constantTimeEqual` for byte-wise comparison that does not.

## Compatibility / Format Impact

- **Added:** seven members and one exported record (`big::DivResult`) under `big::`.
- **Unchanged:** `big::Int`'s layout, canonical form and field order; every other
  package's surface; every file under `packages/`.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partial and say what remains. Fill each
> `Commit:` line the moment it lands. **An unticked box means NOT DONE.**

### Phase 1 — measure `MOD`, then Algorithm D

The highest-risk work, gated on a property test before anything depends on it.

- [x] Measure MFBASIC's `MOD` sign convention: run a program printing `-7 MOD 2`,
      `7 MOD -2`, `-7 MOD -2`, and the matching `/` quotients. Record the results and
      the command in Corrections, and reconcile §4.2 against them. (C-C1: truncating;
      §4.2 unchanged.)
- [x] `gen_big.rs`: `emit_div_mod_magnitude` per §4.1, with all three parts and a
      comment distinguishing Algorithm D's operand normalization from
      `emit_build_int`'s record normalization. (`grep -n 'fn emit_div_mod_magnitude'` → line
      1371; the section comment "Two different "normalize"s meet in this section" names D1
      operand normalization and `emit_build_int` record normalization; `cargo build --release
      -p mfb --all-targets` → no warnings or errors.)
- [x] Property test: for 10,000 random `(a, b)` pairs with `b ≠ 0`, spanning 1 to 512
      bytes of magnitude and all four sign quadrants, assert
      `a = b × quotient + remainder` and `|remainder| < |b|`. Seed fixed and recorded so
      a failure reproduces. (`division_property_over_ten_thousand_seeded_pairs`,
      `math::seed(127)`, also checks both signs; asserts `pairs 10000`, every quadrant >1000,
      shorter divisors in 1000..9000, `identity 0`, `bound 0`, `sign 0` → ok. C-C3.)
- [x] Targeted tests for the cases the correction step exists for: divisor high byte
      just below and just above 128 (normalization boundary); a dividend whose leading
      bytes force the estimate one too large; divisor longer than dividend (quotient 0,
      remainder = dividend); divisor equal to dividend. (`division_targeted_cases` → D3
      correction `53884 27863`, D6 add-back `349 8454523`, top byte 127 `8590196744 7`, top
      byte 128 `8589934591 32767`, longer divisor `0 -12345`, equal `1 0`, negation `-1 0`,
      one-byte divisor `40210710958665 0` — all Python `divmod`; C-C4.) `cargo test --release
      --test rt_big_int division` → `3 passed; 0 failed`.

Acceptance: the 10,000-pair property test passes with a recorded seed, **and** each
targeted case passes. A failure here is root-caused in Algorithm D, not worked around
by weakening the property.
Commit: 5ff478ffd

### Phase 2 — the division members

- [x] `mod.rs`: `add_record` for `DivResult` with the field-order comment. ("Field ORDER is
      contract: `emit_build_div_result` builds this record…", `mod.rs` `DIV_RESULT_TYPE`.)
- [x] `func_divide.rs`, `func_remainder.rs`, `func_div_mod.rs` — all three declare
      `ErrInvalidArgument` for a zero divisor and nothing else. (Each `errors:
      vec!["ErrInvalidArgument"]`; `cargo test --release -p mfb --bin mfb
      codegen::builtins::big` → `8 passed; 0 failed`, including the exact-errors table and
      every-backend lowering with a `divMod` call.)
- [x] Tests: each of the three raises `ErrInvalidArgument` on `b = 0`; `divide` and
      `remainder` agree with the matching `divMod` field across the Phase 1 spread; the
      §4.2 sign convention holds on the four `(±7, ±2)` cases.
      (`division_members_agree_and_raise` → `agreement: 0 of 1000` over 500 seeded
      (`math::seed(128)`) pairs of 1–64 bytes in random sign quadrants, `3 1 | -3 -1 | -3 1 |
      3 -1`, `raised 77050002 | raised 77050002 | raised 77050002` → ok. C-C6.)
- [x] Admit the three division members in all three backend `runtime_calls` lists (plan-127-A C2).
      (`grep -c '"big\.'` → `25` in each backend list.)

Acceptance: all three declare exactly `["ErrInvalidArgument"]` in their registry `errors`
vector (plan-127-A Corrections C1); the agreement and sign tests pass.
Commit: 5ff478ffd (shared with Phase 1, C-C2)

### Phase 3 — the members built on division

- [x] `func_pow.rs`, `func_gcd.rs`, `func_factorial.rs`, `func_mod_pow.rs`. (With
      `emit_int_from_integer`; `cargo build --release -p mfb --all-targets` → no warnings or
      errors; `cargo test --release -p mfb --bin mfb codegen::builtins::big` → `8 passed; 0
      failed`.)
- [x] `modPow`, `pow` and `factorial` each declare `ErrInvalidArgument`; `gcd` declares
      nothing. (`errors:` `vec!["ErrInvalidArgument"]` ×3, `func_gcd.rs` `vec![]`.)
- [x] `modPow`'s, `compare`'s and `equals`'s descriptors carry the §4.5 advisory.
      Amending `compare`/`equals` (landed in plan-127-A) is in scope for this phase.
      (`grep -i constant-time` → `func_mod_pow.rs:31`, `func_compare.rs:26`,
      `func_equals.rs:27`; compare/equals point at `crypto::constantTimeEqual`;
      `mfb man big modPow` renders "Not constant-time — never use it with a secret".)
- [x] Tests: `pow(x, 0)` is `1` and `pow(x, 1)` is `x`; `pow` agrees with folded
      `multiply` for small exponents; `gcd` is non-negative, divides both operands, and
      `gcd(0, 0)` is `0`; `factorial(0)` is `1`, `factorial(20)` matches the exact
      `Integer` value, `factorial(100)` matches an independently computed constant;
      `modPow(b, e, m)` agrees with `remainder(pow(b, e), m)` for small `e`;
      `modPow` with `modulus = 0` raises `ErrInvalidArgument`;
      negative `exponent`/`n` each raise `ErrInvalidArgument`.
      (`powers_gcd_factorial_and_mod_pow_match_an_independent_oracle`, all literals from
      Python — spot-rechecked with `python3 -c "math.factorial(100) …"`: factorial(100),
      `2432902008176640000`, `6 7 0`, `965115194`, `2^100`, `(-3)^41` identical — prints
      `1`, `1`, factorial(20), factorial(100), `pow(7,0)=1`, `pow(0,0)=1`, `pow(-13,1)=-13`,
      gcd cases including the shared-factor pair and consecutive Fibonacci numbers (`1`),
      `modPow vs remainder(pow): 0 of 390`, `pow vs folded multiply: 0 of 41`, `raised
      77050002` ×4 for `pow(2,-1)`, `factorial(-1)`, `modPow(3,-1,7)`, `modPow(3,2,0)`;
      `cargo test --release --test rt_big_int powers_gcd_factorial` → `1 passed; 0 failed`.
      C-C7.)
- [x] Admit the four members in all three backend `runtime_calls` lists (plan-127-A C2).
      (`grep -c '"big\.'` → `29` in each backend list.)
- [x] Doc (added): `scripts/man-run-examples.sh big --run divide remainder divMod pow gcd
      factorial modPow` → `examples: 7 built: 7 ran: 7 failed: 0`; `man-census --fill big` →
      `29 29 29 29 49/49 11 6/6`.

Acceptance: `factorial(100)` matches a constant computed outside MFB and committed as a
test literal; `modPow` agrees with the `pow`+`remainder` reference on every small case;
`mfb man big modPow` renders the non-constant-time advisory.
Commit: —

## Validation Plan

- **Tests:** Rust unit tests in each `func_*.rs` for registry facts; runtime behavior
  extending `tests/rt_big_int.rs`, including every error case above. The Phase 1
  property test lives with the emitter tests inside `src/codegen/builtins/big/`.
- **Coverage check:** `emit_div_mod_magnitude` is the most consequential code in the
  feature and `tests/` gives `src/**` zero coverage — confirm it is in the denominator
  and actually exercised by the property test, not only by `tests/rt_big_int.rs`.
- **Runtime proof:** an `.mfb` program computing `factorial(100)`, a 4096-bit `divMod`,
  and a `modPow` with a 2048-bit modulus, printing each via `toString`, run on a release
  binary against independently computed expectations.
- **Doc sync:** `scripts/man-census.sh --fill big`; `scripts/man-run-examples.sh big
  --run`; `scripts/man-census.sh --memory-scope` → 0 unclassified hits. Verify the §4.5
  advisory renders on all three pages.
- **Acceptance:** `cargo test --no-fail-fast`, then `scripts/test-accept.sh`; regenerate
  `.ir` goldens and prove the delta is only `big`'s.
- **Formatting:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

1. **Algorithm D vs. binary long division.** **Recommend: Algorithm D**, with binary
   long division as a recorded fallback if Phase 1's property test cannot be made to
   pass. The fallback is a decision to write into Corrections with its measured
   slowdown, never a silent substitution.
   **RESOLVED (Phase 1): Algorithm D.** The 10,000-pair property and every targeted case
   (including the D3 correction and D6 add-back pairs) pass; the fallback was not needed.
2. **`gcd(0, 0)`.** **Recommend: return `0`** — the standard convention and the one that
   keeps `gcd` total. Alternative: raise `ErrInvalidArgument`, which makes `gcd`
   fallible for an input that has a defined answer.
   **RESOLVED (Phase 3): return `0`.** `gcd` declares `errors: vec![]`; `gcd(0, 0)` prints `0`
   and results are never negative (`gcd(-12, 18)` → `6`).

## Corrections

<!-- Filled in DURING execution. The `MOD` measurement from Phase 1 goes here, along
     with any consequent change to §4.2. -->

- **C-C1 — `MOD` measured; §4.2 needs no change.** Phase 1 task 1: `/tmp/p127-rt-c/modprobe`
  (operands in `MUT` locals so nothing folds), built with the worktree release `mfb` @
  ad50d1c42, printed `-7 MOD 2 = -1   -7 / 2 = -3`, `7 MOD -2 = 1   7 / -2 = -3`,
  `-7 MOD -2 = -1   -7 / -2 = 3`, `7 MOD 2 = 1   7 / 2 = 3`. Truncated division: quotient toward
  zero, remainder with the dividend's sign — exactly §4.2's intended rule.
- **C-C2 — Phases 1 and 2 share one commit.** Carried from plan-127-A C6: `emit_div_mod_magnitude`
  has no caller until the division members exist, and the `mfb` binary crate warns on an unused
  `pub(crate)` item. Both `Commit:` lines carry the shared hash.
- **C-C3 — the property test runs as a program, not as an in-crate unit test.** The Validation
  Plan places it "with the emitter tests inside `src/codegen/builtins/big/`"; plan-127-A C3
  showed no in-crate test can run an emitter. The 10,000-pair property is
  `division_property_over_ten_thousand_seeded_pairs` in `tests/runtime/rt_big_int.rs`
  (`math::seed(127)`); the in-crate every-backend lowering test covers Algorithm D in process.
  Quotient and remainder are unique under `a = b*q + r`, `|r| < |b|`, the remainder's sign and
  truncation, so the property is a complete check without a second oracle.
- **C-C4 — the D3 and D6 targeted cases were found, not guessed.** An instrumented base-256
  Algorithm D (`/tmp/p127-draft-c/find_d_cases.py`, every result cross-checked against Python
  `divmod`, agreeing with it on 20,000 random pairs) reports `u = [15, 137, 103, 105], v = [50,
  128]` (D3 correction runs; q = 53884, r = 27863) and `u = [38, 17, 133, 176], v = [167, 28,
  129]` (D6 add-back runs; q = 349, r = 8454523). Pinned in `division_targeted_cases`.
- **C-C6 — Phase 2's agreement spread is its own seeded set.** "The Phase 1 spread" is 10,000
  pairs up to 512 bytes; `division_members_agree_and_raise` checks agreement over 500 pairs of
  1–64 bytes (`math::seed(128)`) in all sign combinations. `divide`, `remainder` and `divMod`
  share one lowering (`emit_div_mod_int`), so agreement is a routing check, not an arithmetic
  one; the arithmetic is the Phase 1 property.
- **C-C5 — `modPow` reduces the running base too,** not only the accumulator (§4.4 names
  neither): both products are reduced after every step, so no intermediate exceeds about twice
  the modulus. With truncated remainders the result equals `remainder(pow(base, e), m)`,
  including its sign — which the 390-case agreement check pins.
- **C-C7 — "`gcd` divides both operands" is pinned by oracle values, not a separate
  divisibility loop.** Each `gcd` expectation is Python's `math.gcd`, which is the greatest
  common divisor by definition; the cases cover mixed signs (`gcd(-12, 18) = 6`), a zero operand
  (`gcd(0, -7) = 7`), `gcd(0, 0) = 0`, a large shared factor, and consecutive Fibonacci numbers.
- **Test scope.** Per the user's instruction, each phase runs only its new tests by name
  (`cargo test --release --test rt_big_int division`) and the `big` registry/lowering unit
  tests; earlier phases' runtime tests are not re-run until the single end-of-plan full suite.

## Summary

This letter holds essentially all of the feature's correctness risk, concentrated in one
emitter, and the mitigation is a seeded property test over 10,000 random pairs plus
targeted cases for the normalization boundary and the estimate-correction path — the two
places Algorithm D implementations fail quietly.

Left untouched: the `mfb spec` chapter, the acceptance fixture, the examples, and the
`crypto` integration (plan-127-D).
