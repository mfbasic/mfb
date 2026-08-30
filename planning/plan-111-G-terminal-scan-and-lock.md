# plan-111-G: the terminal scan — prove zero, lock it, archive

Last updated: 2026-08-29
Effort: large (3h–1d)
Depends on: plan-111-F (codegen is string-free; only `src/target`, the `.mfp`
encoder and the tree-wide proof remain).

**This is the letter that makes plan-111 different from plan-106.** plan-106
ended with a census that recorded 155 surviving violations as a follow-up and
archived anyway. This letter ends with a census that must read **zero on every
line**, a gate that fails `cargo test` if any line ever leaves zero, and the
plan's single full cross-target artifact run.

There is no "permitted boundary class" in this letter's vocabulary beyond the
five files named in plan-111-A §2, and no line of the terminal census may be
annotated, reclassified, or deferred. A non-zero line is unfinished work, not a
finding.

See plan-111-A for the shared prerequisites, the five sanctioned boundaries and
the tiered gate policy.

References:

- `src/binary_repr/sections.rs:98-116` — `is_structural` and
  `opaque_structural_kind`, whose doc comment says outright they handle "a
  structural spelling that **did not parse**." With a typed encoder input there
  is no such case; both functions die in Phase 2.
- `src/target/shared/plan/lower.rs:169` —
  `ParameterType::Named(n) if n.resolve() == "Scalar"`, which is
  `type_.is_named("Scalar")` (`src/types.rs:545`).
- `src/target/shared/abi.rs:460` — `move_immediate(dst, type_: &str, value: &str)`,
  the last instruction emitter taking a type as a spelling.
- `planning/completed/plan-106-E-consolidation-no-strings-census.md` §"The
  terminal census" — the format this letter's census mirrors, and the outcome it
  must not repeat.
- `.ai/testing-gates.md` — artifact-gate mechanics and golden regeneration.

## Prerequisites

See plan-111-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-111-F complete | `rg` for all six needle classes over `src/codegen/` → 0 hits; every `codegen` budget 0 | NOT MET until F lands |
| A pre-plan-111 worktree exists for attribution | `git worktree add --detach ../mfb-pre111 <commit before letter A>` | UNMEASURED — create in Phase 4 |
| Pre-plan-111 `N ran` baseline | `scripts/test-accept.sh <target> /tmp/accept-pre111` in the attribution worktree | UNMEASURED — Phase 4 |

## 1. Goal

- The terminal census (§"The terminal census", filled in Phase 3) reads **0 on
  every line**, with every command and count pasted into this file.
- `tests/no_type_strings.rs` has no budget table. Every needle class asserts a
  hard `0` outside the five sanctioned boundary files, and a sixth assertion
  pins the boundary list itself so adding a file to it is a deliberate,
  reviewable act.
- `scripts/artifact-gate.sh all` reads **0 diffs**, and every diff seen on the
  way there is attributed and resolved in Corrections.
- The six letters A–F are archived to `planning/completed/`.

### Non-goals (explicit constraints)

- See plan-111-A §1 Non-goals — all apply.
- **The census may not be annotated.** No line gets a "boundary" note, an
  "acceptable residue" note, or a follow-up. Zero or unfinished.
- **The boundary list may not grow.** It is the five files in plan-111-A §2. If a
  violation cannot be removed, that is a blocker to state on line 1 with a repro,
  not a sixth boundary.
- Do not regenerate a golden to make the artifact gate pass without first
  attributing the diff (Phase 4). A regenerated golden that hid a real change is
  the failure mode this whole plan is a reaction to.

## 2. Current State

After letter F, the measured residue outside the five boundaries is:

### Measured populations

At HEAD (`fd09ea809`), 2026-08-29, tests excluded. These are the pre-plan-111
numbers for the areas no earlier letter claims; re-measure at kickoff, when they
should be unchanged (no letter A–F touches `src/target`).

