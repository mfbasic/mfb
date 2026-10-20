# plan-180-B: `EXIT`/`CONTINUE` take only `FOR`, `DO`, `WHILE`

Last updated: 2026-10-19
Effort: medium (1h–2h)
Depends on: plan-180-A (prerequisites are stated there)

After plan-180-A only `EXIT FOR|DO|WHILE` and `EXIT FUNC` remain. This letter makes the
three loop kinds the whole `EXIT` and `CONTINUE` vocabulary: `EXIT <anything else>` is
`EXIT_UNKNOWN` (the renamed `EXIT_FUNC_FORBIDDEN`), and `CONTINUE <anything else>` is the
new `CONTINUE_UNKNOWN`. The AST/HIR lose `ExitTarget`; both statements carry a `LoopKind`.

Observable outcome: `EXIT FUNC`, `EXIT PROGRAM`, `EXIT` alone and `EXIT foo` all report
`EXIT_UNKNOWN`; the same four spellings under `CONTINUE` report `CONTINUE_UNKNOWN`; bare
`EXIT FOR|DO|WHILE` and `CONTINUE FOR|DO|WHILE` behave exactly as before.

References:

- `./mfb spec language control-flow` (`10_control-flow.md`, the `EXIT FUNC` paragraph).
- `src/docs/spec/diagnostics/01_rule-codes.md`; `.ai/specifications.md`; `.ai/testing-gates.md`.
- `plan-180-A-os-exit.md` (prerequisites), `plan-180-C-named-for-exit.md` (next).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-180-A complete | `grep -c "Commit: —" planning/plan-180-A-os-exit.md` → 0 (or the file is in `planning/completed/`) | NOT MET (2026-10-19) |
| `ExitTarget::Program` is gone | `grep -rn "ExitTarget::Program" src` → no matches | NOT MET (2026-10-19) |

If plan-180-A is not complete, this plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every
> command before you continue, and again before you decide to stop. If you stop, report the
> status of *all* prerequisites.

## 1. Goal

- Exactly three words may follow `EXIT` or `CONTINUE`: `FOR`, `DO`, `WHILE`.
- Every other follower is one diagnostic: `EXIT_UNKNOWN` or `CONTINUE_UNKNOWN`, reported by
  the parser at the token after the keyword.

### Non-goals (explicit constraints)

- Bare `EXIT`/`CONTINUE <kind>` semantics are unchanged: innermost enclosing loop of that
  kind, reaching past inner loops of other kinds (owner, 2026-10-19).
- No IR, NIR, binary or codegen change: `IrOp::ExitLoop`/`ContinueLoop` already carry only
  `LoopKind`, and `EXIT FUNC` lowered to no op. `.ir` goldens and emitted code must not move.
- The AST JSON keeps the `"target"` key with values `for`/`do`/`while` (no key rename, so
  no `.ast` golden moves).
- No migration hint text for `EXIT FUNC`/`EXIT PROGRAM`.

## 2. Current State (after plan-180-A)

- `src/ast/stmt.rs` (`Keyword::Exit` arm): `FOR|DO|WHILE|FUNC` → `ExitTarget`; anything
  else → `MFB_PARSE_UNEXPECTED_TOKEN`. The `Keyword::Continue` arm: `FOR|DO|WHILE` →
  `LoopKind`; else `MFB_PARSE_UNEXPECTED_TOKEN`. Both `return None` after reporting.
- `EXIT FUNC` is accepted by the parser and rejected in `src/ir/shape.rs` (the
  `HirStatement::Exit` arm emits `EXIT_FUNC_FORBIDDEN`; `walk_statements_flow` then emits
  `UNREACHABLE_AFTER_EXIT` for every following statement); `src/ir/lower.rs` lowers it to
  nothing.
- `src/rules/table.rs`: `2-203-0078 EXIT_FUNC_FORBIDDEN`. Highest `2-203` code in use:
  `2-203-0146` (`grep -oE '"2-203-[0-9]{4}"' src/rules/table.rs | sort | tail -1`);
  `2-203-0147` is unused anywhere (`grep -rn "2-203-0147" src planning bugs` → none).
