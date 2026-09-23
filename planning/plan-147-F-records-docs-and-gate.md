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

- [x] `analysis/handover.rs`: H2 admits a record when every field is admissible,
      recursively (`handover_type_within`, with a `seen` set so a self-referential
      record cannot recurse forever, and scalars accepted because they live in the
      record's own block). P3 counts `RETURN WITH r { f := OP(r.f, …) }` as a
      consuming use, and the argument-position rule accepts the field form too.
      Unit rows: `a_record_is_handed_over_only_when_every_field_is` — a record of a
      `List` and an `Integer` is approved, a record with a `RES` field is refused
      (row S8, via `type_contains_resource`).
- [x] `builder_exits.rs`: `try_returned_field_self_update` builds plan-145's field
      `SelfUpdateSite` (`InPlaceDest::Inlined` on the owner's block) via the now
      `pub(crate)` `field_self_update_site`, runs the arm, and then takes letter B's
      existing move — the same "S1, then the move" reduction, one level down.
      `ops_hold_self_update`'s `Return` arm also matches the field form, so a
      function whose only self-update is `RETURN WITH r { … }` reserves the scratch
      its arms need.
- [x] `rt_owned_argument.rs`: `record-field` added and flat — **alloc_calls 6004 → 15
      at N = 2000**, slope 6000 → 2, with `addItem$own1` in the emitted code. The site
      axis from Phase 1 is added to the **matrix** as `Site::S11F`; see Corrections
      for why the runtime axis is served by this case rather than by a second
      `field_expect.tsv` column.
- [x] Semantics fixture: `record-trap-reads-old` added. The handler reads the record
      after a failing field helper and sees it **exactly as it was**:
      `record-trap-reads-old items=2 seen=7 code=13`, and `RECOVER a` yields the old
      record (`b=2`). The `build.log` diff against the previous golden is exactly
      those two lines — every other printed value is unchanged — and the goldens were
      then accepted.

Acceptance: `record-field` is flat in N. The fixture's new case shows the old record.
Every other line of the fixture is unchanged.
  Check 1: `cargo test --bin mfb handover` → **`ok. 7 passed; 0 failed`**;
  `cargo test --test rt_owned_argument` → **`ok. 11 passed; 0 failed; 0 ignored`**
  (2026-09-23).
  Check 2: `bash scripts/test-accept.sh target/release/mfb /tmp/owned-accept 'owned-argument-semantics*'`
  → the only `build.log` diff was the two new lines; goldens accepted, and the
  fixture now reports **`acceptance tests passed (1 test(s) ran)`**.
  Check 3 (the Goal's second bullet): `cargo test --bin mfb
  every_arm_row_fires_at_every_enabled_site` → **`ok. 1 passed; 0 failed`** (154.63 s)
  with `Site::S11F` in `FIELD_SITES` — every field arm fires at the field form of S11.
Commit: 7da7b8417

### Phase 3 — Docs and spec

- [x] The four docs in §3 step 3:
      - **`.ai/collections.md`** — the accumulator section is rewritten and retitled
        ("An accumulator threaded through a helper is handed over, not copied"). It
        names the three sites (S11, S12, S11F), the `<base>$own<mask>` variant and the
        null-store, lists the refusals H1—H6, cites `analysis/handover.rs`, carries the
        re-measured `/tmp/owned` table, and keeps `String` as the documented exception
        with its bug-560 reason.
      - **`05_collections.md` `### Self-updates`** — the site list gains the `RETURN`
        forms (local, owned parameter, and the record field form), with `[[…]]` citations
        to `try_returned_self_update` and `try_returned_field_self_update`, and the
        `String`-must-be-tight exception.
      - **`06_native-calling-convention.md`** — a new "Owned-parameter variants"
        subsection: the `$own<mask>` symbol, what an owned parameter is inside it, the
        null-store's ordering and why it is load-bearing, and that the base symbol,
        arity and ABI are unchanged.
      - **Language spec §14.6** — the `MUT`-only sentence replaced with plan-147-A
        §2.3's amendment text, verbatim.
- [x] Re-run the man check from §2 — **no man change needed**, as §2 predicted. The
      `rg` returns the package intro ("List, Map, and Set helper functions"),
      `types/{list,map}.md`'s "Collection helper functions such as…", and
      `func_transform.rs`'s "through a function" (which describes `transform`'s
      callback, not copying). None of them describes a helper copying.

Acceptance: the spec builds, and its tests and citations pass.
  Check: `cargo build --release && cargo test --bin mfb spec && bash scripts/spec-census.sh --citations`
  → builds clean; **`ok. 43 passed; 0 failed`**; citations
  **`TOTAL unique=1664 … MISS-PATH 0 / MISS-LINE 0 / MISS-SYMBOL 0`** (2026-09-23).
Commit: 8bc3b7d33

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

- **The S11 field form reads its owner TWICE, so P3 cannot ask "read exactly
  once".** `WITH r { f := OP(r.f, …) }` reads `r` as the `WITH`'s base and again as the
  field's source, so `read_count(reads, Place::Local("r")) == 2` and P3 refused every
  record helper — `record-field` measured 6004 → 12004, untouched. Both reads belong to
  the one statement that consumes `r`, which the shape itself guarantees, so P3 skips
  the count for the field form and keeps the liveness check. With that: 6004 → **15**.

- **S11F is added to the MATRIX axis only; the runtime axis is served by
  `record-field`.** Phase 1's decision said "both axes". The matrix half is what the
  Goal's second bullet actually states — *the field form of S11 fires for every
  plan-145 field arm* — and `Site::S11F` in `FIELD_SITES` asserts exactly that, for all
  66 arm rows, in both directions (`RETURN_NEVER` and plan-145's own `FIELD_NEVER`
  exclusions apply there unchanged, which is why `ALL_FIELD_SITES` gained the code).

  The runtime half would mean a second `field_expect.tsv` column: **1,939 rows today,
  one per (line, site)**, so ~130 new rows whose expected outcome is not knowable in
  advance — each would have to be measured, and each measurement is a program build.
  That is a large mechanical exercise which would restate, per arm, what the matrix
  already asserts per arm; the allocation property it would add is what `record-field`
  measures directly (6004 → 15). Recorded here rather than done silently: if the
  per-arm ALLOCATION behaviour at S11F is ever wanted, the column is the way to get
  it, and `field_expect_gen.py` is where it would start.

- **S11F inherits S11's `String` exclusions.** The first matrix run failed 27 rows at
  S11F, every one a `String` arm — the same tightness reason letter B measured (a
  `String` block must be tight to leave its frame). `RETURN_NEVER` now covers
  `Site::S11F` alongside `Return` and `OwnedParam`. The 28th, `StrIdentity`, was
  already excluded at every field site by plan-145's own `deferred:string` row; it
  needed only the new site code in `ALL_FIELD_SITES`.

## Summary

F is composition and bookkeeping: plan-145's field arms, B's move, and C's
analysis widened to records. The doc changes are where plan-147's semantics claim
becomes normative text, so the §14.6 wording is the part to review most carefully.
