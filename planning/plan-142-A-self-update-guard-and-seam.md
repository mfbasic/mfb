# plan-142-A: The self-update census guard and the one-dispatch seam

Last updated: 2026-09-20
Overall Effort: huge (>3d)
Effort: large (3h–1d)
Depends on: nothing (the Prerequisites below gate the whole of plan-142)

## plan-142 as a whole

**Goal of the whole plan:** every non-record self-update `x = op(x, …)` of a `MUT`
binding whose value is a `List`, `Map` or `Set` mutates `x`'s existing block in
place — no copy of `x` — at four binding sites: a **function local** (S1), a
**module-level global** (S2), a binding **inside a `FOR EACH` over itself** (S7),
and a `MUT` **captured by a non-escaping `LAMBDA`** (S9). The same holds for the
`String` self-concat `s = s & t` at those sites. And a guard makes the property
permanent: adding a builtin with a self-update form and no in-place lowering
fails the test suite.

"In place, no copy" is measured, not asserted: at each site, running the
self-update `N` times and `2N` times allocates the same number of blocks, up to
the collection's geometric growth (`perf.mfb_alloc.count` from `mfb build --debug`).
A copying lowering allocates once per statement, so its count doubles.

The facts this plan starts from are plan-141's findings,
`planning/plan-141-findings/inplace-audit.md` (§1, §1b, Appendix B): today 10
`collections` overloads have an in-place arm, and they fire only at S1 (and in
records). No site other than S1 is ever in place.

| Letter | What it lands | Effort |
|---|---|---|
| **A** | the census guard (unit + black-box) and a table-driven dispatch every site shares | large |
| **B** | S1 arms: `filter`, `take`, `drop`, `mid`, `distinct` (shrink/compact) | large |
| **C** | S1 arms: `replace`, `transform`, `sort`, `sortBy`, and the 16 `math` element-wise functions | large |
| **D** | S1 arms: `union`, `intersection`, `difference`, `symmetricDifference`, `merge`, `mapValues` | large |
| **E** | the `Exempt` entries, each proven copy-free: `reduce`, `reduceRight`, 6 `compress`, 3 `crypto` | medium |
| **F** | S7: a `FOR EACH` over the binding its body self-updates | medium |
| **G** | S9: a `by_ref` lambda capture | medium |
| **H** | S2: module-level globals (`StoreGlobal`), incl. `FOR EACH` over a global and `s = s & t` | large |
| **I** | lock the guard (no `Pending` left), docs, the full gate | medium |

Letter order is implementation order. Blast radius rises through the letters:
A is behavior-neutral, B–E add arms on the site that already works (S1), and
F–H open new aliasing surfaces (memory safety), so they come last, behind the
tests A–E built.

References:

- `planning/plan-141-findings/inplace-audit.md` — verdicts, gates (G1–G26), probes.
- `planning/completed/plan-121-gate-inventory.md` and `.ai/collections.md`
  §"In-place mutation" — read before adding an arm (the latter's stale claims are
  listed in plan-141 §3.4; letter I corrects them).
- `src/codegen/collection/assign/inplace_dest.rs` (`InPlaceDest`, `InPlaceGate`),
  `src/codegen/collection/assign/builder_inplace_assign.rs` (the arms),
  `src/codegen/engine/control/builder_control.rs:1135-1263` (the `NirOp::Assign`
  dispatch chain).
- `.ai/testing-gates.md:10` (per-phase gate), `:809` (codegen-inspection tests
  must be proven RED by reverting the fix), `.ai/compiler.md:85-86` (acceptance
  plus an execution test).
- `mfb spec language memory-semantics` §14 — value semantics the arms must keep.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-141 is complete and archived | `ls planning/completed/plan-141-mut-inplace-audit.md planning/plan-141-findings/inplace-audit.md` → both exist | MET (2026-09-21, re-run by /follow-plan in the P-142 worktree: both files listed) |
| bug-665 fixed: a global passed to a function that reassigns it no longer dangles | `ls bugs/completed/bug-665-*.md` → exists, **and** `cargo test --test rt_global_argument_reassigned_by_callee` → pass | MET (2026-09-21: `bugs/completed/bug-665-global-passed-to-a-writer-dangles.md` exists; `test result: ok. 7 passed; 0 failed`) |
| bug-666 fixed: `FOR EACH` over a global the body reassigns no longer reads freed memory | `ls bugs/completed/bug-666-*.md` → exists, **and** `cargo test --test rt_for_each_over_reassigned_global` → pass | MET (2026-09-21: `bugs/completed/bug-666-for-each-over-a-reassigned-global.md` exists; `test result: ok. 6 passed; 0 failed`) |
| The release compiler exists for `--ncode` probes | `ls target/release/mfb` → exists | MET (2026-09-21: built in the worktree, `cargo build --release` exit 0) |

