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

- [x] Delete `SelfUpdate::Pending` and the harness's `pending:` handling.
      `SelfUpdate::Pending` (and its `pending()` constructor) went in plan-142-E:
      once the `Exempt` rows landed nothing constructed it, and dead code does not
      wait. Then the harness: `Status::Pending` and its bound are gone, and a
      `cases.tsv` status other than `arm`/`exempt` panics ("status `…` is neither
      `arm` nor `exempt` … a self-update needs an in-place arm or a proven
      exemption"); the `cases.tsv` header documents the two statuses.
- [x] Run the new-builtin drill; record both failure lines here.
      A throwaway `collections::probeSelf(value AS List OF T) AS List OF T`
      (`Body::Mfb`, registered after `distinct`) was added to the working tree and
      removed after (no branch: the project never branches; the file was restored
      from a copy and `git status` showed it clean):
      * `cargo test --bin mfb self_update_census_covers_every_registry_overload` →
        "self-update-shaped builtin(s) with no SELF_UPDATE_TABLE row — add an `Arm`
        (an in-place lowering) or an `Exempt` with its proof: ["collections::probeSelf"]".
      * `cargo test --test inplace_self_update_census` → "self-update-shaped
        overload(s) documented by `mfb man` with no line in
        tests/runtime/inplace_self_update/cases.tsv — give each an in-place arm (or a
        proven exemption) and a case: collections::probeSelf(value AS List OF T) AS
        List OF T".

Acceptance: `cargo test --bin mfb self_update && cargo test --test inplace_self_update_census --test rt_inplace_self_update`
→ pass; drill failure lines recorded (est. 25 min).
Verified 2026-09-21: `cargo test --bin mfb self_update` → `4 passed`;
`cargo test --test inplace_self_update_census` → `1 passed`;
`cargo test --test rt_inplace_self_update` → `test result: ok. 1 passed; 0 failed`
(679.52s, 254 case/site pairs — the `&` line at S2 included).
Commit: —

### Phase 2 — Docs

- [x] `.ai/collections.md` §"In-place mutation": the four contradictions in
      plan-141 findings §3.4 (dispatch lines, "23 conditions", the `FOR EACH`/append
      claim, the overbroad `x = OP(x, …)` sentence), rewritten for the new
      table-driven seam, the four sites, `SELF_UPDATE_TABLE`, and the rule "a new
      builtin with a self-update form needs a row: `Arm` or `Exempt`".
      Rewritten as "one table, four sites": the dispatch is named by function
      (`NirOp::Assign`/`StoreGlobal` → `try_inplace_self_update`), not by line; the
      "23 conditions" pointer now names the code as the current list; the
      `FOR EACH` rule states what the code does (a loop whose body writes its
      iterable walks a copy, `G7` declines otherwise), and the overbroad sentence
      now lists exactly the four sites. Two more copies of the stale `FOR EACH`
      claim were fixed with it: the section's first bullet in `.ai/collections.md`
      and `try_inplace_append_assign`'s doc comment (plan-141 §3.4 item 3's `bia:17-19`).
- [x] `planning/completed/plan-121-gate-inventory.md` is history — add a one-line
      pointer at its top to `self_update.rs` as the current source of truth, and do
      not edit its body.
- [x] `mfb spec` memory section on collections (`src/docs/spec/memory/05_collections.md`,
      the in-place paragraphs around `:510`): the sites and the failure-atomicity
      rule, cited to the code. Gate: `cargo build && cargo test --bin mfb spec`.
      A new *Self-updates* section (sites, failure atomicity, snapshots, scratch, the
      exempt functions) with `[[…]]` citations to `try_inplace_self_update`,
      `lower_for_each` and `add_global_string_capacities`; `set`'s "excluded while
      the binding is an active `FOR EACH` iterable" is replaced by the copy rule.
      `cargo test --bin mfb spec` → `test result: ok. 43 passed`.
- [x] `mfb man collections`: the package text says helpers "do not mutate their
      arguments" — still true (a self-update is an assignment); leave it, and verify
      no man page claims a self-update copies. Check with
      `scripts/man-census.sh --memory-scope` → 0 unclassified.
      Two did (Correction I1): `collections::append` and `collections::set` said a
      self-update of "a module-level `MUT`, or the list a `FOR EACH` is walking"
      copies the whole collection on every call. Rewritten to name the four sites
      as the cheap shape and "assigning the result anywhere else builds a new
      list". `add` makes no such claim; `remove`'s narrower wording is true.
      `scripts/man-census.sh --memory-scope` → `unclassified memory-vocabulary hits: 0`.

Acceptance: `cargo test --bin mfb spec` → pass; `scripts/man-census.sh --memory-scope`
→ 0 unclassified hits (est. 10 min).
Verified 2026-09-21: `test result: ok. 43 passed`; `unclassified memory-vocabulary hits: 0`.
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

- **I1 (Phase 2): two man pages were stale, not zero.** The plan expected to leave
  the man text alone and only verify it. `collections::append`'s and
  `collections::set`'s descriptions stated the pre-plan cost model (a global or a
  `FOR EACH`-walked binding copies every call), which plan-142-F/H made false;
  they were corrected. The man census counts memory vocabulary, not claims about
  cost, so it could not have flagged them.

## Summary

Deleting `Pending` turns the census from a checklist into a compile-and-test
obligation; the drill shows the guard fires. Then docs and the full gate.
