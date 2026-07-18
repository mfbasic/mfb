# bug-325: plan-20 relocation residue in `src/syntaxcheck/**` — 36 empty-`if` shells, a dead 36-line graph walk, an intentionally-empty reported function, 19 assertion-free tests, and an unenforced LINK-rule mirror

Last updated: 2026-07-18
Effort: medium-to-large
Severity: LOW
Class: Dead-code

Status: Open
Regression Test: acceptance suite (`scripts/test-accept.sh`, 522
`tests/syntax/**/golden/build.log` fixtures) + a new syntaxcheck/`ir::verify`
NATIVE_* rule-parity test (see Validation Plan)

plan-20 relocated ~71 semantic rules out of `src/syntaxcheck/**` into
`src/ir/verify` (`RELOCATED_TO_IR_VERIFY`, `src/ir/verify/mod.rs:70`), making verify
the sole rejecter on both the source and package paths. The relocation deleted each
rule's *diagnostic body* but left its *condition* in place. What remains is a
tree-wide pattern of `if <predicate> {}` — the check still evaluates, the branch does
nothing, and the code reads as though a rule is enforced where none is.

Nothing here misbehaves at runtime. The cost is that `src/syntaxcheck/**` now
misrepresents itself: a reader (or an LLM) sees `if self.record_field_cycle(&type_decl.name) {}`
and concludes syntaxcheck detects record cycles, and 19 tests carry names like
`match_non_exhaustive_falls_through` while asserting literally nothing. The single
correct outcome a fix produces: **every vestigial shell, its uniquely-vestigial
callee, and every assertion-free test is deleted or given a real assertion, with the
compiler's diagnostic output byte-identical before and after.**

References:

- `src/ir/verify/mod.rs:70` (`RELOCATED_TO_IR_VERIFY`, 71 rule names) and `:386` — the
  plan-20 contract this residue is left over from.
- `src/docs/spec/architecture/02_frontend.md` — syntaxcheck's documented role.
- `src/docs/spec/package/12_verifier-rules.md:83` — the `ir::verify` LINK/NATIVE rule
  set.
- bug-301 item G3 already files `check_link_function` as dead; this document restates
  it in its plan-20 context and adds the parity hazard around it. Fix once, in
  whichever lands first.
- Cleanup review 2026-07-18, Agent 13 #3/#4/#5/#6 and Agent 22 #4/#5.

## Current State

All counts below were measured at head, not taken from the review lead.

| Item | Count | Where |
| --- | --- | --- |
| Empty `if <cond> {}` shells | **36** | `checking.rs` 10, `inference.rs` 17, `mod.rs` 9 |
| …of which `cargo clippy` reports (`needless_if`) | **15** | `checking.rs` 6, `inference.rs` 8, `mod.rs` 1 |
| Vestigial graph walk (`record_field_cycle` + `direct_record_successors`) | 36 lines | `mod.rs:1597-1632` |
| Reported function whose body is a comment saying it is empty | 10 lines | `mod.rs:1586-1595`, invoked `mod.rs:2019` |
| `assert!(true)` tests | **19** | `checking.rs` 7, `helpers.rs` 12 |
| `assert!(true)` anywhere else in the repo | **0** | — |
| Dead `check_link_function` | 6 lines | `mod.rs:688-693` |
| Hand-mirrored LINK checking | 334 + 305 lines | `mod.rs:697-1030` vs `ir/verify/mod.rs:2944-3248` |

Verbatim representative — `src/syntaxcheck/mod.rs:2003-2019`, containing three of the
36 shells including the one that is the sole caller of the 36-line graph walk, and the
call to the function whose body is a comment:

```rust
                for field in &type_decl.fields {
                    let type_ = self.parse_type(&field.type_name);
                    self.check_type_reference(file, &type_, field.line);
                    if self.is_resource_type(&type_) {}
                }
                if self.record_field_cycle(&type_decl.name) {}
                ...
                for include in &type_decl.includes {
                    let type_ = self.parse_type(include);
                    self.check_type_reference(file, &type_, type_decl.line);
                    if let Some(kind) = self.user_type_kinds.get(include) {
                        if !matches!(kind, TypeDeclKind::Union) {}
                    }
                }
                self.report_expanded_union_member_conflicts(file, type_decl);
```

### The 36 shells

