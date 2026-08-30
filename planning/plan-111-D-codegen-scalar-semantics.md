# plan-111-D: type codegen's scalar-semantics cluster

Last updated: 2026-08-29
Effort: large (3h–1d)
Depends on: plan-111-C (the type tables and the registry answer typed queries, so
a converted emitter has something typed to call).

The first of three mechanical codegen letters. This one takes the **scalar
semantics** cluster — arithmetic, conversion, string representation, money, SIMD,
error emission and entry/thunk — where the dominant pattern is a `match` on a
rendered scalar type name choosing an instruction sequence.

122 violation sites across 15 files (§2). Every one is the same edit: the
function takes `&ParameterType` instead of `&str`, and its `match` arms become
variant patterns. There is no design content in this letter.

See plan-111-A for the shared prerequisites, the five sanctioned boundaries, the
tiered gate policy, and the rejected alternatives.

References:

- `src/codegen/builtins/math/gen_math.rs` — 28 spelling arms, the single densest
  file in plan-111 and the cleanest instance of the pattern.
- `src/codegen/engine/convert/builder_conversions.rs` — 22 arms + 3 parses; the
  conversion matrix, where a wrong arm is a wrong *number*, not a crash.
- `src/codegen/engine/operators/builder_numeric.rs` — 19 arms + 2 compares +
  2 parses.
- `.ai/codegen-invariants.md` — register lifetimes and the desugars these
  emitters sit inside; `.ai/arch-abi.md` for the per-arch traps in the SIMD and
  entry files.
- `src/numeric.rs` — the single typed promotion algebra (plan-106-A); the
  numeric emitters should be consulting it, not re-deriving from spellings.

## Prerequisites

See plan-111-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-111-C complete | `rg -n '\b(resolve_call\|call_return_type\|argument_types)\(' src/ --glob '!**/tests*'` → definitions only; `string_keyed_type_maps` budget 0 | NOT MET until C lands |
| Scope re-measured at kickoff | the four census commands from plan-111-A §2, restricted to this letter's file list | UNMEASURED — C reduces the parse denominator; re-measure before starting |
| Kickoff `N ran` baseline recorded | `scripts/test-accept.sh <target> /tmp/accept-111d` → record `N ran` | UNMEASURED |

## 1. Goal

- Every file in §2's list takes and matches `ParameterType`. Zero `&str` type
  parameters, zero spelling match arms, zero spelling compares, zero
  `ParameterType::parse` calls in any of them.
- The gate budgets for these 15 files read 0 across all six needle classes.

### Non-goals (explicit constraints)

- See plan-111-A §1 Non-goals — all apply. In particular: **no behavior change.**
  A converted arm must select the identical instruction sequence for the
  identical input type.
- Do not consolidate emitters, merge arms, or "simplify" a dispatch while
  converting it. If two arms look redundant, leave them redundant; a merged arm
  that behaves differently on one type is the exact failure this plan cannot
  have.
- Do not change `src/numeric.rs`'s promotion algebra. If an emitter re-derives
  promotion instead of calling it, record that in Corrections as a finding — do
  not fix it here.
- Do not touch the collection or memory files (letters E and F).

## 2. Current State

These emitters receive a type as a rendered name — historically from
`TypeModel`/registry lookups, which letter C has now made typed — and dispatch on
it with a string `match`. After C, the callers hold a `ParameterType` and render
it purely to satisfy these signatures.

### Measured populations

At HEAD (`fd09ea809`), 2026-08-29, tests excluded. `a` = spelling match arms,
`e` = spelling `==`/`!=`, `p` = `&str` type parameters, `parse` =
`ParameterType::parse` sites. Command: the four patterns from plan-111-A §2 run
per file.

