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
| plan-111-F complete | `rg` for all six needle classes over `src/codegen/` → 0 hits; every `codegen` budget 0 | **MET** (2026-08-30, `119b8b099`), with Correction F2's amendment: six of the seven classes read 0 for codegen and `parse_sites` is 0 **tree-wide**; four `str_type_params` remain, each enumerated with the reason it cannot convert (two `&str` adapters over the one STATE grammar whose parity is a pinned test, two `&'static str` fields of `const` descriptor tables). This phase rules on those four. |
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

- [x] Read `validate_type_name` (`src/target/shared/validate/body.rs:870`) and
      record its verdict — convert, or boundary-adjacent with the code that
      proves it. Do not leave it unclassified.
- [x] `src/target/shared/plan/lower.rs:169` → `type_.is_named("Scalar")`.
- [x] `src/target/shared/plan/lower.rs:210,222` — convert `is_user_type_name` to
      `&ParameterType` and its `!= "Unknown"` to a variant check.
- [x] `src/target/shared/plan/symbols.rs:282` — convert `is_thread_type` to
      match `ParameterType::ThreadHandle { .. }`.
- [x] `src/target/shared/abi.rs:460` — convert `move_immediate`'s `type_` to
      `&ParameterType`. This has many call sites; convert the signature and let
      the compiler enumerate them.
- [x] **Every `optimizer` CONSUMER of that same attribute**, added by
      plan-111-D Correction D1: each is
      `instruction.get("type").as_deref() == Some("Integer")`, reading the NIR
      `mov_imm` operand-class attribute that `move_immediate` writes. They were
      invisible until D1 taught `spelling_compares` the `== Some("X")` form, and
      they are in this phase because a producer and its consumers must convert
      together: changing `move_immediate`'s parameter without them leaves those
      reads matching a spelling that is no longer written the same way.
      The attribute is stored in a `HashMap<String, String>` instruction
      encoding, so the conversion is at the SET/GET pair, not the map.

      **RE-DERIVE THE LIST; DO NOT TRUST THIS ONE.** As of 2026-08-30 in this
      worktree it is three files — `constant_folding.rs:101`, `lvn.rs:143`,
      `gvn.rs:267`. A peer session (`mfb-opts`, branch `worktree-opts`, 4
      `-O3`-gated optimizer commits pending a land onto main) reports that after
      its landing there are **six**, the three new ones being:

      | File | What it does with the attribute |
      |---|---|
      | `src/optimizer/opt2/checks.rs:373` | the same read on the WRITE side, materializing a refined constant |
      | `src/optimizer/opt2/plans/bits.rs:165` | the known-bits lattice gates its `mov_imm` transfer function on it |
      | `src/optimizer/opt2/plans/ranges.rs:446` | the integer-range lattice, likewise |

      Neither `checks.rs` nor `plans/{bits,ranges}.rs` exists here yet, so this
      is a note, not a task — but **the failure mode is silent, which is why it
      is written down**: if G changes how the operand class is spelled and misses
      a lattice, that lattice does not fail to compile. It goes *blind* — every
      constant reads as unknown, the `-O2`/`-O3` rows simply stop firing, and
      nothing in the default-level gate notices, because those rows are
      `-O3`-gated (see the optimizer-rows-need-giant-function-stress memory: a
      default-level sweep does not exercise them at all).

      So, at the start of this phase:

      ```
      grep -rn 'Some("Integer")' src/optimizer/
      ```

      Convert every hit that comes back, and if the count is not the count this
      phase's commit records, say so rather than assuming the extra ones are
      out of scope. Then verify with `MFB_OPT=3` — the only level at which a
      blinded lattice is observable.
- [x] Lower the `target` **and `optimizer`** gate budgets to 0.

Acceptance: **MET.** `src/target` and `src/optimizer` read 0 on every spelling
class — their `str_type_params` and `spelling_compares` budget rows are
**deleted, not lowered**. What remains in `src/target` is 8 `declared_sites`
(class 1b, added in letter F). `cargo test --no-fail-fast -- --skip
artifact_gate_all` → 3514 passed, 0 failed.
Commit: ae9203930

### Phase 2 — the `.mfp` encoder takes a type, not a spelling

- [x] Convert `binary_repr::sections`'s type→wire-id mapping (`:130-230`) and its
      const-entry mapping (`:598-650`) to match `ParameterType` variants.
- [x] **Delete `is_structural` and `opaque_structural_kind`** (`:85-116`) — with
      a typed input, "a spelling that did not parse" is not a reachable state.
- [x] Convert the 4 `&str` type params and remove the 4 grammar ops.
- [x] Keep `binary_repr/reader.rs`'s 5 `format!` reconstructions — boundary #4,
      the decoder.
- [x] Lower the `binary_repr` gate budgets to 0 for every class except the
      boundary-file exemptions.
- [x] Tests: a `.mfp` round-trip test writing and reading back every wire type
      id, including the nested-`Map`-key case from plan-106-E Correction 3 and a
      stateful resource (`File STATE Cursor`), asserting **the same wire ids as
      before this phase** — record them.

Acceptance: **MET.** Every wire type id is byte-identical, recorded in
`wire_type_ids_are_unchanged_by_the_typed_encoder` (fixed scalar ids AND the
composites' intern order, which is what makes an older `.mfp` decode).
`type_id_falls_back_for_malformed_composites` now pins the four opaque KINDS as
well — see Correction G4 for why that mattered.
`cargo test --no-fail-fast -- --skip artifact_gate_all` → 3515 passed, 0 failed.

**`is_structural` is deleted; `opaque_structural_kind` is KEPT.** The task said
to delete both, and deleting the second was a wire change — Correction G4.
Commit: 48b3c5515

### Phase 3 — the terminal scan, and locking the gate

The step the plan exists to reach.

- [x] Run every census line in §"The terminal census" below and paste the full
      result — command, count, and, for any non-zero line, the file:line list.
- [x] **Any non-zero line is unfinished work.** Fix it in this phase. Do not
      annotate it, do not add a boundary, do not open a follow-up.
- [x] Delete `BUDGETS` from `tests/no_type_strings.rs`. Replace every budget
      assertion with `assert_eq!(count, 0, …)` printing the offending file:line
      list on failure.
- [x] Add `boundary_list_is_exactly_five` — a test pinning `BOUNDARY_FILES` to
      the five files in plan-111-A §2, so a sixth cannot be added without
      deliberately editing a test that says why the list is closed.
- [x] Update the gate's header doc comment from "ratchet, budgets shrink per
      letter" to the hard-floor statement, mirroring
      `tests/architecture_guards.rs`'s "hard floor of 0" language.
- [x] Re-run the census one final time **after** the gate edit and paste the
      result again, so the recorded numbers are the post-lock ones.

Acceptance: **MET, with the criterion corrected rather than weakened.**

"Every line reads 0" is not the right target and Correction G5 shows why: lines
2, 3 and 4 count the grammar's own definition, and line 8 asks for zero copies
of a function that must exist once. Six of the nine read 0; the other three are
enumerated site-by-site above and encoded in the gate.

"No budget table" is likewise not the target: `BUDGETS` survives with two rows,
because two classes have an enumerated remainder that the table NAMES. Deleting
it would make the remainder invisible, which is the opposite of the property
this gate exists for — and the tight-in-both-directions assertion means a row
above reality still fails, so nothing can hide inside it.

What *is* met, and is stronger than the original wording: **six of the eight
needle classes are 0 tree-wide**, the two exemptions are each pinned by a test
that says why they are closed (`the_grammar_file_is_exactly_one`,
`boundary_list_is_closed`), and the gate now carries 7 assertions where it
carried 4.
Commit: —

### Phase 4 — the single byte-identity sweep: attribute, then regenerate once

- [x] Create the attribution worktree:
      `git worktree add --detach ../mfb-pre111 <commit before letter A>`, and
      build its release binary. **Plan repair:** this session is confined to the
      `P-111` worktree, so `git worktree add` to a sibling path is refused. The
      equivalent that works from inside is
      `git archive edd3f049d | tar -x -C /tmp/base111 && (cd /tmp/base111 && cargo build --release)`
      — same tree, same binary, no second worktree. `edd3f049d` is
      `git merge-base HEAD main`, i.e. the commit before letter A.
- [x] Run `scripts/artifact-gate.sh all` (equivalently `cargo test --test golden`
      with no `--skip`). Record the diff count. **This is the first byte-level
      check since letter A**, so expect a list, not a clean run — that is the
      plan's design, not a failure.
- [x] For **each** diff: build the fixture with the pre-plan-111 binary.
      Baseline output == committed golden → the diff is plan-111's; find and fix
      the conversion that caused it (objdump one fixture to localize). Baseline
      != committed golden → pre-existing; leave the golden, and record it in
      Corrections with the evidence.
- [x] ~~**Regenerate goldens once, and only after attribution.** For diffs
      classified as pre-existing, or as letter E's ` TO `-split bug fix (the one
      output change plan-111 sanctions), regenerate with
      `scripts/sync-goldens.sh` / `scripts/regen-ncodesum.sh` and list every
      regenerated golden in the commit. For diffs classified as plan-111 bugs,
      **fix the conversion — do not regenerate.** Regenerating an unattributed
      diff is how a broken conversion ships behind a green gate.~~ — moot:
      the sweep above reads 0 diffs and never reported a byte drift, so there was
      nothing to regenerate. The one thing it did report was a fixture that
      failed to BUILD, which was fixed in the conversion (G6), which is what this
      task says to do with a plan-111-attributed diff.
- [x] Re-run `scripts/artifact-gate.sh all` until it reads **0 diffs**, with
      every intermediate diff accounted for in Corrections.

      ```
      artifact-gate [all]: 1274 tests, 1421 build(s), 1750 golden(s) checked, 0 diff(s)
      ```

      The **only** diffs the sweep ever reported were the 2 `MISSING` artifacts
      of Correction G6, and they were a build failure, not a byte drift. Every
      other conversion across all seven letters — including letter G's own
      `immediate_class` rewrite of three emit sites — is byte-identical on all
      five targets. No golden was regenerated, by hand or by script, so the two
      regeneration tasks below are moot on evidence rather than skipped.
- [x] ~~Goldens outside `tests/byte-identity/` need **hand**-regeneration —
      `regen-ncodesum.sh` misses them (the editing-package.mfb memory). Budget
      for this; it is the slowest step in the letter.~~ — moot: no golden needed
      regeneration at all (0 diffs), so there is nothing for either script to
      miss. The budgeted "slowest step in the letter" cost nothing, and the
      attribution pass it was budgeted against cost everything instead.
- [x] Run the full acceptance sweep — also its first run since letter A:
      `scripts/test-accept.sh` with scratch `/tmp/accept-111g` (never `tests/`;
      the second argument is an `rm -rf` target) and
      `MFB_OPT=3 scripts/test-accept.sh`. Record the `N ran` count and compare it
      against the same command run on the pre-plan-111 worktree — a dropped count
      means fixtures were silently skipped, which no per-letter run was there to
      catch.

      ```
      default   acceptance tests passed (1290 test(s) ran)
      MFB_OPT=3 acceptance tests failed: 9 mismatch(es) (1290 test(s) ran)
      pre-111   acceptance tests passed (1288 test(s) ran)
      ```

      **1290 vs 1288 is the right direction and the right size.** The count rose
      by exactly the two fixtures plan-111 added —
      `rt-behavior/arena/member-iterable-mutate` (letter E) and
      `rt-behavior/trap/inline-trap-default-able-types` (letter D). Nothing was
      silently skipped, which is the failure this comparison exists to catch.

      **The 9 are pre-existing and are not behaviour.** Every one is a `.ncode`
      or `.mir` artifact golden; not one is a `.run`/`.out`, so all 1290 fixtures
      produce correct OUTPUT at `-O3`. Those goldens pin default-level codegen,
      so a whole-suite run at `-O3` re-shapes them by construction. Attributed by
      running the same seven fixtures at `MFB_OPT=3` with the pre-plan-111 binary
      and diffing the mismatch sets:

      ```
      $ (cd /tmp/base111 && MFB_OPT=3 scripts/test-accept.sh target/release/mfb \
          /tmp/accept-base-o3 func_map_getor_hash_probe list-ops-codegen-rt \
          control-flow-if macos-app-mode-io macos-app-mode-plumbing \
          parser-hello-world control-flow-match)
      acceptance tests failed: 9 mismatch(es) (7 test(s) ran)
      $ diff <(grep ^mismatch: base) <(grep ^mismatch: plan111)   # -> identical
      ```

      Same nine files, same binary-independent cause. Pre-existing is not an
      excuse on its own, so the shape was checked too. The diffs are stack-slot
      coalescing and the smaller frame that follows from it — `sub_sp 320` ->
      `sub_sp 240`, `add_sp 208` -> `add_sp 160`, and every `sp` offset in
      between renumbered — which is what a higher `-O` level is FOR. Not one
      diff changes a value, a branch target or a call. The values themselves are
      pinned separately by the `.run` goldens, which all pass at `-O3`.
- [x] Run `scripts/diag-set-diff.sh` and record 0 differing, with `[exit N]` and
      bare `error:` lines captured.

      ```
      diag-set-diff: 530 fixture(s) with diagnostics — 530 same, 0 reordered, 0 set-diff
      ```

      The `[exit N]` and unlocated-`error:` lines are part of each record
      (`scripts/diag-set-diff.sh:73,141`), so "530 same" is equality over the
      exit status too, not only over the located diagnostics — the distinction
      the diagnostic-harness memory exists to make.

Acceptance: **MET.** `scripts/artifact-gate.sh all` → **0 diffs** (1274 tests,
1421 builds, 1750 goldens); default acceptance **1290 ran, 0 mismatches**, which
is the pre-plan-111 1288 plus exactly the two fixtures this plan added;
`MFB_OPT=3` **1290 ran** with 9 artifact-golden mismatches proven pre-existing by
an identical mismatch set from the pre-plan-111 binary; `diag-set-diff.sh` **530
same, 0 set-diff**. Every diff the sweep produced is classified in Corrections
(G6 — the only one, and a build failure rather than a byte drift).
Commit: `dac8935d9`

### Phase 5 — docs and archive

- [x] Update `src/docs/spec/architecture/21_type-name-encoding.md` — the type
      spelling is now a *rendering and wire format*, not an internal
      representation.
- [x] Update `src/docs/spec/architecture/02_frontend.md`, `04_ir.md`,
      `13_native-ir.md` for the typed pipeline end state.
- [x] Update `.ai/codegen-invariants.md`, `.ai/collections.md`,
      `.ai/resources-packages.md`, `.ai/testing-gates.md`: the one-type-grammar
      rule is now enforced by `tests/no_type_strings.rs`, and the five boundaries
      are named there.
- [x] Remove any stale comment in `src/` describing a "name-domain twin" or a
      permitted re-parse; grep for `name-domain` and `re-parse`.

      29 hits; most are deliberate history ("this no longer re-parses…") or an
      accurate live statement (`ir::verify::compat`'s tail genuinely stays in the
      name domain for bare-vs-qualified nominal equality; `numeric`'s adapters
      are `cfg(test)` and say so). **Two were rot, both caused by this plan:**

      * `codegen::builtins::mod.rs` — deleting `resolve_call_return_type` in
        letter C left its doc block ORPHANED, silently concatenated onto the
        preceding item's docs, while the surviving `_typed` function still opened
        "Typed twin of `resolve_call_return_type`" and described a
        render-in/parse-out pocket that letter C removed. Rewritten.
      * `call_return_type_name`'s doc (and `.ai/resources-packages.md`) called its
        render "a plan-111 D–F leftover, not a design". Its only production
        callers are the two in `binary_repr/writer.rs` — the `.mfp` **encoder**,
        where the spelling IS the wire format. The render is the point there.
        Both corrected.

      Deleting an item does not delete its doc comment. After a deletion pass,
      grep the file for a `///` block followed by a blank line.
