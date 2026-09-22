# plan-146-H: Lock the guard, sync the docs, run the full gate

Last updated: 2026-09-21
Effort: medium (1h–2h)
Depends on: plan-146-G

Prerequisites: see plan-146-A.

After G, every `String` row of `SELF_UPDATE_TABLE` is `Arm` or `Exempt`, and the
only `Deferred` rows are the 24 `AttributedString` forms (plan-146-A Open Decision
1). This letter deletes `SelfUpdate::Pending` and the harness's `pending:` status
again, as plan-142-I did, so a new `String` builtin with a self-update form cannot
be registered without an arm or a proven exemption. Then it corrects the docs
findings §3.6 lists and runs the project's full gate once.

## 1. Goal

- `SelfUpdate::Pending` does not exist. `cases.tsv` has no `pending:` line, and the
  harness rejects one. `Deferred` stays, for the follow-up plan's rows.
- A new-builtin drill proves the guard for `String`: a throwaway
  `strings::probeSelf(value AS String) AS String` makes both censuses fail naming
  it. The failure lines are recorded here and the change is discarded.
- The docs match the code (Phase 2).
- The full gate is green.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work. `- [~]` partial. Moot tasks struck through with evidence, never
> deleted. **An unticked box means NOT DONE.**

### Phase 1: Lock

- [ ] Delete `SelfUpdate::Pending` and the harness's `pending:` handling. Restore
      the `cases.tsv` header's "no third status" rule, now naming `deferred:` as
      the one exception, with its owning plan.
- [ ] Run the new-builtin drill and record both failure lines.
- [ ] Observation O1 (findings §3.2): build the 15 `Exempt` rows' probes at S2 and
      S9 and record that none of them builds a `su_global_block`/`su_ref_block`
      slot (`markers.py` over the dump, findings Appendix C.1). A seam-visible row
      with no arm would still pay O1's dead load. Any hit is recorded as a
      Correction.

Acceptance: `cargo test --bin mfb self_update && cargo test --test inplace_self_update_census --test rt_inplace_self_update`
→ pass; drill lines recorded (est. 25 min: the full harness, now with the `String`
lines at S1, S2 and S9, is the one check that every expectation holds together).
Commit: —

### Phase 2: Docs

- [ ] `src/docs/spec/memory/05_collections.md` "Self-updates" (`:492`): `String`
      builtin self-updates join the rule. Name the arm families (window, grow,
      rewrite, identity) and the `String` exemptions, and state that a `String`
      binding's spare capacity is tracked by the compiler and never observable.
      Findings §3.6 item 1's claim ("or a `MUT` captured by reference") is true
      after G, so keep it and cite `string_shadow_env_index`. Gate:
      `cargo build && cargo test --bin mfb spec`.
- [ ] `src/docs/spec/memory/03_heap-values.md` "Standalone String": if it states
      that a `String`'s allocation is always `byteLength + 9`, add that an in-place
      self-update may leave spare capacity, which every copy, return and transfer
      drops (findings B.3 fact 2). Check with
      `grep -n 'byteLength + 9\|+ 9' src/docs/spec/memory/03_heap-values.md`.
- [ ] `.ai/collections.md` §"In-place mutation": the `String` arms, the one shadow
      rule (`is_string_self_update`), the shared S9 shadow, and "a new `String`
      builtin with a self-update form needs a row".
- [ ] `.ai/codegen-invariants.md`: findings §3.6 item 3 (an unmarked producer is
      also an aliasing source for `lower_value_owned`). bug-667 owned it. Verify it
      landed (`grep -n 'toString(String)\|pathDirName' .ai/codegen-invariants.md`)
      and record the line. Add it only if missing.
- [ ] `self_update.rs` module doc and the census guard's doc comment: "`x` a
      `List`, `Map` or `Set`" becomes "…or a `String`".
- [ ] `mfb man strings`, `fs`, `os`: verify no page claims a self-update copies.
      Check with `scripts/man-census.sh --memory-scope` → 0 unclassified.

Acceptance: `cargo test --bin mfb spec` → pass;
`scripts/man-census.sh --memory-scope` → 0 unclassified hits (est. 10 min).
Commit: —

### Phase 3: Full gate (run once)

- [ ] `cargo test` (the full-suite gate, `.ai/testing-gates.md:423`).
- [ ] `scripts/artifact-gate.sh target/release/mfb all` (`.ai/testing-gates.md:11`).
- [ ] `scripts/test-accept.sh target/debug/mfb target/accept-actual` (`.ai/compiler.md:85`).

Acceptance: all three commands green, each with its summary line recorded here
(est. 60 min: the full gate, required once by `.ai/testing-gates.md`).
Commit: —

## Validation Plan

- The full gate above is the plan's only full-suite run. Every earlier letter ran
  scoped checks and `cargo test --bin mfb`.

## Open Decisions

None.

## Corrections

## Summary

Deleting `Pending` again makes the `String` census an obligation instead of a
checklist, and the drill shows the guard fires. Then the docs are corrected and
the full gate runs once. The follow-up plan inherits the `Deferred` rows for
`AttributedString` and the `String` field lines.