| File (under `src/codegen/`) | a | e | p | parse | total |
|---|---|---|---|---|---|
| `builtins/math/gen_math.rs` | 28 | 0 | 0 | 0 | 28 |
| `engine/convert/builder_conversions.rs` | 22 | 0 | 0 | 3 | 25 |
| `engine/operators/builder_numeric.rs` | 19 | 2 | 0 | 2 | 23 |
| `string/repr/builder_strings.rs` | 11 | 0 | 1 | 0 | 12 |
| `builtins/vector/builder_vector_inline.rs` | 0 | 3 | 4 | 1 | 8 |
| `builtins/money/gen_money_math.rs` | 6 | 0 | 0 | 0 | 6 |
| `builtins/astrings/gen_astrings.rs` | 0 | 0 | 0 | 3 | 3 |
| `builtins/vector/builder_simd_math.rs` | 0 | 0 | 0 | 3 | 3 |
| `error/emission/builder_error_emission.rs` | 0 | 3 | 0 | 0 | 3 |
| `engine/function/entry.rs` | 0 | 3 | 0 | 0 | 3 |
| `builtins/vector/builder_simd_float_math.rs` | 0 | 0 | 0 | 2 | 2 |
| `builtins/vector/builder_simd_fixed_math.rs` | 0 | 0 | 0 | 2 | 2 |
| `link/thunk/link_thunk.rs` | 0 | 2 | 0 | 0 | 2 |
| `builtins/math/gen_pow.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/money/func_get_rounding.rs` | 0 | 0 | 0 | 1 | 1 |
| **Total** | **86** | **13** | **5** | **18** | **122** |

### Verified properties

- **These 15 files are disjoint from letters E and F.** The per-file census in
  plan-111-A's working notes covers all 71 codegen files with any violation;
  D (122) + E (161) + F (175) = 458 = codegen's measured total
  (147 arms + 59 compares + 143 params + 109 parses). No file appears twice and
  none is unassigned.
- **UNVERIFIED: how many of the 18 parses survive letter C.** Several are
  registry-adjacent and C may remove them as a side effect. Re-measure at
  kickoff; a lower number is C working, not a scope error.

## 3. Design Overview

One phase per coherent sub-cluster, ordered smallest-blast-radius first. There is
no design uncertainty in this letter — the uncertainty was spent in A and C.

Where correctness risk sits, in order:

1. **`builder_conversions.rs` (22 arms).** The conversion matrix decides which
   numeric conversion instruction runs. A misrouted arm produces a *wrong value*,
   not a crash — invisible to a compile-only check and invisible to byte-identity
   only if the wrong arm happens to emit the same bytes. The rt-behavior fixtures
   are the real guard here.
2. **The SIMD files.** Per-arch, and `.ai/arch-abi.md`'s traps apply; an x86-only
   miscompile is invisible on a Mac host except through `.ncodesum` — which this
   plan does not check until letter G. Phase 3 compensates by converting these
   files without touching their arch dispatch at all.
3. **`entry.rs` and `link_thunk.rs`.** ABI-adjacent; `mfb_return` ≡ `c_return` on
   ARM but differs on x86 (the audit-ABI-by-emission-path memory), so a mistake
   here is again host-invisible.

Per plan-111-A §3, the per-phase gate is `cargo test --no-fail-fast` (including
`golden.rs`, host-target byte-identity) plus `diag-set-diff.sh`; the
cross-target `artifact-gate.sh all` is letter G's single run. For this letter
specifically, `cargo test --no-fail-fast` must be read as covering the `rt_*`
runtime tests — **never plain `cargo test`**, which stops at `golden.rs` and
silently skips every `rt_*` test, which are exactly the tests that catch a wrong
conversion arm.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial with one line on what remains; mark
> moot tasks `- [x] ~~text~~ — moot: <evidence>`; fill `Commit:` when a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — arithmetic and money (34 sites, no `&str` params)

Pure arm conversion; these two files take no type as `&str`, so the change is
local to their `match` bodies.

- [ ] `builtins/math/gen_math.rs` — convert 28 spelling arms to `ParameterType`
      variant patterns.
- [ ] `builtins/money/gen_money_math.rs` — convert 6 arms.
- [ ] `builtins/math/gen_pow.rs`, `builtins/money/func_get_rounding.rs` — delete
      the 1 parse each.
- [ ] Lower the gate budgets for these four files to 0.
- [ ] Tests: no new tests — the existing `rt_*` math/money fixtures cover these.
      If a converted arm turns out to have no fixture, record the gap in
      Corrections and add one.

Acceptance: the four files read 0 on all six needle classes;
`cargo test --no-fail-fast` green including `golden.rs` and every `rt_*` test.
Commit: —