Why the two bugs gate the whole plan, not just letter H: letter H makes a
global's block be mutated in place, which is sound only if nothing else holds a
pointer into it. bug-665 (a borrowed parameter) and bug-666 (a loop's saved
pointer) are exactly the two holders that exist today, and today they are
use-after-frees. Their fixes are the aliasing guarantees H relies on. Letter G
has the same shape for `forEach(acc, LAMBDA -> acc = …)`; bug-665's callback
row covers it.

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. **If you stop, report the current status of *all*
> prerequisites.**

## 1. Goal (this letter)

- A unit census test enumerates every overload of every builtin package in the
  registry and fails if one whose return type can equal its first parameter's
  collection type is missing from a new table, `SELF_UPDATE_TABLE`.
- A black-box census test in `tests/guards/` does the same through
  `mfb man`, and fails if an overload has no runtime case in
  `tests/runtime/inplace_self_update/cases.tsv`.
- The `NirOp::Assign` dispatch runs the arms from the table
  (`SELF_UPDATE_ARMS`), not a hand-written `&&` chain, through one entry point
  that takes an `InPlaceDest`. Letters F–H add sites by building a different
  `InPlaceDest`; no arm is written per site.
- A matrix test runs every `Arm` table entry at every *enabled* site and asserts
  the arm fired; at the end of A the only enabled site is S1.

### Non-goals (explicit constraints)

- **Byte-identical codegen for this letter.** The refactor is code motion: every
  arm still declines before it emits (inventory rule `O-order-1`), so iterating a
  table instead of an `&&` chain picks the same arm and emits the same bytes. The
  gate for Phase 2 is `.ncode` byte-identity; see §3.
- No new arm, no new site, no language change, no `mfb man` change.
- Records (`WITH`) stay out of the whole plan (plan-141 §2 is a separate problem).

## 2. Current State

- Arms are chosen by string literal in each `try_inplace_*`
  (`builder_inplace_assign.rs:36, 106, 420, …`; `bulk_append` checks
  `native_builtin_target(target) != Some("append")` at `:1412`). Dispatch is the
  `&&` chain at `builder_control.rs:1166-1263`. No list of arm-capable builtins
  exists anywhere, and no test enumerates the arms (research, 2026-09-20: grep for
  a const table or a test over the chain found none).
- The registry is `pub(crate)` in a bin-only crate (no `src/lib.rs`, no `[lib]` in
  `Cargo.toml`), so an integration test in `tests/` cannot read it. Registry
  census tests live in `#[cfg(test)]` modules: `src/codegen/registry/mod.rs:4302`
  (loops `registry().packages()` at `:4325`) and
  `src/codegen/builtins/collections/mod.rs:442` (asserts 49 members).
- Registry shapes: `registry().packages()` (`registry/mod.rs:1488`) →
  `RegistryPackage::functions()` (`:1151`) → `RegistryFunction.implementations`
  (`:486`) → `Implementation { params, return_type, body, .. }` (`:451`);
  `Parameter.ty: ParameterType` (`:179`); generics are `ParameterType::Var` and
  `ParameterType::Arg(n)` (`src/types.rs:270`, `:290`); `unify` is private to
  `registry/mod.rs` (`:2643`).
- Codegen-inspection precedent: `tests/codegen/codegen_inplace_append_call_result.rs`
  builds with `common::temp_project` + `common::build_ncode` and counts labels;
  `tests/codegen/codegen_inplace_record_field.rs:37` counts stack-slot types. In
  the bin crate, `src/codegen/builtins/tests/inplace.rs` uses
  `crate::testutil::{code_for_src_cached, code_function}`.
