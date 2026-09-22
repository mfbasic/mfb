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

- [ ] Delete `FIELD_PENDING` and `copy:` handling. Delete `FieldReach::None` if
      no arm uses it. If one does (the `String` arms, Open Decision 1), keep it
      and require the arm's rows to be `deferred:`.
- [ ] Run the drill, and record the failure lines.

Acceptance: `cargo test --bin mfb self_update && cargo test --test inplace_self_update_census`
→ pass; drill lines recorded (est. 15 min).
Commit:

### Phase 2: Docs (findings §3.4)

- [ ] `builder_control.rs`, the `NirOp::StateAssign` comment ("`append` … is
      currently the only operation dispatched"): rewrite it to name the field seam
      (§3.4 item 1). B may already have removed the comment with the chain. If so,
      record that.
- [ ] `inplace_dest.rs`, `resolve_inplace_record_field`'s doc comment: deleted
      in B. Confirm with `grep -n 'last-inlined \`List\`' src -r` → 0 (§3.4
      item 2).
- [ ] `.ai/collections.md`:
  - §"In-place mutation" (`:21-25`): records and `STATE` fields are now sites
    of the same seam (item 4);
  - §"A collection inlined in a record" and §"The third container": replace
    "seven mutating operations" with the reallocation classes (`NoRealloc`,
    `Realloc`) and the table of arms (item 3);
  - document the scalar-field stores for both containers, the fixed-size
    overwrite, nested paths, the global holder, and the loop copy.
- [ ] `planning/plan-141-findings/inplace-audit.md` is history. Add a one-line
      pointer at its top to `self_update.rs` and to plan-144's findings §3.4
      item 5. Do not edit the body.
- [ ] `src/docs/spec/memory/05_collections.md` *Self-updates* section
      (plan-142-I): add the field sites, the scalar store, the mixed-`WITH`
      ordering rule, and the `Rebuild` kinds, with `[[…]]` citations. Gate:
      `cargo test --bin mfb spec`.
- [ ] `mfb man variable` §"A handle can carry its own data: STATE" (§3.4 item 7):
      the semantics are unchanged. Add one sentence on which field kinds update
      without rebuilding the payload. Check with
      `scripts/man-census.sh --memory-scope` → 0 unclassified.

Acceptance: `cargo test --bin mfb spec` passes, and
`scripts/man-census.sh --memory-scope` → 0 unclassified hits (est. 10 min).
Commit:

### Phase 3: Full gate (run once)

- [ ] `cargo test --no-fail-fast` (`.ai/testing-gates.md:423`).
- [ ] `scripts/artifact-gate.sh target/release/mfb all` (`.ai/testing-gates.md:11`).
- [ ] `scripts/test-accept.sh target/debug/mfb target/accept-actual` (`.ai/compiler.md:85`).
- [ ] `rt_inplace_self_update` over every site, unfiltered: 79 lines × 19 sites.
- [ ] Re-run plan-144's reading-vs-dump scripts (`fill_rec.py`, `fill_state.py`,
      `summary.py`, findings Appendix C) against the final compiler. Record the
      new §3.3 counts beside the findings' (10/2,415 record, 438/2,760 `STATE`).

Acceptance: all green; counts recorded (est. 90 min: the full gate, required once
by `.ai/testing-gates.md`, plus the one unfiltered harness run).
Commit:

## Corrections

## Summary

Deleting `FIELD_PENDING` turns the field census into a test obligation, and the
drill shows the guard fires. Then the docs and the one full gate.
