# plan-127-D: `crypto` consumes `big::Int`, and the feature's documentation closeout

Last updated: 2026-09-06
Effort: medium (1h–2h)
Depends on: plan-127-C

The one place in the tree that has a documented 64-bit ceiling `big::Int` removes is
`crypto::randomInt`, which today rejects any range whose span overflows a signed 64-bit
`Integer`. This letter gives it a `big::Int` overload with the same unbiased-sampling
contract and no span limit, then closes out the feature's spec chapter, acceptance
fixture and examples.

Behavioral outcome: `crypto::randomInt(min AS big::Int, max AS big::Int)` returns a
uniformly distributed `big::Int` in the inclusive range for spans of any size, and the
`big` package is documented in `mfb spec` and covered by the acceptance suite.

References — read these first:

- `src/codegen/builtins/crypto/func_random_int.rs` — the existing member, its
  rejection-sampling contract, and the span restriction this letter lifts.
- `src/codegen/builtins/crypto/func_random_bytes.rs` — the CSPRNG entropy source the
  overload draws through.
- `src/docs/spec/stdlib/10_crypto.md` — the crypto spec chapter.
- `.ai/specifications.md` — keeping the embedded spec current.
- `.ai/man-content.md` — the man-page content standard.

## Prerequisites

Stated once in plan-127-A and unchanged. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-127-C is complete: Phases 1–3 ticked and their commits recorded | `grep -c '^- \[ \]' planning/completed/plan-127-C-big-int-division.md` → `0` | MET (2026-09-13, worktree-P-127 @ 634c20da8: `0`; plan archived to `planning/completed/`) |
| The members this letter's overload needs exist | `grep -rl 'name: "divMod"\|name: "compare"\|name: "subtract"\|name: "add"\|name: "bitLength"' src/codegen/builtins/big/ \| wc -l` → `5` | MET (2026-09-13, @ 634c20da8: 5 files — `func_add`, `func_compare`, `func_subtract`, `func_div_mod`, `func_bit_length`) |

If plan-127-C is not complete, this letter cannot start, full stop. Unbiased sampling
over a big range needs `subtract` (span), `bitLength` (draw width), `compare`
(rejection) and `add` (offset); rejection sampling without all four is biased, and a
biased CSPRNG member is worse than no member.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. **If you stop, report the current status of *all* prerequisites.**

## 1. Goal

- `crypto::randomInt(min AS big::Int, max AS big::Int) AS big::Int` exists, is
  uniformly distributed, and imposes **no** span restriction.
- The existing `crypto::randomInt(Integer, Integer) AS Integer` is unchanged in
  signature, semantics and error set.
- `mfb spec stdlib big` renders a chapter documenting the type, the canonical form, and
  the absence of operators.
- `tests/acceptance/src/big.mfb` exists and passes under `scripts/test-accept.sh`.

### Non-goals

Every non-goal in plan-127-A §1 applies unchanged. Additionally:

- **No change to `crypto`'s existing members.** The `Integer` overload of `randomInt`
  keeps its exact signature, contract and errors. Adding an overload must not alter
  which implementation an existing call site selects.
- **No `big::Int` in `crypto`'s internal field or scalar arithmetic.** `helper_mod_l`,
  `helper_inv25519`, `helper_scalar_below_l`, `helper_gf448_inv` and the Ed25519/Ed448
  point helpers stay exactly as they are. They are constant-time by construction and
  `big::Int` is not; substituting it would be a security regression, not a
  simplification. This is the single most important constraint in this letter.
- **No `big::Int` overload of `crypto::constantTimeEqual`.** A magnitude's length varies
  with its value, so a big-integer comparison leaks length even when the byte loop does
  not. The honest answer is documentation pointing callers at the fixed-width byte form,
  not a member whose name promises more than it delivers.
- **No ASN.1/DER work, and no change to `packages/`.**

## 2. Current State

`crypto::randomInt(min AS Integer, max AS Integer) AS Integer` draws fresh entropy
through `crypto::randomBytes` per call and uses rejection sampling for an exactly
uniform distribution — explicitly rather than reducing raw entropy modulo the range,
"which skews toward smaller values when the range does not divide the entropy space
evenly" (`src/codegen/builtins/crypto/func_random_int.rs:DESC`, found with
`grep -n "Unbiased sampling" src/codegen/builtins/crypto/func_random_int.rs`).