- Runtime alloc-count precedent: `tests/runtime/rt_inplace_append_builtin_call.rs`
  (`mfb build --debug`, parses `arena.0.alloc_bytes` from stderr).
- Each `tests/<dir>/*.rs` needs a `[[test]]` stanza (`Cargo.toml:90-108`;
  enforced by `tests/guards/test_targets_registered.rs`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Builtin packages / overloads | 42 / 828 | `python3 census-all.py` (Appendix) → `packages 42 overloads 828` |
| Overloads whose first parameter is `List`/`Map`/`Set` and whose return type is the same type, literally | 59: `collections` 23, `math` 27, `compress` 6, `crypto` 3 | same script → per-package `uniq -c` |
| …of which have an arm today | 10 overloads (`append`×2, `set`×2, `insert`, `prepend`, `removeAt`, `add`, `remove`, `removeKey`) | plan-141 findings §1: rows with S1 = `y` → 10 |
| `collections` self-updatable only when type parameters coincide | 4: `transform` (U=T), `mapValues` (U=V), `reduce`, `reduceRight` (U = List OF T) | plan-141 findings §1 + Correction 2 |
| Self-update-shaped overloads, all packages (unit census) | 63 = 59 literal + 4 generic-only | Phase 1's temporary `print_self_update_shaped` over `registry::self_update_shaped` → `literal 59 generic 4 total 63` |
| Generic-only self-update overloads **outside** `collections` | 0 | same run: the 4 generic-only rows are `collections::mapValues`, `transform`, `reduce`, `reduceRight` — no other package |
| `math` functions behind the 27 overloads | 16: `abs acos asin atan atan2 clamp cos exp log log10 max min pow sin sqrt tan` | `grep '^math::' allpkg.txt \| cut -d'(' -f1 \| sort -u \| wc -l` → 16 |
| In-place arms (`fn try_inplace_*`, non-`STATE`) | 19 | plan-141 findings Appendix B.3 |

### Verified properties

- Every arm declines without emitting (`O-order-1`): read in plan-121-A and
  re-read in plan-141 (Appendix B.3 gate order: every gate precedes the first
  `lower_value`). This is what makes the Phase 2 refactor byte-neutral.
- Arm matching is disjoint by builtin name (`builder_control.rs:1192-1196`
  comment; each arm re-checks `native_builtin_target`), so chain order is
  immaterial. **UNVERIFIED** for the two `append` arms, which share a name and split on
  G11 (element vs list item type): Phase 2 keeps their relative order.

## 3. Design Overview

Three pieces:

1. **`SELF_UPDATE_TABLE`** — new file
   `src/codegen/collection/assign/self_update.rs`. One row per registry function
   that has a self-update-shaped overload: `(qualified_name, SelfUpdate)`, where

   ```rust
   pub(crate) enum SelfUpdate {
       /// In place at every site, by these arm ids (a function may need two,
       /// e.g. `append` single-element and bulk).
       Arm(&'static [ArmId]),
       /// No copy of `x` exists to avoid: the result is not derived from `x`'s
       /// block (e.g. `reduce` builds from `initial`). Reason and the cited
       /// lowering line that proves `x` is only read.
       Exempt { reason: &'static str, proof: &'static str },
       /// Not yet in place; names the plan-142 letter that lands it. Letter I
       /// deletes this variant, so after plan-142 a missing arm cannot compile.
       Pending(&'static str),
   }
   ```

   and `SELF_UPDATE_ARMS: &[(ArmId, ArmFn)]`, the dispatch list. `ArmFn` is
   `fn(&mut CodeBuilder, &SelfUpdateSite, &NirValue) -> Result<bool, String>`.

2. **One dispatch entry, one destination type.** `CodeBuilder::try_inplace_self_update(site, value)`
   iterates `SELF_UPDATE_ARMS` in order and returns on the first `true`. `site`
   is a `SelfUpdateSite` carrying the binding name, its type, the `InPlaceDest`,
   and the live-alias facts the gates read. Phase 2 moves the existing plain-local
   arms onto it (their bodies already use `stack_offset` only through
   `InPlaceDest::Direct`/`dest.block_slot()` or pass it straight to a
   `lower_*_in_place(slot, …)`). Record arms (`try_inplace_record_field_*`) are not
   touched — records are out of scope.

   Letters F–H add sites only by constructing `SelfUpdateSite` differently:
   - S1: `InPlaceDest::Direct { slot }` (today's behavior);
   - S2 (H): a new `InPlaceDest::Global` — load the global's block pointer into a
     scratch frame slot, run the arm on that slot, store the (possibly
     reallocated) pointer back to the global's address, the same open/close shape
     as the `STATE` write-back `close_inplace_dest` (`inplace_dest.rs:609`, O4);
   - S9 (G): a new `InPlaceDest::Ref` — the same, through the reference local's
     parent-slot pointer;
   - S7 (F): no new destination; the loop stops aliasing the binding (design in F).

   So a new arm is automatically an arm at every site, and a site is
   automatically served by every arm. That is the structural half of the guard.

3. **The guards.**
   - `self_update_census_covers_every_registry_overload` (unit, in
     `self_update.rs`): for every `package`, `function`, `implementation` with
     `params[0].ty` a `List`/`Map`/`Set` (or a type variable that can be one) and
     `return_type` unifiable with `params[0].ty` (via the registry's `unify`,
     exposed as `pub(crate) fn self_update_shaped(&Implementation) -> bool` in
     `registry/mod.rs`), assert the function has a `SELF_UPDATE_TABLE` row.
   - `self_update_table_has_no_stale_rows` (unit): every row names a registry
     function with at least one self-update-shaped overload; every `Arm` id is in
     `SELF_UPDATE_ARMS`; every `SELF_UPDATE_ARMS` id is referenced by a row.
   - `every_arm_row_fires_at_every_enabled_site` (unit, in the bin crate so it
     can read the table): for each `Arm` row, a per-row probe snippet (a
     `probe: &'static str` field, e.g. `"collections::filter({X}, isPositive)"`)
     is compiled at each enabled site with `testutil::code_for_src_cached`, and
     the arm's marker stack slot must be present in the function
     (plan-141 Appendix C's marker method: each arm allocates a slot type name
     unique in `src/`). `ENABLED_SITES` starts as `[S1]`; F, G, H each append
     their site.
   - `tests/guards/inplace_self_update_census.rs` (black-box, runs the `mfb`
     binary): the `mfb man` census from the Appendix script, asserting every
     self-update-shaped overload has a line in
     `tests/runtime/inplace_self_update/cases.tsv`. That file drives
     `tests/runtime/rt_inplace_self_update.rs`, which for each case and site
     builds `N` and `2N` self-updates with `--debug` and asserts the alloc count
     does not scale (Pending rows are expected-fail and named as such, so the
     file cannot silently drop them).

   Two censuses because the registry is unreachable from `tests/`: the unit one
   ties the registry to the table and the arms; the black-box one ties the
   documented surface (`mfb man`) to runtime behavior. Either catches a new
   builtin.

**Failure atomicity — the correctness rule every arm in B–D obeys.** `x = op(x, …)`
must leave `x` unchanged if `op` fails, because the assignment never happens
(§14; observable through a function-level `TRAP` handler that reads `x`, or
through a global after the error). So an arm must detect every failure it can
raise **before its first write**: bounds, counts, domain errors (a pre-pass over
the elements, allocation-free), and callback results. A callback that can fail
(`filter`'s predicate, `transform`'s `f`) is called for every element before any
write, with results kept in scratch proportional to what they carry (a bitmap for
`filter`, not a second list). Where results cannot be held without a copy of `x`
(`transform`/`mapValues` with a fallible `f` whose results are variable-width),
see Open Decision 1.

**Byte-identity.** Phase 2 of this letter is provably neutral, so its gate is
byte-identity of every committed `.ncode` golden (`cargo test --test golden`,
which runs `scripts/artifact-gate.sh <mfb> all`). A diff there is a bug in the
refactor: objdump one fixture, fix it. Letters B–H change codegen on purpose;
their gates are behavior (runtime tests + the alloc-scaling check), and the
goldens they are expected to shift are named in each letter.

**Risk.** Correctness risk concentrates in F–H (aliasing). Design uncertainty
concentrates in two places, both scheduled first in their letters: callback
fallibility (B Phase 1 measures whether codegen can know a `FUNC` value is
infallible) and generic-only overloads outside `collections` (A Phase 1 measures them).

Rejected alternatives:

- **Keep the `&&` chain and add a site-specific copy of each arm** (a `try_inplace_global_*`
  family): 4 sites × 36 arms of duplicated gates — the exact drift plan-121-A
  removed.
- **A guard that is only a code-review checklist** — the plan-121 gate inventory
  was one, and plan-141 found it eight changes stale.
- **Black-box census only** — it can check the runtime but not that the table,
  the arms and the registry agree; a stale arm id would pass it.
- **Unit census only** — the user asked for a `tests/` guard, and the runtime
  alloc check is the only test that measures "no copy" rather than "the arm ran".

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. `- [~]` partial. Moot tasks struck through with evidence,
> never deleted. **An unticked box means NOT DONE.**

### Phase 1 — Measure the generic-only population

- [x] Add `pub(crate) fn self_update_shaped(imp: &Implementation) -> bool` to
      `src/codegen/registry/mod.rs` (next to `unify`, `:2643`): true when
      `params[0].ty` is, or can be instantiated to, a `List`/`Map`/`Set` and
      `return_type` unifies with it under one substitution. (Landed after
      `substitute`; it runs its own two-sided unifier `types_coincide`, since `unify`
      is pattern-vs-concrete — see Correction A1. `#[cfg(test)]`: the census is its
      only consumer.)
- [x] A temporary `#[test] fn print_self_update_shaped()` that prints every
      qualified name + signature it returns true for. Record the list and its
      count in this plan's Measured populations (replacing UNMEASURED), split into
      literal vs generic-only. If the generic-only set outside `collections` is
      non-empty, add those functions to the letter that owns their kind (or E if
      exempt) before continuing, and re-check that letter's effort.
      Result (`cargo test --bin mfb print_self_update_shaped -- --nocapture`):
      `literal 59 generic 4 total 63`. Literal: `collections` 23, `math` 27,
      `compress` 6, `crypto` 3. Generic-only: `collections::mapValues`,
      `transform`, `reduce`, `reduceRight` — the 4 already known; none outside
      `collections`, so no letter gains scope.
- [x] Delete the temporary test. (`grep -n print_self_update src/codegen/registry/mod.rs` → empty.)

Acceptance: `cargo test --bin mfb print_self_update_shaped -- --nocapture` →
prints ≥ 63 rows (59 literal + the 4 known `collections` generic-only), and the
literal subset equals the Appendix census's 59 exactly (est. 3 min).
Verified 2026-09-21: 63 rows (59 + 4). The Appendix script re-run on the worktree's
`target/release/mfb` prints `packages 42 overloads 828` and 59 hits; `diff` against
the unit census's 59 literal rows (after rendering `Arg0` as the first parameter's
type) differs only in two rendering artifacts of the comparison script — the nested
`FUNC(T) AS Boolean` in `filter`'s signature and `crypto.Argon2Profile` vs
`crypto::Argon2Profile` — so the two sets name the same 59 overloads.
Commit: e86532663

### Phase 2 — Table-driven dispatch, byte-identical

- [x] Create `src/codegen/collection/assign/self_update.rs` with `ArmId`,
      `SelfUpdate`, `SelfUpdateSite`, `SELF_UPDATE_ARMS`, and
      `CodeBuilder::try_inplace_self_update`. Register the module in
      `src/codegen/collection/assign/mod.rs`. (`SelfUpdate` lands with the table in
      Phase 3, its only consumer.)
- [x] Move the 10 plain-local arms (`append`, `bulk_append`, `set_add`, `set`,
      `remove_key`, `prepend`, `remove_at`, `insert`, `set_remove`, `concat`) to
      the `ArmFn` signature, taking the destination from `SelfUpdateSite`. Keep each
      body's emission identical. All nine `Call` arms now run their container gates
      through one `resolve_self_update(site, …)` (was `resolve_inplace_plain_local`
      for four, hand-copied gates for five); the site carries the binding's type,
      so no arm reads `self.locals` for it.
- [x] Replace the first ten links of the `&&` chain at `builder_control.rs:1166-1215`
      with one `try_inplace_self_update` call; leave the eight
      `try_inplace_record_field_*` links as they are.
- [x] Keep `append` before `bulk_append` in `SELF_UPDATE_ARMS` (the one
      name-shared pair).

Acceptance: codegen is unchanged.
  Check: `cargo build --release && cargo test --test golden` → pass with zero
  `.ncode` diffs (est. 20 min — the artifact gate over every committed golden is
  the only check that sees every arm's emission; a scoped fixture set would miss
  the arms it doesn't exercise). A diff = a refactor bug: objdump one fixture,
  fix, re-run.
  Verified 2026-09-21: `cargo build --release && cargo test --test golden` →
  `artifact-gate [all]: 1473 tests, 1648 build(s), 2078 golden(s) checked, 0 diff(s)`,
  `test result: ok. 1 passed`.
Commit: —

### Phase 3 — The table and the unit guards

- [ ] Fill `SELF_UPDATE_TABLE` with one row per function from Phase 1's list:
      the 10 arm-backed as `Arm`; `filter take drop mid distinct` →
      `Pending("B")`; `replace transform sort sortBy` + the 16 `math` functions →
      `Pending("C")`; `union intersection difference symmetricDifference merge mapValues`
      → `Pending("D")`; `reduce reduceRight` + 6 `compress` + 3 `crypto` →
      `Pending("E")`; plus any Phase 1 additions. Each row carries its `probe` snippet.
- [ ] Add `self_update_census_covers_every_registry_overload` and
      `self_update_table_has_no_stale_rows` to `self_update.rs`.
- [ ] Add `every_arm_row_fires_at_every_enabled_site` with `ENABLED_SITES = [S1]`,
      and per-arm marker slot names on `ArmId` (plan-141 Appendix C lists them).
- [ ] RED proof per `.ai/testing-gates.md:809`: temporarily delete one row
      (`collections::take`) → the census test fails naming it; temporarily remove
      one arm from `SELF_UPDATE_ARMS` → the matrix and stale-row tests fail.
      Restore both.

Acceptance: `cargo test --bin mfb self_update` → 3 tests pass; both RED
experiments recorded here with their failure lines (est. 5 min).
Commit: —

### Phase 4 — The black-box guard and the runtime harness

- [ ] Add `tests/guards/inplace_self_update_census.rs` (+ `[[test]]` stanza): the
      Appendix census in Rust over `mfb man` (the binary under test), asserting
      each self-update-shaped signature appears in
      `tests/runtime/inplace_self_update/cases.tsv`.
- [ ] Add `tests/runtime/inplace_self_update/cases.tsv`: columns
      `signature \t status(arm|exempt|pending:<letter>) \t setup \t statement \t check`
      — one line per overload from Phase 1.
- [ ] Add `tests/runtime/rt_inplace_self_update.rs` (+ stanza): for each `arm`
      line and each enabled site (S1 now), build a program running the statement
      `N = 2000` and `2N` times under `mfb build --debug`, parse
      `perf.mfb_alloc.count`, and assert `count(2N) - count(N) < N / 8`; also run a
      value-semantics check (`LET before = x` taken first is unchanged after).
      `pending:<letter>` lines are asserted to **still copy**
      (`count(2N) - count(N) >= N`), so a letter that lands an arm must flip its
      line — a silent improvement fails too. `exempt` lines run the value check only.
- [ ] RED proof: change one `arm` line to a function that copies (e.g. point
      `append`'s case at a statement that uses `collections::take`) → the alloc
      assertion fails; restore.

Acceptance: `cargo test --test inplace_self_update_census --test rt_inplace_self_update`
→ pass (est. 8 min: ~63 cases × 2 builds; no smaller check measures allocation).
Commit: —

## Validation Plan

- Tests added here: the three unit guards, the black-box census, the runtime
  harness (Phases 3–4).
- Coverage check: the matrix test and the runtime harness enumerate from the table
  and `cases.tsv` — a function missing from both fails the two censuses.
- Runtime proof: `rt_inplace_self_update` at S1 for the 10 existing arms.
- Doc sync: none in this letter (letter I).
- Per-phase gate: `.ai/testing-gates.md:10` — `cargo test --bin mfb` plus the
  scoped checks above. The full gate runs once, in letter I.

## Open Decisions

1. **RESOLVED (user, 2026-09-20): decline only in that case.** A fallible callback
   whose results cannot be held without a copy (`transform`/`mapValues` with a
   `FUNC` that can fail, variable-width `U`, when `x` is readable on the failure
   path): the arm declines to the copying path *only* in that case, which the table
   records as a named gate (`G-atomic`) with its own runtime case proving `x` is
   unchanged after the failure — value semantics outrank in-place. B Phase 1's
   measurement decides how often it applies.
2. **RESOLVED (user, 2026-09-20): a separate audit.** `String → String` builtins
   (`s = strings::trim(s)`, …) are out of plan-142; plan-143
   (`planning/plan-143-string-self-update-audit.md`) audits them the way plan-141
   audited collections, and the fix plan is written from its findings. plan-142
   keeps `s = s & t`.
3. **`compress`/`crypto` as `Exempt`.** Their output is a new byte stream whose
   size is unrelated to the input's; there is no in-place form, and `x` is only
   read. Recommended: `Exempt` with a proof that `x` is not copied (letter E).
   Alternative: treat them as out of scope for the census (weaker: a future
   byte-stream builtin would slip past the guard).
   DECISION: `Exempt` with a proof

## Corrections

- **A1 (Phase 1): `unify` cannot answer the self-update question.** The plan said
  `self_update_shaped` would use the registry's `unify`. `unify(pattern, concrete)`
  binds variables on the pattern side only and treats the concrete side as fixed,
  so `transform(List OF T, …) AS List OF U` (both sides generic) cannot be asked of
  it. `self_update_shaped` runs a small two-sided unifier (`types_coincide`, with an
  occurs check) over one binding map instead. Measured result unchanged in kind: 4
  generic-only overloads, all in `collections`.

## Summary

A is the enabler and the guard: a table every arm is registered in, one dispatch
every site uses, and two censuses (registry-side and `mfb man`-side) that fail on
a new self-update-shaped builtin without a row. It is byte-neutral. The real risk
of the plan is in F–H, where new aliasing surfaces open; they are last, behind the
harness this letter builds.

## Appendix — `mfb man` census across every package

Run 2026-09-20 against `target/release/mfb` built from `b6a10efbc`:
`packages 42 overloads 828`; the 59 literal self-update overloads were
`collections` 23, `math` 27, `compress` 6, `crypto` 3.

```python
import re
import subprocess

M = "target/release/mfb"
top = subprocess.run([M, "man"], capture_output=True, text=True).stdout
pkgs = re.findall(r"^│ ([a-zA-Z]+) +│", top[top.index("Builtin packages"):], re.M)
pkgs = [p for p in pkgs if p != "Package"]
hits, total = [], 0
for pkg in pkgs:
    page = subprocess.run([M, "man", pkg], capture_output=True, text=True).stdout
    if "\nFunctions\n" not in page:
        continue
    funcs = []
    for f in re.findall(rf"│ {pkg}::([a-zA-Z0-9_]+)", page[page.index("\nFunctions\n"):]):
        if f not in funcs:
            funcs.append(f)
    for f in funcs:
        fp = subprocess.run([M, "man", pkg, f], capture_output=True, text=True).stdout
        lines = fp.splitlines()
        try:
            i = next(k for k, l in enumerate(lines) if l.strip() in ("Overloads", "Declaration"))
        except StopIteration:
            continue
        j, block = i + 2, []
        while j < len(lines) and not (lines[j].strip() and j + 1 < len(lines)
                                      and set(lines[j + 1].strip()) == {"─"}):
            block.append(lines[j])
            j += 1
        text = " ".join(l.strip() for l in block)
        for s in re.findall(rf"`({pkg}::[^`]+)`", text):
            s = re.sub(r"\s+", " ", s)
            m = re.match(r"\w+::\w+\((.*)\) AS (.*)$", s)
            if not m:
                continue
            total += 1
            params, ret = m.group(1), m.group(2)
            first = re.match(r"\[?\w+ AS (.*?)(?:, \[?\w+ AS |\]?$)", params)
            ft = first.group(1).rstrip("]") if first else ""
            if re.match(r"(List|Set|Map)\b", ft) and ft == ret:
                hits.append(s)
print("packages", len(pkgs), "overloads", total)
for h in hits:
    print(h)
```