| Site | What | Command |
|---|---|---|
| `src/target/shared/abi.rs:460` | `move_immediate(dst, type_: &str, value: &str)` | `rg -n 'fn move_immediate' src/target` |
| `src/target/shared/validate/body.rs:870` | `validate_type_name(type_: &str)` | `rg -n 'fn validate_type_name' src/target` |
| `src/target/shared/plan/lower.rs:169` | `n.resolve() == "Scalar"` | `rg -n 'resolve\(\) == "Scalar"' src/target` |
| `src/target/shared/plan/lower.rs:210` | `is_user_type_name(type_: &str)` | `rg -n 'fn is_user_type_name' src/target` |
| `src/target/shared/plan/lower.rs:222` | `type_ != "Unknown"` | `rg -n 'type_ != "Unknown"' src/target` |
| `src/target/shared/plan/symbols.rs:282` | `is_thread_type(type_: &str)` | `rg -n 'fn is_thread_type' src/target` |
| `src/binary_repr/sections.rs` | 19 spelling match arms, 4 grammar ops, 4 `&str` params | `rg -c` per plan-111-A §2 patterns |

`src/binary_repr/reader.rs`'s 5 `format!` type constructions are the `.mfp`
**decoder** rebuilding a spelling from wire ids — boundary #4, and they stay.

### Verified properties

- **`sections.rs` is an encoder matching a spelling, not a decoder.** Read
  `:130-230`: it maps `"Nothing" => TYPE_NOTHING`, `"Boolean" => TYPE_BOOLEAN`,
  … to wire type ids, and `:598-650` does the same for const entries. Its input
  is available upstream as a `ParameterType`; matching variants instead of
  spellings is a pure conversion, and it makes `is_structural` /
  `opaque_structural_kind` dead — those exist only to handle a spelling that
  failed to parse, which a `ParameterType` cannot be.
- **`src/target/shared/plan/lower.rs:169` already holds a `ParameterType`** —
  read it: `ParameterType::Named(n) if n.resolve() == "Scalar"`. It renders a
  `Symbol` to compare it, where `is_named` does the same structurally.
- **UNVERIFIED: whether `validate_type_name` is a boundary.** It validates a
  decoded name inside the target validator. Phase 1 task 1 reads it and decides:
  if its input comes from a `ParameterType`, it converts; if it validates a
  spelling arriving from outside, it is boundary-adjacent and the decision goes
  in Corrections **with the code that proves it** — not as an assumption.

## 3. Design Overview

Five phases: two small conversions, then the census, then the single artifact
run, then the archive. The census cannot run before the conversions and the
artifact run cannot precede the census — a census that reads zero is what makes a
byte-identity diff interpretable.

Where risk sits:

1. **Phase 4's attribution.** This is the plan's one deliberate accepted cost:
   because `artifact-gate.sh all` runs only here, a diff arrives without the
   "everything before this commit was clean" context that a per-landing gate
   gives. The mitigation is mechanical and is the phase's main content — a
   detached pre-plan-111 worktree, and per-diff classification against it.
2. **Phase 2's encoder conversion**, which touches `.mfp` bytes. The wire ids
   must be identical for every type; a changed id is an unreadable package.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial with one line on what remains; mark
> moot tasks `- [x] ~~text~~ — moot: <evidence>`; fill `Commit:` when a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — `src/target` residue (6 sites)

- [ ] Read `validate_type_name` (`src/target/shared/validate/body.rs:870`) and
      record its verdict — convert, or boundary-adjacent with the code that
      proves it. Do not leave it unclassified.
- [ ] `src/target/shared/plan/lower.rs:169` → `type_.is_named("Scalar")`.
- [ ] `src/target/shared/plan/lower.rs:210,222` — convert `is_user_type_name` to
      `&ParameterType` and its `!= "Unknown"` to a variant check.
- [ ] `src/target/shared/plan/symbols.rs:282` — convert `is_thread_type` to
      match `ParameterType::ThreadHandle { .. }`.
- [ ] `src/target/shared/abi.rs:460` — convert `move_immediate`'s `type_` to
      `&ParameterType`. This has many call sites; convert the signature and let
      the compiler enumerate them.