- `ExitTarget` users (`grep -rn "ExitTarget" src --include='*.rs'`): `src/ast/{types,stmt,
  serialize}.rs`, `src/hir/mod.rs`, `src/ir/{lower,shape}.rs`, `src/monomorph/lower.rs`,
  `src/testing/desugar/coverage.rs` (test module).
- `src/fmt.rs` treats the word after `EXIT`/`CONTINUE` lexically (`after_exit`), and its
  comment mentions `EXIT FUNC`; formatting is token-based and needs no logic change.

### Measured populations

| What | Count | Command |
|---|---|---|
| Files mentioning `EXIT FUNC` | 11 | `grep -rc "EXIT FUNC" --include='*.mfb' --include='*.rs' --include='*.md' src tests packages examples \| awk -F: '$2>0'` → src/fmt.rs, src/ir/shape.rs, src/ir/tests.rs, src/ast/tests.rs, src/rules/table.rs, tests/syntax/control-flow/exit-func-invalid, spec: language/10_control-flow, architecture/04_ir, architecture/02_frontend, diagnostics/01_rule-codes, package/12_verifier-rules |
| `EXIT_FUNC_FORBIDDEN` assertions in unit tests | 2 | `grep -n "EXIT_FUNC_FORBIDDEN" src/ir/shape.rs` → the tests at the `assert_eq!(codes, ["EXIT_FUNC_FORBIDDEN", "UNREACHABLE_AFTER_EXIT"])` and `("EXIT_FUNC_FORBIDDEN", 2)` sites |

### Verified properties

- **The parser can emit any registered rule.** `Parser::report(rule, detail, token)`
  (`src/ast/parser.rs`) takes a rule name; `rule_for` looks it up in `RULES`
  (`src/rules/mod.rs`). Rule codes are display labels, not phase-bound (`01_rule-codes.md`,
  "The Rule Record"), so a `2-203` code emitted by the parser is legal.
- **No IR difference.** `EXIT FUNC` lowered to `Vec::new()` and the loop forms already map
  `ExitTarget::{For,Do,While}` → `LoopKind::{For,Do,While}` one-to-one
  (`src/ir/lower.rs:lower_statement`), so collapsing to `LoopKind` is IR-neutral.

## 3. Design Overview

- `Statement::Exit { kind: LoopKind, line }` and `HirStatement::Exit { kind, line }` —
  `ExitTarget` is deleted. `Continue` is already `{ kind, line }`.
