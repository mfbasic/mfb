# plan-147-F: Record accumulators, the doc and spec sync, and the full gate

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-147-E

The census's record accumulators are threaded the same way as the collection ones
(plan-147-A §2.2):

- `packages/json_schema/src/index.mfb:195` `walkSchema(…, acc AS Acc, depth)`: an `Acc`
  holding three `Map OF String TO String`, threaded as
  `state = walkSchema(…, state, depth + 1)`;
- `examples/browser/dom/src/lib.mfb:300` `gatherSpecs(…, acc AS SpecWalk)`;
- `json_schema`'s `addError`/`absorbChild`, and `zip`/`tar` `addText`, which return
  `WITH state { … }`.

This letter extends hand-over to records whose fields have no resources. It adds
the field form of S11, `RETURN WITH r { f := OP(r.f, …) }`, on an owned local or
owned parameter, using plan-145's field seam, which is a prerequisite of the whole
plan. Then it syncs every doc the plan obligates and runs the project's full gate
once.

References: plan-147-A (goal, prerequisites, §2.3 — the §14.6 amendment text is
there); plan-147-B/E (the sites); plan-145 (the field seam and its `InPlaceDest::Inlined`
arms); `.ai/spec-content.md` (as-is-at-HEAD rule, citations); `.ai/testing-gates.md`
(the full gate).

## Prerequisites

See plan-147-A. plan-147-E must be complete: `ls planning/plan-147-E-* 2>/dev/null` →
no matches.

## 1. Goal

- A record argument with an owned field accumulator is handed over and updated in
  place. A new `rt_owned_argument.rs` case, `record-field`, is flat in N:
  `st = addItem(st, i)` with `addItem` returning `WITH s { items := collections::append(s.items, i) }`.
- The field form of S11 fires for every plan-145 field arm. Plan-145's field harness
  gains the site, if it has a site axis, or a matching `rt_owned_argument.rs` case per
  field kind, if it does not. Which one is decided in Phase 1 from plan-145's landed
  harness.
- The docs match the code (Phase 3), and the full gate is green (Phase 4).

### Non-goals

- Records that contain a resource, or a thread. H2 still refuses those (row S8).
- Every plan-147-A non-goal.

## 2. Current State

(After E. Plan-145 is complete: re-read its archived letters for the field seam's
final shape before Phase 1. Anything this section assumes and it contradicts goes in
Corrections.)

- H2 (C §3.1) admits `List`/`Map`/`Set`/`String` only.
- B's S11 dispatch matches `is_self_update_call` on a `Local` only. The field form
  `WITH r { f := OP(r.f, …) }` is plan-145's field self-update shape. Plan-145 lowers it
  at the field sites it enabled (S3–S7, S9, S10, `STATE`) through
  `InPlaceDest::Inlined{block_slot, field_index, write_back}`.
- Docs that describe today's behaviour and will be wrong after plan-147:
  - `.ai/collections.md` §"An accumulator must not be threaded through a helper"
    (committed `2f55eb184`);
  - `src/docs/spec/memory/05_collections.md` `### Self-updates` (`:492`, the site list);
  - `src/docs/spec/memory/06_native-calling-convention.md` §"Argument Passing" (it
    has no owned-parameter or variant-symbol wording);
  - `src/docs/spec/language/14_memory-semantics.md` §14.6 (the `MUT`-only sentence,
    plan-147-A §2.3 row S5).
- No `mfb man` page claims a helper copies. Checked 2026-09-21:
  `rg -n -i 'through a (helper|function)|helper function|copies the whole' src/codegen/builtins/collections src/codegen/builtins/strings src/docs/man`
  → only the package intro "List, Map, and Set helper functions" and `types/{list,map}.md`
  "Collection helper functions such as…". Neither describes copying. So no man
  change is expected. Phase 3 re-runs the check.

## 3. Design