`checking.rs` — `:210 :222 :257 :265 :381 :400 :481 :538 :609 :632`
`inference.rs` — `:190 :253 :377 :499 :515 :544 :547 :696 :705 :804 :818 :821 :869 :872 :1329 :1330 :1428`
`mod.rs` — `:2007 :2009 :2016 :2025 :2037 :2117 :2147 :2215-2218 :2328`

Clippy's `needless_if` reports only 15 of these because its pattern does not fire on:
the multi-line form (`mod.rs:2215-2218`, whose `{}` sits alone on `:2218`), `if let`
shells (`inference.rs:190,869`), or a majority of the single-line ones for
pattern-shape reasons. **Clippy is not a sufficient inventory for this cleanup** — the
authoritative sweep is `grep -nE '^\s*(\} *else +)?if .*\{\}\s*$' src/syntaxcheck/*.rs`
(35 hits) plus `grep -nE '^\s*\{\}\s*$'` (1 hit, `mod.rs:2218`).

Two degenerate cases worth naming:

- `src/syntaxcheck/inference.rs:819-822` is a `for` loop whose **entire body** is one
  empty `if`:
  ```rust
        for field in fields {
            if !self.visible_from(file, field.visibility, owner_file_path) {}
        }
  ```
- `src/syntaxcheck/inference.rs:1329-1330` are two shells over pure struct-field
  predicates that compile to nothing whatsoever — no call, no side effect, no branch:
  ```rust
            if param.type_name.is_none() {}
            if param.default.is_some() {}
  ```

### The 19 assertion-free tests

Every `assert!(true)` in the entire repository is in `src/syntaxcheck/`:
`checking.rs:1188,1205,1222,1277,1297,1316,1387` and
`helpers.rs:470,488,531,571,591,615,665,741,756,816,862,883`. All 19 follow the same
shape, e.g. `src/syntaxcheck/checking.rs:1387` (`match_non_exhaustive_falls_through`):

```rust
        let _ = check_src(src);
        assert!(true);
```

Each names a semantic rule (`propagate_outside_trap_is_walked`,
`match_non_exhaustive_falls_through`, `foreach_over_resource_list_marks_element_borrowed`,
`state_assign_to_local_without_state_type_is_walked`, …) but would pass unchanged if
the rule it names were deleted outright — they assert only that `check_src` does not
panic. `src/syntaxcheck/helpers.rs:449-450` states the situation plainly in a comment
on a neighbouring (real) test: *"Range/mismatch checks are ir::verify no-ops here."*

## Root Cause

`src/syntaxcheck/**` reports diagnostics exclusively through `self.report`
(`mod.rs:2441`) and `self.report_warning` (`mod.rs:2465`). plan-20 emptied the bodies
guarded by these conditions but kept the conditions, because deleting a condition
would also have deleted the `infer_expression`/`parse_type`/`check_type_reference`
calls that frequently sit on the *same statement or the surrounding loop* — and those
are load-bearing (they populate locals and report nested errors). The safe local edit
at the time was to leave the `if` and empty the block. Thirty-six of those edits
accumulated.

`record_field_cycle` (`mod.rs:1620-1632`) and its helper `direct_record_successors`
(`mod.rs:1597-1617`) are the extreme case: the relocation moved the *rule* to
`ir/verify/mod.rs:2347` (a distinct reimplementation with a `seen` set and a
`target`, called from `:2205` and recursively at `:2361`), leaving syntaxcheck's
worklist DFS reachable only from the empty shell at `mod.rs:2009`. A full-repo grep
for `record_field_cycle|direct_record_successors` returns exactly: the definitions,
the internal recursion at `:1622`/`:1630`, the empty shell at `:2009`, the
`ir::verify` copy, and one test (`mod.rs:2722`, `record_field_cycle_walk`).
`direct_record_successors` has no caller other than `record_field_cycle`.

`report_expanded_union_member_conflicts` (`mod.rs:1586-1595`) went one step further:
the body was replaced by a comment stating that the rule now lives in `ir::verify` and
"the body is intentionally empty" — but the call at `mod.rs:2019` was left in place,
so the file both documents the deletion and performs the call.

## The correctness question: are the predicates pure?

This is the one place this cleanup can change behavior, so it is settled by
construction rather than by inspection:

- `self.report` and `self.report_warning` (`mod.rs:2441`, `:2465`) both take
  **`&mut self`**.
- Every predicate appearing in the 36 shells takes **`&self`**:
  `compatible` (`types.rs:117`), `expression_compatible` (`types.rs:243`),
  `is_numeric` (`types.rs:289`), `is_resource_type` (`resources.rs:4`),
  `visible_from` (`mod.rs:2221`), `record_field_cycle` (`mod.rs:1620`),
  `direct_record_successors` (`mod.rs:1597`).
