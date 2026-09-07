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
| plan-127-B is complete: Phases 1–4 ticked and their commits recorded | `grep -c '^- \[ \]' planning/plan-127-B-big-int-arithmetic.md` → `0` | NOT MET |
| The magnitude emitters exist | `grep -c "fn emit_add_magnitude\|fn emit_sub_magnitude\|fn emit_mul_magnitude" src/codegen/builtins/big/gen_big.rs` → `3` | NOT MET |

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
| Members existing after plan-127-B | 23 | plan-127-A §4.4 (11) + plan-127-B §4.3 (12) |

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

- [ ] Measure MFBASIC's `MOD` sign convention: run a program printing `-7 MOD 2`,
      `7 MOD -2`, `-7 MOD -2`, and the matching `/` quotients. Record the results and
      the command in Corrections, and reconcile §4.2 against them.
- [ ] `gen_big.rs`: `emit_div_mod_magnitude` per §4.1, with all three parts and a
      comment distinguishing Algorithm D's operand normalization from
      `emit_build_int`'s record normalization.
- [ ] Property test: for 10,000 random `(a, b)` pairs with `b ≠ 0`, spanning 1 to 512
      bytes of magnitude and all four sign quadrants, assert
      `a = b × quotient + remainder` and `|remainder| < |b|`. Seed fixed and recorded so
      a failure reproduces.
- [ ] Targeted tests for the cases the correction step exists for: divisor high byte
      just below and just above 128 (normalization boundary); a dividend whose leading
      bytes force the estimate one too large; divisor longer than dividend (quotient 0,
      remainder = dividend); divisor equal to dividend.

Acceptance: the 10,000-pair property test passes with a recorded seed, **and** each
targeted case passes. A failure here is root-caused in Algorithm D, not worked around
by weakening the property.
Commit: —

### Phase 2 — the division members

- [ ] `mod.rs`: `add_record` for `DivResult` with the field-order comment.
- [ ] `func_divide.rs`, `func_remainder.rs`, `func_div_mod.rs` — all three declare
      `ErrInvalidArgument` for a zero divisor and nothing else.
- [ ] Tests: each of the three raises `ErrInvalidArgument` on `b = 0`; `divide` and
      `remainder` agree with the matching `divMod` field across the Phase 1 spread; the
      §4.2 sign convention holds on the four `(±7, ±2)` cases.

Acceptance: `native_member_declares_error` reports `true` for all three; the agreement
and sign tests pass.
Commit: —

### Phase 3 — the members built on division

- [ ] `func_pow.rs`, `func_gcd.rs`, `func_factorial.rs`, `func_mod_pow.rs`.
- [ ] `modPow`, `pow` and `factorial` each declare `ErrInvalidArgument`; `gcd` declares
      nothing.
- [ ] `modPow`'s, `compare`'s and `equals`'s descriptors carry the §4.5 advisory.
      Amending `compare`/`equals` (landed in plan-127-A) is in scope for this phase.
- [ ] Tests: `pow(x, 0)` is `1` and `pow(x, 1)` is `x`; `pow` agrees with folded
      `multiply` for small exponents; `gcd` is non-negative, divides both operands, and
      `gcd(0, 0)` is `0`; `factorial(0)` is `1`, `factorial(20)` matches the exact
      `Integer` value, `factorial(100)` matches an independently computed constant;
      `modPow(b, e, m)` agrees with `remainder(pow(b, e), m)` for small `e`;
      `modPow` with `modulus = 0` raises `ErrInvalidArgument`;
      negative `exponent`/`n` each raise `ErrInvalidArgument`.

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
2. **`gcd(0, 0)`.** **Recommend: return `0`** — the standard convention and the one that
   keeps `gcd` total. Alternative: raise `ErrInvalidArgument`, which makes `gcd`
   fallible for an input that has a defined answer.

## Corrections

<!-- Filled in DURING execution. The `MOD` measurement from Phase 1 goes here, along
     with any consequent change to §4.2. -->

## Summary

This letter holds essentially all of the feature's correctness risk, concentrated in one
emitter, and the mitigation is a seeded property test over 10,000 random pairs plus
targeted cases for the normalization boundary and the estimate-correction path — the two
places Algorithm D implementations fail quietly.

Left untouched: the `mfb spec` chapter, the acceptance fixture, the examples, and the
`crypto` integration (plan-127-D).