1. **H2 for records.** A record type whose fields, recursively, are scalars, `String`s,
   collections of H2 types, or H2 records, and which `type_contains_resource` rejects
   nothing in. It is also a consumable parameter (C §3.2 P3), extended with the
   field form below.
2. **S11, field form.** In B's dispatch, also match
   `RETURN WITH r { f := OP(r.f, …) }` on an owned local or owned parameter `r` at
   its last use. Build plan-145's field `SelfUpdateSite` (its `Inlined` destination on
   `r`'s block), run the arm, then take B's existing move of `r`. A `WITH` that
   updates several fields goes through plan-145's two-field path (its S10) unchanged.
3. **The docs**, each describing the code as it is after this letter:
   - **`.ai/collections.md`:** rewrite the accumulator section. A helper is in place when
     the caller's argument is at its last use and nothing reads it on a failure path;
     list the refusals (H1–H6). Say the `/tmp/owned` numbers were re-measured (Phase 4)
     and cite `analysis/handover.rs`.
   - **`05_collections.md` `### Self-updates`:** add S11 (`RETURN`, local and field form)
     and S12 (owned parameter) to the site list, with `[[…]]` citations to the dispatch
     functions B and E added.
   - **`06_native-calling-convention.md`:** a subsection "Owned-parameter variants". It
     covers the `$own<mask>` symbol, when a caller calls one, the null-store at the
     call, and that the base symbol and its ABI are unchanged. Cite
     `collect_handover_args`, the variant lowering, and the null-store.
   - **Language spec §14.6:** replace the `MUT`-only sentence with plan-147-A §2.3's
     amendment text. No other language-spec text changes: §6 and §14.1–§14.3 already
     permit this, and plan-147-A §2.3 records why.

**Correctness risk:** step 2 composes two landed mechanisms, plan-145's field arms
and B's move. The risk is in the composition's error path: an arm that fails after
another field was already updated. Plan-145's failure-atomicity rule covers each
arm, and its S10 covers the two-field `WITH`. Phase 2 runs plan-147-A's semantics
fixture, extended with a record case.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work; `- [~]` for partial with one line on what remains; moot tasks are
> struck through with evidence, never deleted; fill `Commit:` when a phase lands.
> **An unticked box means NOT DONE.**

### Phase 1 — Read plan-145's landed seam

- [x] Read plan-145's landed seam (nine letters, `planning/completed/plan-145-A..I`).

      **Entry points.** `CodeBuilder::field_self_update_site(container, value)`
      (`builder_control.rs:887`) recognises a single-update `WITH` over its owner and
      answers `(FieldSite, field type, the update's value)`; `peel_field_path`
      (`:913`) descends inlined record levels. The caller builds a `SelfUpdateSite`
      whose `dest` is `InPlaceDest::Inlined { block_slot, field_index, path,
      write_back }` (or `StateField`/`GlobalField`) and calls the ordinary
      `try_inplace_self_update`. `FieldSite` and `FieldContainer` are
      `self_update.rs:136`/`:161`; the container is `Record { local }`,
      `State { resource }` or `Global { name }`.

      **Harness site axis: YES, and it is two-sided.** The matrix has
      `FIELD_SITES` (`self_update.rs`, S3–S10 and T1–T8) with a `field_source` renderer
      per site; the runtime harness has its own `FieldSite` enum
      (`rt_inplace_self_update.rs:1218`), its own `FIELD_SITES` (`:1256`), and a
      per-(line, site) expectation table `inplace_self_update/field_expect.tsv`.

      **Decision:** add the site. The field form of S11 becomes a new field site,
      `S11F`, on BOTH axes — a `RETURN WITH r { f := OP(r.f, …) }` probe on an owned
      parameter, which is also the `record-field` shape the Goal's first bullet
      names. That reuses plan-145's per-arm coverage instead of hand-writing one
      `rt_owned_argument.rs` case per field kind, and it puts the new site under the
      same `field_expect.tsv` ledger every other field site already answers to.