- `struct SyntaxChecker` (`mod.rs:218-…`) holds only plain owned fields —
  `HashMap`/`HashSet`/`Vec`/`bool`/`Type`. A grep for `Cell<`, `RefCell`, `Mutex`,
  `Atomic`, and `unsafe` across all of `src/syntaxcheck/*.rs` returns **zero hits**.

Therefore an `&self` predicate cannot reach `diagnostics`, `had_error`, or any other
mutable state: purity is enforced by the borrow checker, not asserted. The remaining
shells contain no method call at all (`matches!`, `.is_none()`, `.is_some()`,
`arguments.len() != fields.len()`, `!local.mutable`, `self.loop_stack.iter().rev().any(…)`,
`if let Expression::Number(_value) = …`), which are trivially pure.

**Deleting the shells is therefore behavior-preserving.** State this explicitly in the
commit message: if a future predicate is made `&mut self`, the same deletion would not
be safe, and the acceptance goldens are the backstop.

Equally important, and the reason this is a scalpel and not a chainsaw: **the helpers
are not vestigial.** Measured non-empty-`if` call sites in `src/syntaxcheck/**`:

| Helper | total call sites | inside an empty `if` | live elsewhere |
| --- | --- | --- | --- |
| `expression_compatible` | 20 | 15 | yes (e.g. `builtins.rs:1065`) |
| `compatible` | 18 | 1 | yes |
| `is_resource_type` | 11 | 1 | yes |
| `visible_from` | 8 | 4 | yes |
| `is_numeric` | 6 | 1 | yes |
| `record_field_cycle` | 1 | **1** | **no — delete** |

Only `record_field_cycle` (and, transitively, `direct_record_successors`) loses its
last caller. Every other helper must survive.

## The structural hazard: LINK checking is mirrored with nothing enforcing parity

`src/syntaxcheck/mod.rs:697-1030` (`check_link_function_in`, 334 lines) and
`src/ir/verify/mod.rs:2944-3248` (`check_link_functions`, 305 lines) are two
independently-maintained bodies validating the same `LINK` ABI facts. Measured
`NATIVE_*` rule names emitted per file:

- `src/syntaxcheck/mod.rs` — 13 distinct: `NATIVE_ABI_NO_RESULT`,
  `NATIVE_ABI_RESULT_MARKER`, `NATIVE_ABI_UNBOUND_PARAM`, `NATIVE_ABI_UNBOUND_SLOT`,
  `NATIVE_ABI_UNKNOWN_CTYPE`, `NATIVE_BIND_IN_INVALID`, `NATIVE_CONST_OUT`,
  `NATIVE_CONST_UNKNOWN_SLOT`, `NATIVE_CPTR_ESCAPE`, `NATIVE_CSTRUCT_ESCAPE`,
  `NATIVE_CSTRUCT_INVALID`, `NATIVE_FREE_INVALID`, `NATIVE_STRUCT_FIELD_MISMATCH`.
- `src/ir/verify/mod.rs` — those same 13, **plus** `NATIVE_BIND_STATE_INVALID`.

The asymmetry is legitimate: `NATIVE_BIND_STATE_INVALID` is the one `NATIVE_*` name
in `RELOCATED_TO_IR_VERIFY` (`src/ir/verify/mod.rs:83`), so verify is deliberately its
sole rejecter. But nothing in the tree records that. The invariant

> syntaxcheck's `NATIVE_*` set ∪ (`RELOCATED_TO_IR_VERIFY` ∩ `NATIVE_*`) == `ir::verify`'s `NATIVE_*` set

currently holds by luck across two ~320-line hand-synced bodies. Adding a rule to one
side and forgetting the other is silent today.