- [ ] **The three `optimizer` CONSUMERS of that same attribute**, added by
      plan-111-D Correction D1: `src/optimizer/opt2/constant_folding.rs:101`,
      `lvn.rs:143`, `gvn.rs:267` — each is
      `instruction.get("type").as_deref() == Some("Integer")`, reading the NIR
      `mov_imm` operand-class attribute that `move_immediate` writes. They were
      invisible until D1 taught `spelling_compares` the `== Some("X")` form, and
      they are in this phase because a producer and its consumers must convert
      together: changing `move_immediate`'s parameter without them leaves three
      reads matching a spelling that is no longer written the same way.
      The attribute is stored in a `HashMap<String, String>` instruction
      encoding, so the conversion is at the SET/GET pair, not the map.
- [ ] Lower the `target` **and `optimizer`** gate budgets to 0.

Acceptance: `src/target` **and `src/optimizer`** read 0 on all six needle
classes; `cargo test --no-fail-fast -- --skip artifact_gate_all` green.
Commit: —

### Phase 2 — the `.mfp` encoder takes a type, not a spelling

- [ ] Convert `binary_repr::sections`'s type→wire-id mapping (`:130-230`) and its
      const-entry mapping (`:598-650`) to match `ParameterType` variants.
- [ ] **Delete `is_structural` and `opaque_structural_kind`** (`:85-116`) — with
      a typed input, "a spelling that did not parse" is not a reachable state.
- [ ] Convert the 4 `&str` type params and remove the 4 grammar ops.
- [ ] Keep `binary_repr/reader.rs`'s 5 `format!` reconstructions — boundary #4,
      the decoder.
- [ ] Lower the `binary_repr` gate budgets to 0 for every class except the
      boundary-file exemptions.
- [ ] Tests: a `.mfp` round-trip test writing and reading back every wire type
      id, including the nested-`Map`-key case from plan-106-E Correction 3 and a
      stateful resource (`File STATE Cursor`), asserting **the same wire ids as
      before this phase** — record them.

Acceptance: every wire type id is byte-identical to the pre-phase values
(recorded in the test); a package built before this phase still decodes;
`cargo test --no-fail-fast -- --skip artifact_gate_all` green.
Commit: —

### Phase 3 — the terminal scan, and locking the gate

The step the plan exists to reach.

- [ ] Run every census line in §"The terminal census" below and paste the full
      result — command, count, and, for any non-zero line, the file:line list.
- [ ] **Any non-zero line is unfinished work.** Fix it in this phase. Do not
      annotate it, do not add a boundary, do not open a follow-up.
- [ ] Delete `BUDGETS` from `tests/no_type_strings.rs`. Replace every budget
      assertion with `assert_eq!(count, 0, …)` printing the offending file:line
      list on failure.
- [ ] Add `boundary_list_is_exactly_five` — a test pinning `BOUNDARY_FILES` to
      the five files in plan-111-A §2, so a sixth cannot be added without
      deliberately editing a test that says why the list is closed.
- [ ] Update the gate's header doc comment from "ratchet, budgets shrink per
      letter" to the hard-floor statement, mirroring
      `tests/architecture_guards.rs`'s "hard floor of 0" language.
- [ ] Re-run the census one final time **after** the gate edit and paste the
      result again, so the recorded numbers are the post-lock ones.

Acceptance: every line of the terminal census reads 0;
`cargo test --test no_type_strings` passes with no budget table; lowering any
count is impossible because there is nothing left to lower.
Commit: —

### Phase 4 — the single byte-identity sweep: attribute, then regenerate once

- [ ] Create the attribution worktree:
      `git worktree add --detach ../mfb-pre111 <commit before letter A>`, and
      build its release binary.
- [ ] Run `scripts/artifact-gate.sh all` (equivalently `cargo test --test golden`
      with no `--skip`). Record the diff count. **This is the first byte-level
      check since letter A**, so expect a list, not a clean run — that is the
      plan's design, not a failure.
- [ ] For **each** diff: build the fixture with the pre-plan-111 binary.
      Baseline output == committed golden → the diff is plan-111's; find and fix
      the conversion that caused it (objdump one fixture to localize). Baseline
      != committed golden → pre-existing; leave the golden, and record it in
      Corrections with the evidence.
