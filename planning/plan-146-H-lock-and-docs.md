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

- [x] Delete `SelfUpdate::Pending` and the harness's `pending:` handling. Restore
      the `cases.tsv` header's "no third status" rule, now naming `deferred:` as
      the one exception, with its owning plan. (No `pending:` line was left to flip:
      letters B–G flipped all 41 as they landed.)
- [x] Run the new-builtin drill and record both failure lines.
      A throwaway `strings::probeSelf(value AS String) AS String` (`Body::mfb`,
      returning `value`) registered in `strings/mod.rs`:
      - unit: `self-update-shaped builtin(s) with no SELF_UPDATE_TABLE row — add an
        `Arm` (an in-place lowering) or an `Exempt` with its proof:
        ["strings::probeSelf"]`;
      - black-box: `self-update-shaped overload(s) documented by `mfb man` with no
        line in tests/runtime/inplace_self_update/cases.tsv — give each an in-place
        arm (or a proven exemption) and a case: strings::probeSelf(value AS String)
        AS String`.
      The change was discarded (`git diff --stat src/codegen/builtins/strings/mod.rs`
      → empty).
- [x] Observation O1 (findings §3.2): build the 15 `Exempt` rows' probes at S2 and
      S9 and record that none of them builds a `su_global_block`/`su_ref_block`
      slot (`markers.py` over the dump, findings Appendix C.1). A seam-visible row
      with no arm would still pay O1's dead load. Any hit is recorded as a
      Correction.
      One program holding all 15 rows at S2 (a `SUB` over a module-level global) and
      all 15 at S9 (one `forEach` lambda each), `mfb build -q -ncode` → **0 of 221
      functions** carry a `su_global_block`, `su_ref_block` or `inplace_str_*` slot.
      None of the 15 is seam-visible, so none pays the dead load. O1 is closed for
      the `String` rows.

Acceptance: `cargo test --bin mfb self_update && cargo test --test inplace_self_update_census --test rt_inplace_self_update`
→ pass; drill lines recorded (est. 25 min: the full harness, now with the `String`
lines at S1, S2 and S9, is the one check that every expectation holds together).
Result: recorded with Phase 3's full gate below (`cargo test` runs all three).
Commit: d648d0231

### Phase 2: Docs

- [x] `src/docs/spec/memory/05_collections.md` "Self-updates" (`:492`): `String`
      builtin self-updates join the rule. Name the arm families (window, grow,
      rewrite, identity) and the `String` exemptions, and state that a `String`
      binding's spare capacity is tracked by the compiler and never observable.
      Findings §3.6 item 1's claim ("or a `MUT` captured by reference") is true
      after G, so keep it and cite ~~`string_shadow_env_index`~~
      `string_shadow_captures` (plan-146-G Correction G1). Gate:
      `cargo build && cargo test --bin mfb spec`.
- [x] `src/docs/spec/memory/03_heap-values.md` "Standalone String": if it states
      that a `String`'s allocation is always `byteLength + 9`, add that an in-place
      self-update may leave spare capacity, which every copy, return and transfer
      drops (findings B.3 fact 2). Check with
      `grep -n 'byteLength + 9\|+ 9' src/docs/spec/memory/03_heap-values.md`.
      It does (`:35`), so the exception is recorded there.
- [x] `.ai/collections.md` §"In-place mutation": the `String` arms, the one shadow
      rule (`is_string_self_update`), the shared S9 shadow, and "a new `String`
      builtin with a self-update form needs a row". Plus the two traps letter G
      paid for: the env word must be read into a frame slot (`%closure_env` is a
      call-boundary token) and `closure_env_free_types` must count every extra word.
- [x] `.ai/codegen-invariants.md`: findings §3.6 item 3 (an unmarked producer is
      also an aliasing source for `lower_value_owned`). bug-667 owned it. Verify it
      landed and record the line. Add it only if missing.
      **It landed**: `:193` ("`toString(String)` is the IDENTITY arm: it hands its
      argument through only when that argument is the statement's own pending
      temporary… the old unconditional identity made `s = toString(s)` free the
      block it stored (bug-667)") and `:201` (`toString(Boolean)` and
      `fs::pathDirName`'s `.`/`/` "did not, and a binding freed rodata (SIGBUS,
      bug-667)"). Nothing to add there; plan-146's own invariant — a bound `String`
      block may be larger than `byteLength + 9`, and the shadow is what makes its
      free correct — is new text beside it.
- [x] `self_update.rs` module doc and the census guard's doc comment: "`x` a
      `List`, `Map` or `Set`" becomes "…or a `String`".
- [x] `mfb man strings`, `fs`, `os`: verify no page claims a self-update copies.
      Check with `scripts/man-census.sh --memory-scope` → 0 unclassified.
      `unclassified memory-vocabulary hits: 0` (plus the four standing carve-outs).

Acceptance: `cargo test --bin mfb spec` → pass;
`scripts/man-census.sh --memory-scope` → 0 unclassified hits (est. 10 min).
Result: `cargo test --bin mfb spec` → `test result: ok. 43 passed; 0 failed`;
`scripts/man-census.sh --memory-scope` → `unclassified memory-vocabulary hits: 0`.
Commit: d648d0231

### Phase 3: Full gate (run once)

- [x] `cargo test` (the full-suite gate, `.ai/testing-gates.md:423`).
      `222` suites reported `test result: ok`, exit 0, zero `FAILED`
      (`grep -cE '^test result: ok'`). The unit suite alone is
      `test result: ok. 4297 passed; 0 failed; 1 ignored`. One suite had to be
      corrected first: `codegen_helper_scratch_release` pinned seven helper
      `(arena_alloc, arena_free, guarded release)` triples written for bug-574's
      marshal-and-release design, which plan-146-F's `borrow_cstring` retired —
      see plan-146-F Correction F8 for the measurement and the proof the test
      described the old design rather than catching a leak.
- [x] `scripts/artifact-gate.sh target/release/mfb all` (`.ai/testing-gates.md:11`).
      `artifact-gate [all]: 1492 tests, 1667 build(s), 2112 golden(s) checked, 0 diff(s)`
- [x] `scripts/test-accept.sh target/debug/mfb target/accept-actual` (`.ai/compiler.md:85`).
      `acceptance tests passed (1518 test(s) ran)`

Acceptance: all three commands green, each with its summary line recorded here
(est. 60 min: the full gate, required once by `.ai/testing-gates.md`).
Result: MET. All three ran on the tree with main merged in at `e9c1fa167`
(bug-679, bug-680 and the `examples/dungeon` split — docs and examples only, no
`src/` or `tests/` overlap), and after `cargo fmt --all` over both workspaces
reported no churn.
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