`src/syntaxcheck/mod.rs:688-693` sits in the middle of this. `check_link_function` is
a 6-line wrapper calling `check_link_function_in(file, function, &[])`; the live path
calls `check_link_function_in` directly from `check_link_block` with the real cstruct
list. `cargo clippy` confirms it: `src/syntaxcheck/mod.rs:688:19: warning: method
check_link_function is never used`. And `src/ir/verify/mod.rs:2936` opens
`check_link_functions`'s doc comment with *"Validate the merged LINK table
(syntaxcheck's `check_link_function` on the IR)"* — the surviving documentation of
this seam names the dead symbol.

## Goal

- All 36 empty `if … {}` shells are removed from `src/syntaxcheck/**`, preserving every
  load-bearing call on the surrounding statements.
- `record_field_cycle`, `direct_record_successors` (`mod.rs:1597-1632`), and the test
  `record_field_cycle_walk` (`mod.rs:2722`) are deleted; `ir/verify/mod.rs:2347`
  remains the only implementation.
- `report_expanded_union_member_conflicts` (`mod.rs:1586-1595`) and its call
  (`mod.rs:2019`) are deleted.
- `check_link_function` (`mod.rs:688-693`) is deleted and
  `ir/verify/mod.rs:2936`'s doc comment is corrected to name `check_link_function_in`.
- Each of the 19 `assert!(true)` tests either gains a real assertion about the
  behavior its name claims, or is deleted; `grep -r 'assert!(true)' src/ tests/`
  returns zero.
- A parity test asserts the syntaxcheck/`ir::verify` `NATIVE_*` rule-set invariant
  above and fails if either side gains a rule the other lacks without a
  `RELOCATED_TO_IR_VERIFY` entry.
- `cargo clippy` reports zero `needless_if` and zero `assertions_on_constants` in
  `src/syntaxcheck/**`.

### Non-goals (must NOT change)

- **Diagnostics output must not change.** No rule name, message, span, or emission
  order may shift. `scripts/test-accept.sh` compares 522
  `tests/syntax/**/golden/build.log` files verbatim; a green run with **zero
  regenerated goldens** is the acceptance bar. Regenerating a golden to accommodate
  this cleanup is explicitly forbidden — it would mean a predicate was not pure after
  all, which is a finding, not a golden update.
- **The predicate helpers themselves.** `compatible`, `expression_compatible`,
  `is_numeric`, `is_resource_type`, and `visible_from` all have live non-empty callers
  (table above) and stay. Only the shells go, plus `record_field_cycle` /
  `direct_record_successors`, whose only caller *is* a shell.
- **The surrounding load-bearing calls.** Many shells sit inside loops or after
  `let` bindings whose right-hand side calls `infer_expression`, `parse_type`, or
  `check_type_reference` — those DO report diagnostics and DO populate `locals`. Delete
  the `if`, never the statement above it, and never the enclosing `for` (except
  `inference.rs:819-822`, where the loop body is nothing but the shell and the loop
  goes with it — verify the iterator is side-effect-free first).
- **The plan-20 split.** `ir::verify` stays the sole rejecter for every rule in
  `RELOCATED_TO_IR_VERIFY`. Do not "restore" a deleted diagnostic into syntaxcheck.
- **`src/rules/table.rs`.** Confirmed not drifted (233 names, zero orphans); this
  cleanup deletes no rule, so the table is untouched.
- The `NATIVE_BIND_STATE_INVALID` asymmetry — it is correct. The parity test must
  encode it as an expected exception sourced from `RELOCATED_TO_IR_VERIFY`, not
  hardcode the name.

## Blast Radius

- `src/syntaxcheck/checking.rs:210,222,257,265,381,400,481,538,609,632` — 10 shells;
  fixed by this bug.
- `src/syntaxcheck/inference.rs:190,253,377,499,515,544,547,696,705,804,818,821,869,872,1329,1330,1428`
  — 17 shells; fixed. `:821` takes its enclosing `for` (`:819-822`) with it;
  `:1329,:1330` compile to nothing today.
- `src/syntaxcheck/mod.rs:2007,2009,2016,2025,2037,2117,2147,2215-2218,2328` — 9
  shells; fixed. `:2009` is the last caller of `record_field_cycle`.
- `src/syntaxcheck/mod.rs:1597-1632` — `direct_record_successors` +
  `record_field_cycle`; deleted with `:2009`.
- `src/syntaxcheck/mod.rs:2722` (`record_field_cycle_walk`) — a test whose only subject
  is the deleted walk; deleted. `mod.rs:2723` comment goes with it.
- `src/syntaxcheck/mod.rs:1586-1595,2019` — `report_expanded_union_member_conflicts`
  and its call; deleted. `mod.rs:2674` and `:2772` are test comments naming it;
  update or drop.
- `src/syntaxcheck/mod.rs:688-693` — dead `check_link_function`; deleted (same item as
  bug-301 G3).
- `src/ir/verify/mod.rs:2936` — doc comment naming the dead symbol; corrected.
- `src/syntaxcheck/checking.rs:1188,1205,1222,1277,1297,1316,1387` and
  `src/syntaxcheck/helpers.rs:470,488,531,571,591,615,665,741,756,816,862,883` — 19
  assertion-free tests; each gets a real assertion or is deleted.
- `src/ir/verify/mod.rs:2205,2347,2361` — the live `record_field_cycle`; **unaffected**,
  this is the implementation that stays.
- `src/syntaxcheck/mod.rs:697-1030` and `src/ir/verify/mod.rs:2944-3248` — the mirrored
  LINK bodies. **Latent, NOT fixed here.** Merging them is a behavior-visible change
  (spans differ: syntaxcheck reports slot-level, verify reports function-level, per
  `ir/verify/mod.rs:2942-2943`) and would move goldens. Out of scope; this bug adds
  only the parity *test* so the divergence becomes loud.
- `src/builtins/**`, `src/resolver/**`, `src/monomorph/**` — unaffected; the shells and
  the dead walk are confined to `src/syntaxcheck/`.

## Fix Design

Four independent, separately-landable edits, ordered by risk:

1. **Shell deletion.** Mechanical, but do it by hand, not with `clippy --fix` — clippy
   sees only 15 of 36 and its suggested rewrite (`!self.expression_compatible(…);` as a
   bare statement) is exactly the wrong outcome: it preserves the dead call instead of
   removing it. Delete the whole `if`. Where the shell is the sole body of a loop
   (`inference.rs:819-822`), delete the loop after confirming the iterator has no side
   effects. Where the shell is nested inside a live `if let`
   (`mod.rs:2016,2025`; `inference.rs:869`), collapse the now-empty outer binding too
   only if it likewise contains nothing else.
2. **Dead-symbol deletion.** `record_field_cycle` + `direct_record_successors` +
   `record_field_cycle_walk`; `report_expanded_union_member_conflicts` + its call;
   `check_link_function` + the `ir/verify` doc comment that names it. `cargo` already
   proves the last one dead; confirm the first two with a fresh
   `cargo clippy --all-targets` after deletion (no new `never used` warnings).
3. **Test repair.** For each of the 19, read what the test name claims and assert it —
   most should become `assert!(rejects_with(src, "<RULE>"), "{:?}", check_src(src))`
   or the `!rejects_with` form already used at `helpers.rs:449-454`. Where the named
   rule is genuinely relocated and syntaxcheck genuinely has nothing to assert, the
   honest outcome is deletion, or relocation of the assertion to
   `src/ir/verify/tests.rs`. Do not leave a renamed no-op.
4. **The parity test.** New test, colocated with the LINK checking it guards
   (`src/syntaxcheck/mod.rs`'s inline test module, or `src/ir/verify/tests.rs`):
   extract the set of `"NATIVE_*"` string literals appearing in
   `src/syntaxcheck/mod.rs` and in `src/ir/verify/mod.rs` — via `include_str!` on the
   two sources, which keeps the test a pure text invariant with no refactor coupling —
   and assert
   `syntaxcheck_set ∪ (RELOCATED_TO_IR_VERIFY ∩ native_names) == verify_set`, with a
   failure message naming the offending rule and pointing at whichever body is missing
   it.

Rejected alternatives:

- **`cargo clippy --fix`** — see above; it converts shells into bare
  discarded-expression statements, keeping the dead work and satisfying the lint. That
  is a worse end state than today because the intent becomes unrecoverable.
- **Renaming the predicates `check_*` so the discarded bool reads as incidental**
  (Agent 22 #5's suggestion) — it dresses up dead code as intentional. Delete it.
- **Merging the two LINK bodies now** — the span granularity differs by design, so it
  moves goldens; keep this bug diagnostics-neutral and file the merge separately.
- **A CI grep banning `assert!(true)`** — worth doing, but as a follow-up once the 19
  are gone; adding the guard first just blocks the branch.

Expected output shift: **none.** That is the whole point, and the acceptance suite is
the proof.

## Phases

### Phase 1 — baseline + purity audit (no behavior change)

- [ ] Record a clean `scripts/test-accept.sh` run and the golden count as the
      byte-exact baseline.
- [ ] Re-derive the 36-shell inventory at head with the two greps in Current State and
      write the confirmed list into this file (done above; re-confirm before editing).
- [ ] Re-confirm the purity argument: every predicate in the inventory is `&self`;
      `report`/`report_warning` are `&mut self`; zero `Cell`/`RefCell`/`unsafe` in
      `src/syntaxcheck/*.rs`. If any predicate is `&mut self`, **stop** — that shell is
      not safe to delete and is a separate finding.

Acceptance: baseline recorded; inventory and purity verdict written down per site.
Commit: —

### Phase 2 — delete the shells and the dead symbols

- [ ] Delete all 36 shells, preserving surrounding load-bearing calls.
- [ ] Delete `record_field_cycle`, `direct_record_successors`, `record_field_cycle_walk`.
- [ ] Delete `report_expanded_union_member_conflicts` and its call at `mod.rs:2019`;
      update the test comments at `mod.rs:2674,2772`.
- [ ] Delete `check_link_function` (`mod.rs:688-693`); fix `ir/verify/mod.rs:2936`'s
      doc comment to name `check_link_function_in`.
- [ ] `cargo clippy --all-targets`: zero `needless_if` in `src/syntaxcheck/**`, zero new
      `never used`.

Acceptance: `scripts/test-accept.sh` green with **zero regenerated goldens**;
`cargo test` green.
Commit: —

### Phase 3 — tests: 19 real assertions + the parity guard

- [ ] Give each of the 19 `assert!(true)` tests a real assertion, or delete it and
      record why in the commit message.
- [ ] Add the syntaxcheck/`ir::verify` `NATIVE_*` parity test; confirm it passes at
      head (the `NATIVE_BIND_STATE_INVALID` exception must come from
      `RELOCATED_TO_IR_VERIFY`, not a literal).
- [ ] Prove the parity test bites: temporarily delete one `NATIVE_*` emission from
      `src/syntaxcheck/mod.rs`, confirm the test fails naming that rule, revert.
- [ ] `grep -r 'assert!(true)' src/ tests/` returns nothing.

Acceptance: full suite green; the parity test demonstrated to fail on an induced
divergence.
Commit: —

## Validation Plan

- Regression test(s): the acceptance suite is the primary guard — 522
  `tests/syntax/**/golden/build.log` fixtures pin diagnostic text, ordering, and spans
  verbatim, so any behavior change from deleting a shell surfaces as a golden diff.
  Plus the new `NATIVE_*` rule-parity test (Phase 3), which is the only *new* coverage
  this bug adds, and the 19 repaired tests.
- Runtime proof: build a fixture exercising each rule family the deleted shells
  mention (record-field cycle, expanded-union member conflict, resource-field
  declaration, private-field visibility, non-exhaustive MATCH) and diff `mfb build`
  output before and after — byte-identical, with the diagnostic still coming from
  `ir::verify`.
- Doc sync: `src/ir/verify/mod.rs:2936` doc comment (names a symbol being deleted).
  Separately noted but **not** in scope: cleanup review Agent 13 #8 reports
  `spec/architecture/02_frontend.md:231-234` claims syntaxcheck emits no semantic
  rules while it emits ~43 — unverified here, and a spec fix, not a code fix.
- Full suite: `cargo test` + `scripts/test-accept.sh` + `cargo clippy --all-targets`
  (warning count must drop by ≥34: 15 `needless_if`, 19 `assertions_on_constants`, 1
  `never used`).

## Open Decisions

- Repair vs. delete for the 19 tests — recommend repair where the named rule is still
  reachable from `check_src` output, delete where it is purely an `ir::verify` rule
  with nothing observable at the syntaxcheck layer. Decide per test in Phase 3, record
  the verdict in the commit.
- Where the parity test lives — `src/ir/verify/tests.rs` (recommended: it already owns
  `RELOCATED_TO_IR_VERIFY`) vs. `src/syntaxcheck/mod.rs`'s inline module.
- Whether to add a CI grep banning `assert!(true)` repo-wide (Agent 22 #4) — recommend
  yes, as a follow-up commit after Phase 3.

## Summary

The engineering risk is concentrated entirely in one question — are the shell
predicates side-effect-free? — and that question is already answered by the borrow
checker: `report`/`report_warning` need `&mut self`, every predicate in the inventory
takes `&self`, and `src/syntaxcheck/` contains no interior mutability and no `unsafe`.
Given that, the 36 deletions are provably inert and the 522 diagnostic goldens make it
demonstrable rather than argued. The audit, not the edit, is the work: clippy sees only
15 of 36, so a lint-driven cleanup would leave 21 shells and convert the other 15 into
dead statements. Left untouched: all five predicate helpers (they have live callers),
the plan-20 split itself, and the two mirrored LINK bodies — which this bug only makes
loud, via a parity test, rather than merging.
