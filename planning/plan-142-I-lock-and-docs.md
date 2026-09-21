# plan-142-I: Lock the guard, sync the docs, run the full gate

Last updated: 2026-09-20
Effort: medium (1h–2h)
Depends on: plan-142-H

Prerequisites: see plan-142-A.

After H every row of `SELF_UPDATE_TABLE` is `Arm` or `Exempt`. This letter makes
that permanent: the `Pending` variant is deleted, so a future builtin with a
self-update form cannot be registered without an arm or a justified exemption —
the census test fails, and the table cannot express "later". It then corrects the
docs plan-141 found stale and runs the project's full gate once.

## 1. Goal

- `SelfUpdate::Pending` no longer exists; `cargo test --bin mfb self_update` and the
  black-box census pass; `cases.tsv` has no `pending:` line and the harness rejects one.
- A **new-builtin drill** proves the guard: add a throwaway `collections::probeSelf(value AS List OF T) AS List OF T`
  registry member on a scratch branch → `self_update_census_covers_every_registry_overload`
  fails naming it, and `inplace_self_update_census` fails naming it. Recorded here,
  branch discarded.
- Docs match the code.

## Phases

### Phase 1 — Lock

- [ ] Delete `SelfUpdate::Pending` and the harness's `pending:` handling.
- [ ] Run the new-builtin drill; record both failure lines here.

Acceptance: `cargo test --bin mfb self_update && cargo test --test inplace_self_update_census --test rt_inplace_self_update`
→ pass; drill failure lines recorded (est. 25 min).
Commit: —

### Phase 2 — Docs

- [ ] `.ai/collections.md` §"In-place mutation": the four contradictions in
      plan-141 findings §3.4 (dispatch lines, "23 conditions", the `FOR EACH`/append
      claim, the overbroad `x = OP(x, …)` sentence), rewritten for the new
      table-driven seam, the four sites, `SELF_UPDATE_TABLE`, and the rule "a new
      builtin with a self-update form needs a row: `Arm` or `Exempt`".
- [ ] `planning/completed/plan-121-gate-inventory.md` is history — add a one-line
      pointer at its top to `self_update.rs` as the current source of truth, and do
      not edit its body.
- [ ] `mfb spec` memory section on collections (`src/docs/spec/memory/05_collections.md`,
      the in-place paragraphs around `:510`): the sites and the failure-atomicity
      rule, cited to the code. Gate: `cargo build && cargo test --bin mfb spec`.
- [ ] `mfb man collections`: the package text says helpers "do not mutate their
      arguments" — still true (a self-update is an assignment); leave it, and verify
      no man page claims a self-update copies. Check with
      `scripts/man-census.sh --memory-scope` → 0 unclassified.

Acceptance: `cargo test --bin mfb spec` → pass; `scripts/man-census.sh --memory-scope`
→ 0 unclassified hits (est. 10 min).
Commit: —

### Phase 3 — Full gate (run once)

- [ ] `cargo test` (the full-suite gate, `.ai/testing-gates.md:423`).
- [ ] `scripts/artifact-gate.sh target/release/mfb all` (`.ai/testing-gates.md:11`).
- [ ] `scripts/test-accept.sh target/debug/mfb target/accept-actual` (`.ai/compiler.md:85`).
- [ ] Re-run plan-141's `/tmp/inplace_probe` timing program; record local vs global
      ns/op for `List` and `Map` `set` beside plan-141's numbers.

Acceptance: all three commands green; timings recorded (est. 60 min — the full
gate, required once by `.ai/testing-gates.md`).
Commit: —

## Corrections

## Summary

Deleting `Pending` turns the census from a checklist into a compile-and-test
obligation; the drill shows the guard fires. Then docs and the full gate.