- Parser: one helper, `parse_loop_kind_after(keyword)`, used by both arms: `FOR|DO|WHILE`
  → `Some(kind)`; otherwise report `EXIT_UNKNOWN` / `CONTINUE_UNKNOWN` at the peeked token
  (detail: "EXIT must be followed by FOR, DO, or WHILE." / "CONTINUE must be followed by
  FOR, DO, or WHILE.") and `return None` — the same recovery the arms use today.
- Shape: the `EXIT_FUNC_FORBIDDEN` arm and the `ExitTarget::Func` case of `erased_exit`
  go; `erased_exit` keeps only the inline-TRAP-handler case.
- Rules: `2-203-0078` renamed to `EXIT_UNKNOWN` ("EXIT must be followed by FOR, DO, or
  WHILE"); new `2-203-0147 CONTINUE_UNKNOWN` ("CONTINUE must be followed by FOR, DO, or
  WHILE").

Correctness risk: low — front-end only. Gate class: **byte-identical** IR/NIR/emitted code
for every valid program (`artifact-gate.sh all` must show no diff on any valid fixture);
only `build.log` goldens of invalid fixtures that spell `EXIT FUNC` change.

Rejected alternatives:
- *Keep `EXIT FUNC` parsed and rejected in shape* — two places own one rule; the parser
  already knows the follower is not a loop kind.
- *Rename the AST JSON key to `"loop"`* — churns `.ast` goldens for a cosmetic match with
  `continue`.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the work;
> `- [~]` for partial; strike through moot tasks with evidence. **An unticked box means NOT
> DONE.**

### Phase 1 — one vocabulary, two diagnostics

- [ ] `src/rules/table.rs` + `01_rule-codes.md`: `2-203-0078` → `EXIT_UNKNOWN`; add
      `2-203-0147 CONTINUE_UNKNOWN`.
- [ ] `src/ast/stmt.rs`: shared loop-kind helper; both arms emit the new rules.
- [ ] `src/ast/types.rs`: delete `ExitTarget`; `Statement::Exit { kind: LoopKind, line }`.
      Follow every match: `src/ast/serialize.rs` (`exit_target_name` deleted;
      `"target": kind.name()`), `src/hir/mod.rs`, `src/monomorph/lower.rs`,
      `src/ir/lower.rs` (`ExitLoop { kind }` directly), `src/ir/shape.rs`,
      `src/testing/desugar/coverage.rs` tests.
- [ ] `src/fmt.rs`: comment no longer cites `EXIT FUNC` as a form.
- [ ] Tests — `tests/syntax/control-flow/exit-func-invalid` (added `211e73353`, plan-159-D;
      protects: `EXIT FUNC` is rejected). Proof the old diagnostic is wrong: the owner's
      2026-10-19 rename (Decisions). Replace it with `exit-unknown-invalid` (one function
      per spelling: `EXIT FUNC`, `EXIT PROGRAM 1`, `EXIT`, `EXIT foo`, each → `EXIT_UNKNOWN`)
      and add `continue-unknown-invalid` (same four under `CONTINUE` → `CONTINUE_UNKNOWN`);
      goldens from `mfb build -ast -ir`. The two shape unit tests asserting
      `EXIT_FUNC_FORBIDDEN` move to `src/ast/tests.rs` as parser tests asserting
      `EXIT_UNKNOWN` at the same line (their `UNREACHABLE_AFTER_EXIT` expectation goes:
      a statement that failed to parse has no successor in the AST).
      `src/ir/tests.rs` / `src/ast/tests.rs` cases that parse `EXIT FUNC` successfully
      become `EXIT_UNKNOWN` parser assertions.
- [ ] Docs: `10_control-flow.md` (the `EXIT FUNC` paragraph becomes the `EXIT_UNKNOWN` /
      `CONTINUE_UNKNOWN` rule), `04_ir.md` §4 (no `EXIT FUNC` row), `02_frontend.md`,
      `12_verifier-rules.md`, `01_rule-codes.md`.

Acceptance: the eight unknown spellings each give exactly one diagnostic of the right
name; valid programs compile to identical IR.
  Check: `bash scripts/test-accept.sh target/release/mfb /tmp/acc-180b '*-unknown-invalid'`
  → pass; `bash scripts/test-accept.sh target/release/mfb /tmp/acc-180b 'exit-loop-invalid'
  'continue-loop-invalid' 'control-flow-*'` → pass unchanged (est. 3 min).
Commit: —

## Validation Plan

- Tests: the two new syntax fixtures; parser unit tests.
- Coverage check: `grep -rn "EXIT_UNKNOWN\|CONTINUE_UNKNOWN" tests src/ast/tests.rs` hits
  all eight spellings.
- Runtime proof: none needed (no codegen change); the byte-identity gate is the proof.
- Doc sync: the five spec files above; `./target/release/mfb spec language control-flow`
  renders.
- Final gate (once): `bash scripts/test-fast.sh`; `bash scripts/artifact-gate.sh
  target/release/mfb all` → no diff on any valid fixture (15–20 min: the only check that
  proves the `ExitTarget` collapse left IR byte-identical everywhere); full
  `bash scripts/test-accept.sh target/release/mfb /tmp/acc-180b-full` (a golden is
  replaced, AGENTS.md requires both).

## Decisions

- Only `FOR`, `DO`, `WHILE` may follow `EXIT`/`CONTINUE` — owner, 2026-10-19.
- `EXIT_FUNC_FORBIDDEN` is renamed `EXIT_UNKNOWN` and covers every unknown follower —
  owner, 2026-10-19.
- `CONTINUE` mirrors `EXIT`: `CONTINUE_UNKNOWN` — owner, 2026-10-19 ("mimic CONTINUE same
  as EXIT").
- No special errors or hints for removed spellings — owner, 2026-10-19.
- The parser, not shape, reports both rules — author (one owner per rule).

## Corrections

## Summary

A front-end-only change; the risk is a missed `ExitTarget` match, which the compiler finds.
Codegen, IR and the package format are untouched.
