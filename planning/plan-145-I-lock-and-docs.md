# plan-145-I: Lock the field guard, sync the docs, run the full gate

Last updated: 2026-09-21
Effort: medium (1h–2h)
Depends on: plan-145-H

Prerequisites: see plan-145-A.

After H, every `FIELD_PENDING` entry is gone, and every `field_expect.tsv` and
`field_kinds.tsv` line is `arm`, `arm+value`, `rebuild:<reason>`,
`deferred:<plan>` or `na:<reason>`. This letter makes that permanent, corrects
every contradiction the findings list (§3.4), and runs the full gate once.

## 1. Goal

- `FIELD_PENDING` and the harness's `copy:<letter>` status no longer exist. A
  `copy:` line panics ("a field self-update needs an in-place lowering, a
  `Rebuild` row with its proof, or a deferral to a named plan").
- **New-kind drill.** Add a throwaway package record type with a `Float` field to
  the working tree, and confirm both field-kind censuses fail naming it. Add a
  throwaway `SELF_UPDATE_ARMS` entry with `FieldReach::None`, and confirm the
  matrix fails naming its field sites. Record both failure lines here, then remove
  the throwaway code (`git status` clean).
- The docs match the code.

## Phases

### Phase 1: Lock

- [x] Delete `FIELD_PENDING` and `copy:` handling. Delete `FieldReach::None` if
      no arm uses it. If one does (the `String` arms, Open Decision 1), keep it
      and require the arm's rows to be `deferred:`.
      `FIELD_PENDING` deleted, and the matrix requires every pair not in
      `FIELD_NEVER` to fire; the harness's `FieldExpect::Copy` deleted, and a
      `copy:` line panics "a field self-update needs an in-place lowering, a
      `Rebuild` row with its proof, or a deferral to a named plan".
      `FieldReach::None` stays (the `String` concat); the new
      `an_arm_with_no_field_reach_is_deferred_at_every_field_site` requires such
      an arm in `FIELD_NEVER` at every field site and every `field_expect.tsv`
      line of its rows `deferred:` (or `na:`).
- [x] Run the drill, and record the failure lines.
      A throwaway `color::DrillProbe` record (one `Float` field) in the `color`
      package: `cargo test --bin mfb field_kind_census` → "color.DrillProbe
      (InlinedFixed) has no FIELD_KIND_TABLE row — classify it: `Arm(letter)`,
      `Rebuild { reason, proof }` or `Deferred(plan)`"; `MFB_TEST_EXE=target/debug/mfb
      cargo test --test inplace_self_update_census` → "package record type(s)
      documented by `mfb man` with no line in …/field_kinds.tsv — regenerate it
      with field_kinds_gen.py and classify each: color::DrillProbe". A
      `FieldReach::None` arm (Correction I1): `cargo test --bin mfb self_update` →
      "collections::filter at S3: no probe fired Filter" (and at every field
      site), "Filter has FieldReach::None but FIELD_NEVER does not list it at S3",
      and "Filter has FieldReach::None, but field_expect.tsv says `collections::filter(…)
      S3 arm` — a field self-update needs an in-place lowering, …". Both removed:
      `git diff src/codegen/builtins/color/mod.rs` → empty, and no `Filter`
      change in `self_update.rs`.

Acceptance: `cargo test --bin mfb self_update && cargo test --test inplace_self_update_census`
→ pass; drill lines recorded (est. 15 min).
`cargo test --bin mfb self_update` → "7 passed"; the census guard is in the
full gate (Phase 3).
Commit: `096acb8bd`

### Phase 2: Docs (findings §3.4)

- [x] `builder_control.rs`, the `NirOp::StateAssign` comment ("`append` … is
      currently the only operation dispatched"): rewrite it to name the field seam
      (§3.4 item 1). B may already have removed the comment with the chain. If so,
      record that.
      B removed it: `grep -rn "currently the only operation dispatched" src/codegen`
      → 0; the `StateAssign` arm's comments name the field seam
      (`field_self_update_site`).
- [x] `inplace_dest.rs`, `resolve_inplace_record_field`'s doc comment: deleted
      in B. Confirm with `grep -n 'last-inlined \`List\`' src -r` → 0 (§3.4
      item 2).
      `grep -rn "fn resolve_inplace_record_field" src` → 0 (the function and its
      doc are gone). The phrase grep → 1, and it is `lower_field_append`'s own doc
      ("into the last-inlined `List`, growing the owner's block") — accurate: an
      `append` grows, so it needs the last-inlined field (Correction I2).
- [x] `.ai/collections.md`:
  - §"In-place mutation" (`:21-25`): records and `STATE` fields are now sites
    of the same seam (item 4);
  - §"A collection inlined in a record" and §"The third container": replace
    "seven mutating operations" with the reallocation classes (`NoRealloc`,
    `Realloc`) and the table of arms (item 3);
  - document the scalar-field stores for both containers, the fixed-size
    overwrite, nested paths, the global holder, and the loop copy.
  All three: the section title and intro name the field sites; the
  `InPlaceDest` list gains `WriteBack` and the unopened field destinations; the
  table is `FieldReach`'s three classes with their arms; a "Field sites beyond
  the arms" entry covers the store routine, the mixed `WITH`, nested paths, the
  global and the by-ref owner; the `FOR EACH` rule notes the loop copy. `grep -n
  seven .ai/collections.md` → 0.
- [x] ~~`planning/plan-141-findings/inplace-audit.md` is history. Add a one-line
      pointer at its top to `self_update.rs` and to plan-144's findings §3.4
      item 5. Do not edit the body.~~ — moot: the file no longer exists; it was
      removed in `50784e853` ("planning: add plan-145 … remove
      plan-141-findings/inplace-audit.md"), so there is no top to annotate.
- [x] `src/docs/spec/memory/05_collections.md` *Self-updates* section
      (plan-142-I): add the field sites, the scalar store, the mixed-`WITH`
      ordering rule, and the `Rebuild` kinds, with `[[…]]` citations. Gate:
      `cargo test --bin mfb spec`.
      A field paragraph and four rules (stored scalars and fixed-size fields, one
      collection beside scalars with the ordering rule, growth needs the last
      field, rebuilt by design), each cited. `cargo test --bin mfb spec` → "43
      passed".
- [x] `mfb man variable` §"A handle can carry its own data: STATE" (§3.4 item 7):
      the semantics are unchanged. Add one sentence on which field kinds update
      without rebuilding the payload. Check with
      `scripts/man-census.sh --memory-scope` → 0 unclassified.
      `src/docs/man/variable/package.md`: scalars, builtin-changed collections and
      fixed-size records update where they lie; a value-sized field (a `String`)
      is rebuilt. `scripts/man-census.sh --memory-scope` → "unclassified
      memory-vocabulary hits: 0".

Acceptance: `cargo test --bin mfb spec` passes, and
`scripts/man-census.sh --memory-scope` → 0 unclassified hits (est. 10 min).
Both above.
Commit: `096acb8bd`

### Phase 3: Full gate (run once)

- [x] `cargo test --no-fail-fast` (`.ai/testing-gates.md:423`).
      On the tree with main merged in (`e07ad2462`, then `3e542ca19`): `exit 101` over 221 test targets — 5,965 passed, 2 failed, both in `codegen_inplace_record_field`: two plan-121-C declines that plan-145-C/-D replaced by design (plan-145-C Correction C6, plan-145-D Correction D8). Corrected in `0cd7b2687` (each checked against the four-question rule: history, protected behavior, dependents, proof); `cargo test --test codegen_inplace_record_field` → "10 passed". The `--bin mfb` target: "4293 passed; 0 failed; 1 ignored".
- [x] `scripts/artifact-gate.sh target/release/mfb all` (`.ai/testing-gates.md:11`).
      On the merged tree: `artifact-gate [all]: 1485 tests, 1660 build(s), 2098 golden(s) checked, 0 diff(s)`.
- [x] `scripts/test-accept.sh target/debug/mfb target/accept-actual` (`.ai/compiler.md:85`).
      With a copy of that `target/debug/mfb` (so the concurrent `cargo test` could
      not rebuild it mid-run, as plan-142-I did): `acceptance tests passed (1511 test(s) ran)`.
- [x] `rt_inplace_self_update` over every site, unfiltered: 79 lines × 19 sites.
      Inside the full gate (no site filter runs every site): `rt_inplace_self_update` → "2 passed" (3778.24 s) — every `cases.tsv` line at the four plain sites and every line and kind at the 15 field sites against the final `field_expect.tsv`/`field_kinds.tsv` (no `copy:` left).
- [x] Re-run plan-144's reading-vs-dump scripts (`fill_rec.py`, `fill_state.py`,
      `summary.py`, findings Appendix C) against the final compiler. Record the
      new §3.3 counts beside the findings' (10/2,415 record, 438/2,760 `STATE`).
      The probes (`/tmp/p145-probes`, plan-144's `rec` and `state` programs)
      rebuilt with the final compiler (at `096acb8bd`), read with `markers.py`
      (extended with plan-145's store slots, `FS`) and counted from the dump by
      `count_dump.py` — the reading scripts encode plan-144's pre-plan-145 rules,
      so the dump is the authority (Correction I3). **Record sites: 1,537 of 2,415
      in place** (plan-144: 10) — S3 240, S4 249, S5 249, S6 249, S7 52, S9 249,
      S10 249. **`STATE` sites: 1,770 of 2,760** (plan-144: 438) — T1 239, T2 248,
      T3 239, T4 248, T5 248, T6 248, T7 52, T8 248. The rest are the `String`
      rows (`deferred:string`), the value-sized kinds (`rebuild:size-varies`) and
      the `n/a` cells (S7/T7 over a non-collection, the `r.prop = v` row, `json::Json`
      as a `STATE` type). The four `STATE` probe callees with a `json::Json`
      payload parameter no longer compile (bug-674's check) and were dropped from
      the source; plan-144 already counted them `n/a`.

Acceptance: all green; counts recorded (est. 90 min: the full gate, required once
by `.ai/testing-gates.md`, plus the one unfiltered harness run).
All green (above); counts recorded.
Commit: `(recorded in the next commit)`

## Corrections

- **I1 — the `FieldReach::None` drill flips an existing arm.** A brand-new
  `SELF_UPDATE_ARMS` entry needs a new `ArmId` with markers and a table row, so the
  drill set `filter`'s reach to `None` instead — the matrix and the new deferral
  test both named every field site — and set it back.
- **I3 — the plan-144 counts come from the dump.** `fill_rec.py`/`fill_state.py`
  predict each cell from plan-144's code reading and check it against the markers;
  that reading is the pre-plan-145 lowering, so re-running them would report
  disagreements, not counts. `count_dump.py` counts the same cells (every overload
  in place, no copy path) from the rebuilt probes' markers alone.
- **I4 — main was merged before the gate, not after.** main had advanced (bug-484,
  plan-147's plan, a `.ai/collections.md` line); merging first let the full gate
  run once, on the merged tree.
- **I2 — the `last-inlined \`List\`` grep finds one accurate doc.** The stale
  `resolve_inplace_record_field` comment is gone with its function; the one hit
  left is `lower_field_append`'s, which is true.

## Summary

Deleting `FIELD_PENDING` turns the field census into a test obligation, and the
drill shows the guard fires. Then the docs and the one full gate.