That member documents its own ceiling: *"Because the count of outcomes is
`max - min + 1`, a span so large that `max - min` overflows a signed 64-bit `Integer`
is also rejected with `ErrInvalidArgument`"* (same `DESC`, `grep -n "Range and errors"`).
That restriction is a consequence of `Integer`, not of the algorithm, and it is the gap
this letter closes.

After plan-127-C the `big` package has 29 members (was "30" — plan-127-A Corrections C5)
and two exported records, all
natively lowered, none of them documented in `mfb spec`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `crypto` members before this letter | 21 | `ls src/codegen/builtins/crypto/func_*.rs \| wc -l` → 21 |
| `crypto` member files this letter edits | 1 | `func_random_int.rs` |
| `crypto` internal arithmetic helpers this letter must NOT touch | 14 | `ls src/codegen/builtins/crypto/helper_*.rs \| grep -icE "mod_l\|inv25519\|gf448_inv\|scalar\|point\|pack_point"` → 14 |
| Package spec chapters under `src/docs/spec/stdlib/` | 18 (+ `spec.md`) | `ls src/docs/spec/stdlib/ \| wc -l` → 19 |
| Acceptance fixtures under `tests/acceptance/src/` | 20 | `ls tests/acceptance/src/ \| wc -l` → 20 |

### Verified properties

- **Overload resolution selects on concrete argument types and returns the first
  implementation that unifies.** VERIFIED by reading `match_overload`
  (`src/codegen/registry/mod.rs:514-542`). `ParameterType::Integer` and
  `Named("big.Int")` do not unify with each other, so adding the `big::Int` row cannot
  capture an existing `Integer` call site. Phase 1 asserts this rather than assuming it.
  **FALSE as written (D-C4):** they do not unify STRICTLY, but lenient `leaf_matches`
  accepts a scalar against a nominal in either direction, so on the lenient return-type
  path the earlier `Integer` row captured the `big::Int` call. The `Integer` call site was
  never at risk (strict and lenient both pick row 1 for it); the `big::Int` call site was.
- **`crypto` implements its own field and scalar arithmetic and does not need a general
  bignum.** VERIFIED by reading the helper inventory: `helper_mod_l.rs`,
  `helper_ed448_mod_l.rs`, `helper_inv25519.rs`, `helper_gf448_inv.rs`,
  `helper_scalar_below_l.rs`, `helper_ed448_scalar_below_l.rs`, `helper_clamp_scalar.rs`
  and the point/scalarmult helpers implement Ed25519 and Ed448 arithmetic directly.
  These are curve-specific and constant-time; `big::Int` is neither. **`randomInt` is the
  only member where `big::Int` is an improvement rather than a regression.**
- **UNVERIFIED — whether a fallibility census keys on member name across overloads.**
  Both `randomInt` overloads declare `ErrInvalidArgument`, so their verdicts agree and
  the question does not bite here. Phase 1 confirms the census is green rather than
  reasoning about it.

## 3. Design Overview

Three independent pieces, landable in order: the crypto overload (behavior), the spec
chapter (documentation), the acceptance fixture and examples (coverage).

**Where correctness risk concentrates: the uniformity of the big overload.** A bias in
a CSPRNG-backed member is invisible to functional testing — every individual result
looks fine — and it is a security defect, not a quality one. Phase 1's acceptance is
therefore a distribution test over a small range where bias would be measurable, not
just a range-membership check.

**Where design uncertainty concentrates: none.** Rejection sampling over a big range is
the same algorithm as over an `Integer` range, with `bitLength` supplying the draw
width; the existing member is the reference.

**Byte-identity is not this letter's gate.** Same narrow non-disturbance role: a program
that does not `IMPORT big` and does not call the new overload must stay byte-identical.

### Rejected alternatives

- **A `big::random` member in the `big` package instead.** Rejected: it would need the
  CSPRNG, which is `crypto`'s, and a random-number member in a pure-arithmetic package
  invites use as a general RNG. It belongs with the entropy source.
- **Reducing a wide random draw modulo the span.** Rejected for the reason the existing
  member already documents — it skews toward smaller values. Rejection sampling, same as
  the `Integer` overload.

## 4. Detailed Design

### 4.1 The `crypto::randomInt` big overload

```basic
crypto::randomInt(min AS big::Int, max AS big::Int) AS big::Int   ' ErrInvalidArgument (max < min)
```

Algorithm, mirroring the existing member:

1. If `big::compare(max, min) < 0`, raise `ErrInvalidArgument`. If equal, return `min`.
2. `span = big::add(big::subtract(max, min), big::fromInteger(1))`.
3. `width = big::bitLength(span)`; draw `ceil(width / 8)` bytes from
   `crypto::randomBytes` and mask the high byte down to `width` bits.
4. Build a candidate with `big::fromBytes(bytes, FALSE, Little)`. If
   `big::compare(candidate, span) >= 0`, discard and redraw — **never** reduce.
5. Return `big::add(min, candidate)`.

The masking in step 3 is what bounds the expected number of redraws to under two; without
it, a span just over a byte boundary rejects almost everything. Fresh entropy is drawn
per attempt, matching the existing member.

The only error is `ErrInvalidArgument` for `max < min`. There is no span restriction —
that is the point — and the `DESC` must say so where the `Integer` overload's ceiling is
documented, so a reader who hits the ceiling finds the way past it on the same page.

### 4.2 The spec chapter

A new `src/docs/spec/stdlib/19_big.md`, registered in `src/docs/spec/stdlib/spec.md`,
covering:

- the `Int` value type, its two fields, and its **canonical form**;
- that a `big::Int` is a copy-semantics value, not a handle, and needs no `close`;
- that `=`, `<>`, `<` and the arithmetic operators do **not** apply, with the reason
  (a `List` field is not comparable, and records are never orderable) and the members
  that replace them;
- that a `big::Int` cannot be a `Map` key or `Set` element, and that keying by
  `big::toString` is the workaround;
- the total/fallible split, and that the arithmetic members cannot overflow;
- that nothing in `big` is constant-time, pointing at `crypto::`.

The last two are the facts a reader most needs and least expects.

### 4.3 Acceptance fixture and examples

`tests/acceptance/src/big.mfb`, joining the 20 existing fixtures, exercising: the
conversion round trips in both byte orders; the arithmetic identities; a 4096-bit
multiply; `divMod`'s defining identity; `factorial(100)`; `parse`∘`toString`; and the
new `crypto::randomInt` big overload's range membership.

## Compatibility / Format Impact

- **Added:** one overload row on `crypto::randomInt`; one spec chapter; one acceptance
  fixture.
- **Unchanged:** `crypto::randomInt`'s existing `Integer` signature, contract and error
  set; every other `crypto` member; all 14 crypto internal arithmetic helpers; `big`'s
  surface, layout and canonical form; every file under `packages/`.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partial and say what remains. Fill each
> `Commit:` line the moment it lands. **An unticked box means NOT DONE.**

### Phase 1 — the `crypto::randomInt` big overload (the only behavior change here)

- [x] Add the `big::Int` implementation row to `func_random_int.rs` per §4.1, as a
      second `implementations` entry — not a new member and not a renamed one. (Rewrite row
      to `__crypto_randomIntBig` + gated helper `helper_random_int_big.rs`, D-C1/D-C2;
      `git status --short src/codegen/builtins/crypto/` → `func_random_int.rs`, `mod.rs` (helper
      registration + tests), new `helper_random_int_big.rs`; the 14 internal arithmetic helpers
      untouched. Required the D-C4 resolver fix.)
- [x] Extend the member's `DESC` where the 64-bit span ceiling is documented to name the
      big overload as the way past it. ("**Past the 64-bit ceiling.**" paragraph right after
      "Range and errors", plus a `big::Int` example; `mfb man crypto randomInt` renders
      `Overloads 1. …(min AS Integer, max AS Integer) AS Integer` / `2. …(min AS big::Int, max
      AS big::Int) AS big::Int` and errors `1, 2` for all three codes.)
- [x] Test: an existing `crypto::randomInt(1, 10)` call site still selects the `Integer`
      implementation and returns an `Integer` — `registry::call_return_type_typed`
      reports `Integer` for the two-`Integer` shape and `big.Int` for the two-`big.Int`
      shape. (`call_return_type_typed` takes no argument types, so the typed resolvers are
      used: `random_int_big_overload_registry_facts` — `rewrite_target` →
      `__crypto_randomInt` / `__crypto_randomIntBig`, strict `resolve_call_typed` → `Integer` /
      `big.Int`, both rows' errors exact; `argument_typed_return_resolution` — lenient
      `resolve_call` → `Integer` / `big.Int`, the latter red before D-C4. `cargo test --release
      -p mfb --bin mfb -- codegen::registry:: codegen::builtins::crypto::tests
      inline_builtin_fallibility_census` → `68 passed; 0 failed`.)