- [ ] **Regenerate goldens once, and only after attribution.** For diffs
      classified as pre-existing, or as letter E's ` TO `-split bug fix (the one
      output change plan-111 sanctions), regenerate with
      `scripts/sync-goldens.sh` / `scripts/regen-ncodesum.sh` and list every
      regenerated golden in the commit. For diffs classified as plan-111 bugs,
      **fix the conversion — do not regenerate.** Regenerating an unattributed
      diff is how a broken conversion ships behind a green gate.
- [ ] Re-run `scripts/artifact-gate.sh all` until it reads **0 diffs**, with
      every intermediate diff accounted for in Corrections.
- [ ] Goldens outside `tests/byte-identity/` need **hand**-regeneration —
      `regen-ncodesum.sh` misses them (the editing-package.mfb memory). Budget
      for this; it is the slowest step in the letter.
- [ ] Run the full acceptance sweep — also its first run since letter A:
      `scripts/test-accept.sh` with scratch `/tmp/accept-111g` (never `tests/`;
      the second argument is an `rm -rf` target) and
      `MFB_OPT=3 scripts/test-accept.sh`. Record the `N ran` count and compare it
      against the same command run on the pre-plan-111 worktree — a dropped count
      means fixtures were silently skipped, which no per-letter run was there to
      catch.
- [ ] Run `scripts/diag-set-diff.sh` and record 0 differing, with `[exit N]` and
      bare `error:` lines captured.

Acceptance: `scripts/artifact-gate.sh all` → 0 diffs; both acceptance sweeps at
baseline `N ran` with 0 mismatches; `diag-set-diff.sh` 0 differing; every diff
encountered is classified in Corrections.
Commit: —

### Phase 5 — docs and archive

- [ ] Update `src/docs/spec/architecture/21_type-name-encoding.md` — the type
      spelling is now a *rendering and wire format*, not an internal
      representation.
- [ ] Update `src/docs/spec/architecture/02_frontend.md`, `04_ir.md`,
      `13_native-ir.md` for the typed pipeline end state.
- [ ] Update `.ai/codegen-invariants.md`, `.ai/collections.md`,
      `.ai/resources-packages.md`, `.ai/testing-gates.md`: the one-type-grammar
      rule is now enforced by `tests/no_type_strings.rs`, and the five boundaries
      are named there.
- [ ] Remove any stale comment in `src/` describing a "name-domain twin" or a
      permitted re-parse; grep for `name-domain` and `re-parse`.
- [ ] Move `planning/plan-111-A` … `planning/plan-111-G` to
      `planning/completed/`, with the baseline artifacts.
- [ ] Delete the `../mfb-pre111` attribution worktree (`--force` if needed).

Acceptance: the seven letters are in `planning/completed/`; no `src/` comment
claims a permitted type-string re-parse; `cargo test --no-fail-fast` green — **with no `--skip`**, since Phase 4 has run.
Commit: —

## The terminal census

Run in Phase 3, then again after the gate is locked. **Every line must read 0.**
Paste the command, the count, and — for any non-zero line — the full file:line
list, then fix it. Do not annotate.

Shared prefix `r` = `rg -n --glob '!**/tests*' --glob '!**/*_tests.rs' --glob '!src/testutil.rs' --glob '!src/docs/**' --glob '!src/ast/**' --glob '!src/lexer.rs'`,
and the five boundary files from plan-111-A §2 excluded per line 1.