Acceptance: the decision is recorded with the file and symbol it rests on.
  Check: `rg -n '\*\*Decision:' planning/plan-147-F-records-docs-and-gate.md` →
  **one line** (line 134, 2026-09-23), resting on
  `builder_control.rs:887 field_self_update_site` and
  `rt_inplace_self_update.rs:1256 FIELD_SITES`. (The plan's own wording, `rg -n
  'Decision:'`, matches this Check line too, so it can never return one; the
  pattern is anchored to the bolded decision itself.)
Commit: 2d502e735

### Phase 2 — Records

- [ ] `analysis/handover.rs`: H2 and P3 for records (§3 step 1), with unit rows: record
      with collection fields approved; record with a `RES` field refused.
- [ ] `builder_exits.rs`: the S11 field form (§3 step 2).
- [ ] `rt_owned_argument.rs`: `record-field`, plus the per-field-kind cases or the site
      axis from Phase 1.
- [ ] Semantics fixture: add `record-trap-reads-old` (a handler reads the record after a
      failing field helper). Regenerate only that case's `.run` lines and review them by
      hand.

Acceptance: `record-field` is flat in N. The fixture's new case shows the old record.
Every other line of the fixture is unchanged.
  Check 1: `cargo test --bin mfb handover && cargo test --test rt_owned_argument` → all
  passed (est. 6 min).
  Check 2: `bash scripts/test-accept.sh target/release/mfb /tmp/owned-accept 'owned-argument-semantics*'`
  → the only diff is the new case, which is then accepted into the golden (est. 2 min).
Commit: —

### Phase 3 — Docs and spec

- [ ] The four docs in §3 step 3.
- [ ] Re-run the man check from §2. Any hit that now describes wrong behaviour is fixed
      at its registry descriptor (`.ai/man-content.md` rules).

Acceptance: the spec builds, and its tests and citations pass.
  Check: `cargo build --release && cargo test --bin mfb spec && bash scripts/spec-census.sh --citations`
  → passed, and 0 dangling citations in the four edited files (est. 6 min).
Commit: —

### Phase 4 — Full gate (run once)

- [ ] `cargo test --no-fail-fast` (`.ai/testing-gates.md`, the full-suite gate).
- [ ] `bash scripts/artifact-gate.sh target/release/mfb all`. Diffs are expected
      only where letters B, D, E and F named an approved site, and each is already listed
      in that letter's Corrections. Any other diff is a bug.
- [ ] `bash scripts/test-accept.sh target/debug/mfb target/accept-actual`. Use a copy of
      the binary, so the concurrent `cargo test` cannot rebuild it mid-run
      (plan-142-I).
- [ ] Re-run `/tmp/owned` (plan-147-A §2.2) and record every row next to its baseline.
- [ ] Archive plan-147-A…F to `planning/completed/`.

Acceptance: all three commands are green, and the timings are recorded.
  Check: the three commands above (est. 60 min — the full gate, required once per
  `.ai/testing-gates.md`; plan-142-I measured this set).
Commit: —

## Validation Plan

- Tests: `record-field`, the per-field cases, the new semantics case.
- Coverage check: Phase 4's gate lists every moved golden against a named site.
- Runtime proof: the `/tmp/owned` table, before and after.
- Doc sync: the four docs in §3 step 3; `mfb man` unchanged (re-checked).
- Final gate: Phase 4.

## Open Decisions

- Should §14.6's amendment also name the owned-variant mechanism? **Recommended: no.**
  The language spec states the rule, "one owner, not read again". The mechanism
  belongs in the memory spec's calling-convention section, which is where §3 step 3
  puts it.

## Corrections

## Summary

F is composition and bookkeeping: plan-145's field arms, B's move, and C's
analysis widened to records. The doc changes are where plan-147's semantics claim
becomes normative text, so the §14.6 wording is the part to review most carefully.