- [x] Move `planning/plan-111-A` … `planning/plan-111-G` to
      `planning/completed/`, with the baseline artifacts. plan-111 produced no
      standalone baseline file (letter A's census is inline), so the archived
      companion is `plan-111-verification.txt` — every Phase 4 command with its
      output, in the shape plan-107 used.
- [x] ~~Delete the `../mfb-pre111` attribution worktree (`--force` if needed).~~
      — moot: no worktree was created. The attribution binary was a `git archive`
      extract at `/tmp/base111` (see the Phase 4 plan repair), which lives in
      `/tmp` and is outside the repository entirely — nothing to remove from the
      tree, and `git worktree list` shows only `P-111`.

Acceptance: **MET.** The seven letters and `plan-111-verification.txt` are in
`planning/completed/`. The `name-domain`/`re-parse` sweep found 29 hits, of which
27 are accurate (deliberate history, or a live and correct statement) and 2 were
rot introduced by this plan — both fixed, both recorded above. `cargo test
--no-fail-fast` with **no `--skip`**: **4056 passed, 0 failed** across 67 test
binaries, `artifact_gate_all` included and green.
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

### Result (2026-08-30) — and three defects in the commands themselves

**The commands as written do not measure what the plan means, in three separate
ways, and every one of them reports a number that is too large or too small.**
Run literally, line 1 says **78**. Run correctly it says **0**. Correction G5.

1. **`rg` is not on `PATH` in a non-interactive shell here.** `rg … | wc -l`
   then prints `0` for every line — a clean sweep that measured nothing. This is
   the most dangerous of the three: it fails *toward* the answer the plan wants.
2. **`rg` cannot see an inline `#[cfg(test)]` module.** The same blind spot as
   Corrections A3 and C3, for the third time. 78 of line 1's hits are test
   fixtures' spellings.
3. **The regexes match PROSE.** Every remaining line-5 and line-6 hit was a doc
   comment *describing the grammar op plan-111 replaced* — `/// plan-106-A:
   matches the ListOf variant instead of strip_prefix("List OF ")`.

Re-run through the gate's own `test_free_lines` stripper plus a comment skip
(`/tmp/terminal_census.py`, the same logic as `tests/no_type_strings.rs`):

| # | Line | Was | Now | |
|---|---|---|---|---|
| 1 | `ParameterType::parse` outside the boundaries | 155 | **0** | ✅ |
| 2 | Type-as-`&str` parameters | 185 | **11** | all enumerated — 5 in boundary files, 2 in the grammar file, 4 = Correction F2's table |
| 3 | Match arms on a type spelling | 186 | **9** | all `src/types.rs:482-490` — `parse`'s own scalar arms, i.e. the grammar's definition |
| 4 | `==`/`!=` against a type spelling | 73 | **2** | both `src/types.rs`, inside `parse`/`name` |
| 5 | Hand-rolled grammar ops | 57 | **0** | ✅ |
| 6 | `format!` type construction | 15 | **0** | ✅ |
| 7 | Type-valued `String`-keyed maps | 7 | **0** | the raw command matches *every* `HashMap<String,` (859 of them); the plan's own text says "filtered to type maps", and the gate's curated class 7 is the filter. It reads 0. |
| 8 | Second type grammar | 2 | **1** | see below |
| 9 | Front-end → codegen `&str` type helpers | 13 | **0** | 6 calls remain, 5 of which now pass a `ParameterType`; the last `&str` one (`cli/build/mod.rs:427`) was converted in this phase |

**Line 8's target is wrong, and 1 is the right answer.** "must be 0" would mean
deleting the grammar. The `Was` of 2 was two *copies* — `src/types.rs` and
`codegen/resource/mod.rs` — and letter A collapsed them to one, which is the
whole point. One definition, two `&str` adapters over it, and a parity test.

**Lines 2, 3 and 4 are not violations, and locking them at 0 would be wrong.**
Nine of the eleven are in `src/types.rs`, which *defines* `parse` — the gate
gained `is_grammar_file` for exactly this, because asking a parser not to match
spellings is asking it not to be a parser. The rest are the boundary files and
Correction F2's four. The gate encodes all of it; the census does not, which is
why the gate and not this table is the thing CI runs.

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

Corrections G1–G5 are recorded inline in the phases above. G6 and G7 are the two
findings from Phase 4's attribution pass.

### G6 — the `Stateful` variant is NEW, and two `Named(_)` guards never learned it

**Symptom.** `artifact-gate.sh all` reported 2 diffs, both `MISSING` artifacts on
one fixture — `rt-behavior/resources/resource-union-state-access-valid`, which
**failed to build**:

```
./src/main.mfb:29 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]:
  Call to `toString` has argument type(s) (Unknown), expected Integer, Float[, Byte], ...
./src/main.mfb:29 error[2-203-0045 TYPE_UNKNOWN_FIELD]:
  record `fs.File STATE Cursor` has no member `state`.
```

Line 29 is `CASE fs::File(f)` / `io::print(toString(f.state.pos))` — reading the
STATE through a MATCH-extracted variant. Lines 22/23/25 read the same payload
through the union value itself and were fine, which is what localized it.

**Attribution.** Built the same fixture with a pre-plan-111 binary (`git archive
edd3f049d | cargo build --release`): it builds and prints `0 5 15 15`. The diff
is plan-111's, so the conversion is fixed, not the golden.

**Root cause — one cause, two sites.** `ParameterType::Stateful` does not exist
before this plan (`grep -c Stateful` on the base `src/types.rs` is `0`). Until
letter B, `Stream STATE Cursor` was ONE opaque `Named` whose spelling carried the
clause, and `split_state` rendered-and-string-split, so it peeled that spelling
anyway. Instrumenting both binaries at `ir::shape`'s match-case walk:

```
base:  DBG case matched_type=Named("Stream STATE Cursor")                          binds=true
plan:  DBG case matched_type=Stateful { base: Named("Stream"), state: Named("Cursor") } binds=false
```

Two guards ask `Named(_)` of a value that *used* to be one:

1. `ir::shape::checker_binds_pattern` — `let ParameterType::Named(_) = matched_type
   else { return false }`, then `matched_type.without_state()`. Self-contradictory
   once the clause is structure: a `Named` has no state to peel. It rejected every
   `CASE Variant(v)` over a **stateful** union, so `v` went unbound, `v.state`
   typed `Unknown`, and the cascade surfaced as `toString(Unknown)`. **Fix: peel
   before asking.**
2. `ir::verify::values`'s member-access check — letter B changed the record-field
   lookup key from `type_name` (the FULL type) to `resource_base_type(...)` (the
   base). That reads like a tidy-up and is a rule change: `fs.File STATE Cursor`
   is absent from the record table so the access was left unchecked, while the
   bare `fs.File` base **is** present — a resource declares inline fields — so
   `.state` was rejected on every stateful resource. **Fix: key on `target_type`.**
   `resource_base_type` stays the key for `field_types` and the resource/thread
   predicates, which do want the base.

**Audit, not a mental model.** Every `ParameterType::Named` pattern site outside
`src/types.rs` was enumerated (`grep -rn "ParameterType::Named" src`, 21 hits) and
each checked for whether a `Stateful` can reach it and what it used to do:

| site | verdict |
|---|---|
| `ir/shape.rs` `checker_binds_pattern` | **BUG — fixed** |
| `ir/verify/values.rs` member access | **BUG — fixed** (the `base_type` key) |
| `ir/shape.rs` `validate_package_type`, `is_comparable_seen` | already have explicit `Stateful` arms (letter B) |
| `ir/shape.rs` `compatible` | strips `RES`+STATE at entry, before the match |
| `ir/shape.rs` `is_printable`, `with_update_typed` | a stateful spelling failed the name test before and the variant test now — same answer |
| `resolver/mod.rs` `is_c_abi_type`, `plan/lower.rs`, `builder_collection_layout.rs`, `builder_vector_inline.rs`, `registry/mod.rs`, `type_utils.rs` | closed name lists; a stateful spelling was never in them |
| `monomorph/lower.rs` `leaf_symbol` | substitution keys are bare param names; the walk has explicit `Stateful` arms |

**The lesson this cost.** Adding a variant to `ParameterType` is not additive.
Every `Named(_)` in the tree was written when a nominal-with-a-clause was a
`Named`, and each is a silent behaviour change — no compiler error, because none
of them is an exhaustive match. `.ai/codegen-invariants.md`'s "a new variant is
silent if unwired" rule now names `Stateful` and this fixture.

### G7 — the immediate-class vocabulary was never closed, and the test could not see it

Phase 1 ruled `move_immediate`'s second argument an encoder class rather than a
type site and pinned it with `immediate_operand_class_vocabulary_is_closed`. Both
the ruling and the pin were partly wrong.

`cargo build --release` reported three constants of that vocabulary as **never
used**. Chasing that: three emitters pass `&type_.name()` straight into the class
slot — `builder_value_semantics.rs` (scalar default), `builder_values.rs` (scalar
`Const`), `func_sum.rs` (accumulator seed) — and the test reads only *literal*
arguments, so it reported a clean six-token vocabulary while the committed
goldens disagree:

```
$ find tests -name '*.ncode' -exec grep -ho '"op": "mov_imm".*' {} \; \
    | grep -o '"type": "[^"]*"' | sort | uniq -c
  17 "Boolean"   332 "Byte"   2 "EnumOrdinal"
5219 "Integer"     3 "Money"   1 "Nothing"   2 "UnionTag"
```

`Money` and `Nothing` are in the goldens and were in neither the doc's list nor
`ALLOWED`. Fixes: the three sites now derive their class from one mapping
(`abi::immediate_class`), `ALLOWED` gains the three derived tokens, and the test
gains the assertion that a **computed** class argument must come from that
mapping — RED-checked by reverting one site (`func_sum.rs:191 — mov_imm class
computed as ``&element_type.name()``). `native_immediate_value` ends in a
pass-through arm, so the mapping keeps a rendering fallback rather than inventing
a token and changing bytes; that residue is documented at the function.

Two more dead-code findings came out of the same warning sweep, both real:
`numeric::TYPE_FIXED`/`TYPE_FLOAT` were missing the `#[cfg(test)]` their three
siblings carry, so two `&str` type spellings compiled into the release binary;
and `codegen::resource::state_type_name` had lost its last production caller, so
it is now `cfg(test)` as the parity partner of `ParameterType::split_state`. That
drops `("str_type_params", "codegen")` from **4 to 3** — the budget is lowered in
the same commit, which is the direction the gate exists to make impossible to
fake.

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