- [x] Test: `max < min` raises `ErrInvalidArgument` on the big overload; `min = max`
      returns `min`. (`crypto_random_int_big_overload_is_in_range_and_uniform` → `raised
      77050002`, `123456789012345678901234567890`.)
- [x] Test: every result of 1,000 draws over a big range lies within `[min, max]`. (Same
      test, range `[-2^128, 2^128 - 1]` → `outside: 0 of 1000`, `past Integer: TRUE` (>990 of
      1000 draws beyond `Integer`) — also the Validation Plan's runtime proof.)
- [x] **Distribution test:** 10,000 draws over a small range (`0..6`) built from
      `big::Int` values; assert every outcome occurs and no bucket deviates from the
      expected count by more than a stated tolerance. A modulo-reduction bug fails this
      and passes every other test in this phase. (Same test: `draws: 10000`, `every outcome:
      TRUE`, `within 25% of 1429: TRUE` (each bucket in 1072..1786, about 10 standard deviations
      wide); `cargo test --release --test rt_big_int crypto_random_int_big_overload` → `1
      passed; 0 failed`.)
- [x] Confirm the inline-`TRAP` fallibility census is green with the overloaded member
      (both rows declare `ErrInvalidArgument`, so the verdicts agree).
      (`inline_builtin_fallibility_census` passes, and `random_int_big_overload_registry_facts`
      asserts `inline_builtin_is_infallible("crypto.randomInt", …)` is `false` for both
      argument shapes.)
- [x] Added: the two registry block-ownership census tests list the 26 `big` block-returning
      members (D-C5) → both pass in the 68 above.
- [x] Added: `scripts/man-run-examples.sh crypto --run randomInt` → `examples: 3 built: 3 ran: 3
      failed: 0`, example 3 printing `TRUE` (it failed with `TYPE_BINDING_MISMATCH` before D-C4).

Acceptance: the distribution test passes, the `Integer` overload's return type and error
set are unchanged, and `mfb man crypto randomInt` renders both signatures.
Commit: aedc4c542

### Phase 2 — the spec chapter

- [x] `src/docs/spec/stdlib/19_big.md` per §4.2; register it in
      `src/docs/spec/stdlib/spec.md`. (Reading-order entry `big` after `transports`; covers the
      value, canonical form, value-not-handle, operators/keys/elements, the total/fallible
      split, division, and timing. `cargo test --release -p mfb --bin mfb docs::spec::` → `8
      passed; 0 failed`, including `spec_citations_resolve` over its five `[[…]]` citations.)
- [x] Update `src/docs/spec/stdlib/10_crypto.md` where `randomInt`'s span ceiling is
      described, to name the big overload. (The "Secure random and identifiers" bullet names
      the `Integer` form's span limit and the `big::Int` form's none, citing
      `helper_random_int_big.rs:BODY`. The chapter never stated the ceiling before, so it is
      stated here with the way past it — D-C6.)
- [x] Verify: `mfb spec stdlib big` renders; every claim in it is checked against the
      shipped descriptors rather than against plan-127-A/B/C. (Renders; the fallible table
      and total list match each member's `errors:` vector (`grep -n 'errors:'
      src/codegen/builtins/big/func_*.rs`). Probe programs `/tmp/p127-spec-probe` built with the
      release `mfb`: `a = b`/`a <> b` → `2-203-0061 TYPE_REQUIRES_COMPARABLE`; `a < b`, `a + b`,
      `a / b` → `2-203-0001 TYPE_BINARY_OPERATOR_MISMATCH`; `Map OF big::Int TO Integer` and
      `Set OF big::Int` → `2-203-0061`; the accepted program printed `TRUE 0` (a MUT default is
      zero), `-7 TRUE` (hand-built `big::Int[[7, 0, 0], TRUE]` reads as -7), `0 0` (negative zero
      reads as zero) and `1` (a `toString`-keyed map). Timing: `big::compare` on 64 KB values →
      `top byte differs: 10 us for 2000 compares`, `bottom byte differs: 79321 us`. First render
      showed two defects, fixed before ticking: bullets whose text held `<>`/`<` lost their
      continuation indent (rewritten as single-line bullets) and backticks inside bold rendered
      literally (D-C6).)

Acceptance: `mfb spec stdlib big` renders the chapter, and its operator, comparability
and constant-time statements each match the shipped behavior when spot-checked by
running a program that attempts them.
Commit: —

### Phase 3 — acceptance fixture, examples, and the census closeout

- [ ] `tests/acceptance/src/big.mfb` per §4.3; wire it into the acceptance project the
      way the existing 20 fixtures are.
- [ ] Regenerate goldens with `scripts/sync-goldens.sh` and prove the delta is only
      `big`'s and the new fixture's.
- [ ] Run `scripts/man-census.sh --fill big` — every member and both record types
      documented, no gaps.
- [ ] Run `scripts/man-census.sh --memory-scope` — 0 unclassified hits across the `big`
      pages. The C/Rust memory vocabulary is banned; "copy" and "value" are the
      permitted words for what a `big::Int` does.
- [ ] Run `scripts/man-run-examples.sh big --run` — every example on every `big` page
      compiles and runs.
- [ ] Run `scripts/man-run-examples.sh crypto --run` — the amended `randomInt` page's
      examples still compile and run.

Acceptance: `scripts/test-accept.sh` passes with the new fixture; both census runs
report zero gaps; both example runs are green.
Commit: —

## Validation Plan

- **Tests:** Rust unit tests in `func_random_int.rs` for the overload's registry facts
  (both signatures resolve, both declare `ErrInvalidArgument`, return types are
  `Integer` and `big.Int` respectively). Runtime behavior in `tests/rt_big_int.rs` or a
  sibling, including the distribution test and the error cases.
- **Coverage check:** the new overload's lowering must be in the coverage denominator —
  `tests/` gives `src/**` zero coverage, so the registry facts are asserted from unit
  tests inside `src/codegen/builtins/crypto/`.
- **Runtime proof:** an `.mfb` program drawing from a range whose span exceeds
  `Integer`, printing several results and asserting range membership — the case the
  `Integer` overload rejects outright. Run on a release binary.
- **Regression proof:** `crypto`'s existing tests pass untouched, and the 14 internal
  arithmetic helpers are byte-identical — `git diff --stat src/codegen/builtins/crypto/`
  shows only `func_random_int.rs`.
- **Doc sync:** `mfb man crypto randomInt`, `mfb man big`, `mfb spec stdlib big`,
  `mfb spec stdlib crypto`.
- **Acceptance:** `cargo test --no-fail-fast`, then `scripts/test-accept.sh`.
- **Formatting:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

1. **Whether the big overload lives on `crypto::randomInt` or a distinct name.**
   **Recommend: the overload** — same contract, same entropy source, same mental model,
   and `match_overload` selects on concrete types so no existing call site moves.
   Alternative: `crypto::randomBig`, which is more searchable but splits one concept
   across two names.
2. **Distribution-test tolerance.** **Recommend: assert every one of the 7 outcomes
   occurs and no bucket deviates by more than 25% from the expected 1,429** — loose
   enough not to flake, tight enough that a modulo-reduction bias over `0..6` fails it.
   Alternative: a chi-squared test, which is stricter but adds a statistical dependency
   to the suite for one member.

## Corrections

<!-- Filled in DURING execution. -->

- **D-C1 — `crypto::randomInt` is a source-companion rewrite, so the overload is a second rewrite
  row plus a gated helper.** §4.1 assumes a native member. The shipped `Integer` row is
  `Body::Rewrite("__crypto_randomInt")` (MFBASIC in `crypto/helper_random_int.rs`). The big row
  rewrites to `__crypto_randomIntBig` in `crypto/helper_random_int_big.rs`, registered with
  `HelperGate::WhenBothImported("crypto", "big")` (precedent: the `term`/`astrings` bridge,
  `term/helper_astrings_bridge.rs`) and carrying its own `IMPORT crypto`/`big`/`collections`.
  `WhenImported("big")` would be wrong: it injects into a program importing `big` without
  `crypto`.
- **D-C2 — the big row's error set is not only `ErrInvalidArgument`.** §4.1 says "The only error
  is `ErrInvalidArgument`". The helper draws through `crypto::randomBytes`, which declares
  `ErrUnknown` and `ErrOutOfMemory`; the `Integer` row declares all three for the same reason, so
  the big row declares the same three and the census verdicts agree.
- **D-C3 — the helper composes native `big` members and does no digit arithmetic,** so
  plan-127-A's "no MFBASIC-source implementation" non-goal (about the arithmetic) is not crossed.
- **D-C4 — §2's "overload resolution returns the first implementation that unifies" was a
  latent defect for this overload, and a Phase 1 prerequisite no task covered.** `ParameterType::
  Integer` and `Named("big.Int")` DO unify on the lenient path: `leaf_matches` accepts a scalar
  against a nominal in either direction when not strict. Lenient `dispatch` feeds return-type
  inference (`resolved_return_type`), so `crypto::randomInt(bigA, bigB)` inferred `Integer`
  (first row) while `rewrite_target` — already strict-first — ran the big body. Measured before
  the fix: `LET x AS big::Int = crypto::randomInt(low, high)` failed with `2-203-0007
  TYPE_BINDING_MISMATCH` (`man-run-examples.sh crypto --run randomInt`, example 3), the runtime
  test failed at build, and `resolve_call("crypto.randomInt", ["big.Int","big.Int"], false)` →
  `Some("Integer")`. Fix: `resolved_return_type`'s lenient arm is
  `resolve(call).or_else(|| dispatch(call))`, the same preference `rewrite_target` uses.
  Blast radius measured with a temporary in-crate census over every multi-implementation member,
  each called with each implementation's own parameter types: `shapes=367 deltas=1`, the one delta
  `randomInt(big.Int, big.Int) old=Some("Integer") new=Some("big.Int")`. The census was removed
  after measuring; the pin is `argument_typed_return_resolution` (lenient) plus
  `random_int_big_overload_registry_facts` (strict + rewrite targets).
- **D-C5 — letters A–C left two registry census tests red; found here, fixed here.**
  `codegen::registry::raw_result_block_ownership::every_block_returning_runtime_helper_is_classified`
  and `every_string_returning_runtime_helper_is_marked_fresh` scrape the runtime-call catalog and
  demand every block-returning call be listed in `CALLER_ARENA_BLOCK_RESULTS` (and every `String`
  one in `STRING_RESULT_HELPERS`) after confirming its result is allocated in the caller's arena
  and handed back as the only pointer — those lists license the caller to free an unbound result.
  The per-phase scoped runs (`codegen::builtins::big`) never reached them. Measured: 26 `big`
  names missing from the first list, `big.toString`/`big.toRadixString` from the second. Audit of
  every result publisher in `gen_big.rs` and the member files (`grep -n
  'RESULT_VALUE_REGISTER\|emit_alloc_magnitude(\|emit_alloc('`): results come only from
  `emit_build_int` over an `emit_alloc_magnitude` block (`emit_alloc` →
  `ARENA_ALLOC_SYMBOL = "_mfb_arena_alloc"`), the fold/`pow`/`factorial`/`gcd`/`modPow`
  accumulators (each a fresh record; `pow`'s borrowed-argument square is never returned or
  released), `emit_int_to_string`'s `emit_alloc`, `toBytes`'s `emit_build_byte_list`, and
  `divMod`'s `emit_build_inlined_record_sized`. No path returns an argument or rodata, so all 26
  are added (sorted) with a comment naming the allocation.
- **D-C6 — Phase 2 docs: two render defects and one unstated ceiling.** (a) In
  `19_big.md` a wrapped bullet whose text held `` `<>` `` / `` `<` `` rendered its continuation
  line unindented under `mfb spec stdlib big`; the two bullets are single source lines now.
  Backticks inside a bold run rendered literally ("Nothing in `big`"); the bold sentence has no
  code span now. (b) §2 says `10_crypto.md` describes `randomInt`'s span ceiling; it did not
  (`grep -n -i randomInt src/docs/spec/stdlib/10_crypto.md` → one bullet, no ceiling). The
  bullet now states both forms' span rules. A citation placed mid-sentence left " , uuid4"
  after stripping, so it moved to the bullet's end, the file's convention.
- **D-C7 — the acceptance fixture's helpers carry a `big` prefix** (`bigToInteger`,
  `bigDivide`, …) because `tests/acceptance` is one project with one `FUNC` namespace.

## Summary

The feature's only genuine crypto consumption is `randomInt`, and the discipline that
matters is knowing where it stops: `crypto`'s 14 curve and scalar helpers are
constant-time by construction, `big::Int` is not, and swapping one for the other would
trade a documented limitation for a silent side channel. This letter lifts the one
ceiling that is an artifact of `Integer` and leaves the rest alone.

Risk sits in the uniformity of the new draw, which no functional test would catch —
hence a distribution test rather than a range check.

With this letter the feature is complete: 29 `big` members, two records, one crypto
overload, one spec chapter, one acceptance fixture. `big::Dec` remains undesigned and
purely additive whenever it is wanted.