| # | Line | Command | Was (2026-08-29) | Now |
|---|---|---|---|---|
| 1 | `ParameterType::parse` outside the five boundaries | `r 'ParameterType::parse\(' src/` minus boundary files | 155 | **must be 0** |
| 2 | Type-as-`&str` parameters | `r '\b(type_\|type_name\|element_type\|value_type\|key_type\|field_type\|return_type\|declared_type\|target_type\|source_type\|state_type\|param_type\|arg_type\|base_type\|union_type\|member_type\|collection_type\|scrutinee_type)\s*:\s*&(\x27[a-z]+ )?str' src/` | 185 | **must be 0** |
| 3 | Match arms on a type spelling | `r '^\s*"(Integer\|String\|Boolean\|Float\|Fixed\|Byte\|Money\|Nothing\|AttributeString\|Scalar\|Unknown\|Error)"( \| "[A-Za-z]+")* =>' src/` | 186 | **must be 0** |
| 4 | `==`/`!=` against a type spelling | `r '[!=]= "(Integer\|String\|Boolean\|Float\|Fixed\|Byte\|Money\|Nothing\|AttributeString\|Scalar\|Unknown\|Error\|Result)"' src/` | 73 | **must be 0** |
| 5 | Hand-rolled grammar ops on a type spelling | `r '(split_once\|strip_prefix\|strip_suffix\|starts_with\|ends_with\|contains)\("( STATE \| TO \| OF \|List OF\|Set OF\|Map OF\|Result OF\|MapEntry OF\|RES \|Thread OF\|ThreadWorker OF\|FUNC\(\|ISOLATED FUNC\()' src/` | 57 | **must be 0** outside `ParameterType::parse` |
| 6 | `format!` type construction | `r 'format!\("(List OF\|Set OF\|Map OF\|Result OF\|MapEntry OF\|Thread OF\|ThreadWorker OF\|RES )' src/` | 15 | **must be 0** outside `name()` and the `.mfp` decoder |
| 7 | Type-valued `String`-keyed maps | `r '(HashMap\|BTreeMap)<String,' src/` filtered to type maps | 7 in `TypeModel` | **must be 0** |
| 8 | Second type grammar | `rg -n 'fn split_state_clause' src/` | 2 | **must be 0** |
| 9 | Front-end → codegen `&str` type helpers | `r 'codegen::(engine::types\|resource)::' src/ir src/hir src/monomorph src/resolver src/cli` | 13 | **must be 0** |

The "Was" column is plan-111-A §2's measurement; the "Now" column is filled in
Phase 3 with the real number, which is 0 or the plan is not done.

## Validation Plan

- Tests: `cargo test --no-fail-fast` — **with no `--skip` in this letter**, so
  `artifact_gate_all` runs. Letters A–F skipped it (plan-111-A §3); G is where
  it comes back, in Phase 4.
- Gate: `cargo test --test no_type_strings` — no budget table; hard zeros;
  `boundary_list_is_exactly_five` passing.
- Coverage check: the gate test itself must be proven to fire — reintroduce one
  violation in a scratch commit, watch the gate fail with its file:line, revert.
  A gate that cannot fail is not a gate.
- Runtime proof: `scripts/test-accept.sh` and `MFB_OPT=3 scripts/test-accept.sh`,
  both with scratch `/tmp/accept-111g`, at the pre-plan-111 `N ran` with 0
  mismatches after regeneration.
- Artifact gate: `scripts/artifact-gate.sh all` → **0 diffs**. This is plan-111's
  single run and its final acceptance check.
- Diagnostics: `scripts/diag-set-diff.sh` → 0 differing, `[exit N]` captured.
- Doc sync: the spec and `.ai/` files listed in Phase 5.
- Formatting: `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **`validate_type_name`'s classification** — recommended: convert it. It sits
  inside the target validator, downstream of every typed layer, so its input
  should already be a `ParameterType`. Resolve in Phase 1 from the code, and if
  it genuinely validates an externally-arriving spelling, record that in
  Corrections with the evidence rather than adding a sixth boundary. (§Phase 1)

## Corrections

<Filled in DURING execution. Every artifact-gate diff and its attribution belongs
here, with the pre-plan-111 baseline output that classified it.>

## Summary

The engineering risk is Phase 4's attribution, which is the accounted-for cost of
running the cross-target gate once rather than per landing: a diff arrives without
a clean-before context, so it must be classified against a pre-plan-111 binary.
Phase 2's wire-id conversion is the other real risk — a changed id is an
unreadable `.mfp`, which the round-trip test pins.

What is left untouched after this letter: `src/ast/**` and `src/lexer.rs` (the
AST is the string domain, by design) and the five boundary files, whose job is
converting between the string world and the type world. Everything between
`hir::elaborate` and the emitted byte is typed, and `tests/no_type_strings.rs`
keeps it that way.