### Phase 2 — conversion, numeric operators, string representation (60 sites)

The highest-value files in this letter, and the one real correctness risk.

- [ ] `engine/convert/builder_conversions.rs` — convert 22 arms and delete 3
      parses. Convert **one arm per commit or in small named groups**; a 22-arm
      rewrite in one commit cannot be reviewed against the conversion matrix.
- [ ] `engine/operators/builder_numeric.rs` — convert 19 arms, 2 compares, delete
      2 parses. Where an arm re-derives numeric promotion, call
      `src/numeric.rs`'s typed algebra instead of reimplementing it — but only
      where it is already equivalent; a behavior change here is out of scope and
      belongs in Corrections.
- [ ] `string/repr/builder_strings.rs` — convert 11 arms and 1 `&str` param.
- [ ] Lower the gate budgets for these three files to 0.
- [ ] Tests: add an rt-behavior fixture for any conversion pair in the matrix
      that the existing corpus does not exercise — determine which by reading the
      matrix against the fixture list, not by assuming coverage.

Acceptance: the three files read 0 on all six needle classes; every `rt_*`
numeric/conversion/string test green under `cargo test --no-fail-fast`;
`scripts/diag-set-diff.sh` 0 differing.
Commit: —

### Phase 3 — SIMD, error emission, entry and thunk (28 sites, host-invisible risk)

Scheduled last: these are the files where a mistake does not show up on the host.

- [ ] `builtins/vector/builder_vector_inline.rs` — 3 compares, 4 `&str` params,
      1 parse.
- [ ] `builtins/vector/builder_simd_math.rs` (3 parses),
      `builder_simd_float_math.rs` (2), `builder_simd_fixed_math.rs` (2).
- [ ] `builtins/astrings/gen_astrings.rs` — 3 parses.
- [ ] `error/emission/builder_error_emission.rs` — 3 compares.
- [ ] `engine/function/entry.rs` — 3 compares.
- [ ] `link/thunk/link_thunk.rs` — 2 compares.
- [ ] **Do not touch arch dispatch in any of these files.** The conversion is to
      the type argument only; if a change appears to require touching an
      arch branch, stop and record why in Corrections.
- [ ] Lower the gate budgets for these seven files to 0.
- [ ] Tests: run the vector/SIMD `rt_*` suite explicitly and record the count.

Acceptance: all 15 files in §2 read 0 on all six needle classes; the letter's
end gate below passes.
Commit: —

## Validation Plan

- Tests: `cargo test --no-fail-fast` — **never plain `cargo test`**. The `rt_*`
  tests sort after `golden.rs` and are silently skipped without `--no-fail-fast`;
  they are the only tests that catch a wrong conversion arm.
- Gate: `cargo test --test no_type_strings` — all 15 files at 0, budgets tight.
- Coverage check: the conversion matrix in `builder_conversions.rs` is the one
  place to verify coverage rather than assume it — enumerate its arms against the
  rt fixture list and record any arm with no fixture.
- Runtime proof: `scripts/test-accept.sh` with scratch `/tmp/accept-111d` at the
  kickoff `N ran` and 0 mismatches, plus `MFB_OPT=3 scripts/test-accept.sh` —
  `-O2+` rows are not exercised by default-level sweeps.
- Artifact gate: **not run in this letter** (plan-111-A §3). Host-target
  byte-identity comes from `golden.rs`; the cross-target sweep is letter G's.
- Diagnostics: `scripts/diag-set-diff.sh` → 0 differing.
- Doc sync: `.ai/codegen-invariants.md` if any emitter's documented contract
  mentions a `&str` type argument.
- Formatting: `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

None. This letter's design was settled in plan-111-A and plan-111-C; if a
decision appears necessary, it is a sign the conversion is changing behavior —
stop and record it in Corrections.

## Corrections

<Filled in DURING execution.>

## Summary

Risk is `builder_conversions.rs` (a wrong arm is a wrong number, caught only by
rt fixtures) and the SIMD/entry/thunk files (host-invisible, caught only by
letter G's cross-target sweep). Everything else is arm-for-arm substitution.

Untouched: collections and layout (letter E), memory/engine/resource/registry
(letter F), and the five sanctioned boundaries.
