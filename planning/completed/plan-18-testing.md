# Plan: Built-in Test Framework (`TESTING` blocks + `mfb test` + coverage) — overview

Last updated: 2026-07-02
Overall Effort: x-large

**Split plan** (by effort into three landable sub-plans; see "Execution phases"). This file is the
overview holding the shared design, grammar, and global acceptance criteria; the phases and concrete
tasks live in the lettered sub-plans.

This builds a **first-class test framework for MFB programs** — Jest-style `TESTING` / `TGROUP` /
`TCASE` blocks written in `.mfb` source, a new `mfb test` subcommand that compiles and runs them
with a synthesized entry point, a streaming pass/fail tree on the terminal, a non-zero exit on any
failure, and an optional `--coverage` mode that emits `coverage.html` with a file tree and
color-coded, annotated source.

The single behavioral outcome: a program with `TESTING` blocks builds normally under `mfb build`
(the blocks are ignored and contribute nothing to the binary), and under `mfb test` produces a live
tree of group/case results plus a summary line, exiting 0 iff every case passed.

It complements:

- `mfb spec language` (new top-level `TESTING` block form; the canonical spec lives under
  `src/docs/spec/language/**`).
- `mfb spec diagnostics` (new resolver diagnostics for misplaced `TGROUP`/`TCASE`/`expect*`, and the
  internal test-abort trap code; `error-codes` table is the build input for `errorCode::`).
- `mfb spec package` (the four `expect*` assertion builtins).

---

## 1. Goal

- New source surface: `TESTING … END TESTING` top-level blocks containing `TGROUP <desc> … END
  TGROUP`, each containing `TCASE <desc> … END TCASE` whose body is ordinary MFB statements plus the
  assertion builtins.
- Four compiler-lowered assertion builtins, call syntax, valid only inside a `TCASE`:
  - `expectEQ(actual, expected)` — pass iff `actual == expected`.
  - `expectNQ(actual, expected)` — pass iff `actual != expected`.
  - `expectTrap(expr)` — pass iff evaluating `expr` traps.
  - `expectTrap(expr, expected)` — pass iff evaluating `expr` traps and the trap `error code == expected`.
  - `expectNTrap(expr)` — pass iff evaluating `expr` does **not** trap.
- `mfb build`: `TESTING` blocks parse and validate but are **dropped before codegen** — zero bytes in
  the normal binary, no behavioral or golden change.
- `mfb test`: compiles including the `TESTING` blocks, discards the normal top-level program body,
  synthesizes a driver entry that runs every `TCASE` in declaration order under per-case trap
  isolation, and streams:

  ```
  * <group_description>
    * [P] <case_description>
    * [F] <case_description>
      X <failure detail>
  * <group_description>
    * [P] <case_description>

  Tests: NN  Pass: NN  Fail: NN
  ```

  Exit status is non-zero iff any case failed (CI-usable).
- `mfb test --coverage`: additionally instruments the user's `.mfb` statements, dumps hit counts at
  exit, and writes `coverage.html` — a tree of the program's source files with per-file line-coverage
  stats; clicking a file shows its source, color-coded covered/uncovered, with failed-case lines
  annotated.

### Non-goals (explicit constraints)

- **No change to value / copy / move / freeze semantics.** `TCASE` bodies are ordinary MFB code and
  obey the same ownership and scope-drop rules as any function body.
- **No change to layout/ABI or normal-build output.** `TESTING` is dropped at monomorph for
  `mfb build`; native-code goldens for non-test programs stay byte-identical. Coverage is a separate
  build mode (`mfb test --coverage`) and never affects a normal build.
- **`expectEQ`/`expectNQ` reuse the language's `==`/`!=`** — no new structural-equality semantics are
  invented. Operands must be types the `==` operator already accepts and that have `toString` (needed
  for the failure message).
- **No new reserved words beyond `TESTING`.** `TGROUP`/`TCASE` are contextual — recognized only
  inside a `TESTING` block (see Open Decisions) — so existing programs using those identifiers are
  unaffected. `expect*` are builtin function names, not keywords.
- **Sequential, single-process test execution** in V1 — shared globals, no per-case fork/isolation
  beyond trap catching. No parallelism.
- **Line coverage only** in V1. Branch coverage is out of scope.

## 2. Current State

- **Entry model**: MFB has no `main`/`PROGRAM` entry block — the top-level statement sequence *is* the
  program body (`Keyword::Program` at `src/lexer.rs:101` is only the `EXIT PROGRAM` target,
  `src/ast/stmt.rs:118`). The synthesized test entry replaces that top-level body.
- **Subcommands** dispatch in `src/main.rs:45` (`build`, `pkg`, `repo`, `audit`, `man`, `spec`, `doc`,
  `fmt`). There is no `run` or `test` yet; `mfb build` produces a binary you run separately. `mfb test`
  is a new arm.
- **Top-level AST** is `Item` (`src/ast/types.rs:56`): `Binding | Function | Type | Resource |
  FuncAlias | Link | Doc`. A new `Item::Testing(TestingBlock)` slots in beside `Doc` (a similar
  free-standing, build-time-only-in-some-modes block).
- **Statements** all carry `line: usize` (`src/ast/types.rs:449`) — the source truth coverage needs.
  That line is **dropped** below the AST today (only `IrOp::For` keeps a `loc`), so coverage
  instrumentation must hook at AST→IR lowering where the line still exists — see plan-18-C.
- **Keyword lexing**: `keyword()` / `lookup_keyword()` (`src/lexer.rs:597-601`), `Keyword` enum
  (`src/lexer.rs:63`). `TESTING` is added there; `TGROUP`/`TCASE` are matched contextually by the
  block parser (Open Decisions §D1).
- **Builtins** are declared per-package in `src/builtins/*.rs` with the resolution surface in
  `general.rs` (`is_general_call`, `resolve_call`, `arity`, `call_param_names`,
  `call_return_type_name`, `reserved_builtin_name` — `src/builtins/general.rs:40-360`). The four
  `expect*` builtins get a new `src/builtins/testing.rs` wired into this surface.
- **Source-location injection precedent**: `error()` lowering builds an `ErrorLoc` constructor from
  the call-site `(file, line, column)` at `src/ir/lower.rs:2926` (`error_loc_value`), using
  `IrFunction::file` (`src/ir/mod.rs:93`). The `expect*` builtins mirror this to stamp each assertion
  with its call site.
- **Trap infrastructure**: user `TRAP`/`Propagate`/`Recover` handling plus the out-of-line ErrorLoc
  work (plan-16 trap outlining) provides the trap-region machinery that `expectTrap`/`expectNTrap`
  and the per-case isolation reuse. The `_mfb_shutdown` teardown hook
  (memory: shutdown-and-signal-handlers) is where coverage counts are flushed at exit.
- **Buffered stdout** (plan-14): the reporter must `io::flush` per line so the tree streams as cases
  run rather than dumping at the end.

## 3. Design Overview

Three layers, landed in dependency order:

1. **Surface + exclusion (plan-18-A)** — lexer/parser/AST for `TESTING`/`TGROUP`/`TCASE`, resolver
   validation (nesting rules, `expect*`-only-in-`TCASE`, string descriptions), and the monomorph seam
   that **drops** `TESTING` for `mfb build` and **retains** it for `mfb test`. Ends with: a program
   with `TESTING` blocks builds byte-identically under `mfb build`, and `mfb test` (stub driver)
   proves the blocks reach codegen. Lowest-risk, separately valuable, no new runtime.

2. **Runner + assertions (plan-18-B)** — the four `expect*` builtins (EQ/NQ desugar to `==`/`!=` +
   `toString` for the message; Trap/NTrap use trap-guarded evaluation), `TCASE`→generated-SUB desugar
   plus a compile-time registration table, the synthesized `mfb test` driver entry (iterate table,
   per-case trap isolation, streaming reporter, tally, exit code), and the recorded-failure slot that
   lets one trap catch point distinguish assertion failures from genuine runtime errors. This is where
   the correctness risk concentrates: trap-guarded evaluation and the driver's unwind handling.

3. **Coverage (plan-18-C)** — statement instrumentation injected at AST→IR lowering (a counter
   increment per user statement, keyed to a compile-time `slot → (file, line)` map), a runtime counter
   array flushed at `_mfb_shutdown`, and an HTML generator producing the file tree + annotated,
   color-coded source with failed-case line annotations. Gated entirely behind `mfb test --coverage`
   so normal-build goldens are untouched.

The key reuse insight that keeps B tractable: `expectEQ(a,b)` is *not* new comparison codegen — it
lowers to the existing `==` plus a call-site-stamped "record failure and raise the test-abort trap"
primitive; the driver catches that trap exactly as it catches a runtime error, and reads the recorded
slot to tell the two apart. Only `expectTrap`/`expectNTrap` need genuinely new (trap-guarded) codegen,
and that builds on the existing trap-region machinery.

## 4. Grammar (shared reference)

```
Item        := … | TestingBlock
TestingBlock:= 'TESTING' NEWLINE Group* 'END' 'TESTING'
Group       := 'TGROUP' StringLit NEWLINE Case* 'END' 'TGROUP'
Case        := 'TCASE' StringLit NEWLINE Statement* 'END' 'TCASE'
```

- A `TESTING` block appears anywhere a top-level `Item` may appear (bottom of a file, right after the
  function under test, or in a dedicated `*_test.mfb`-style file — no placement rule).
- Multiple `TESTING` blocks per compilation are allowed and concatenated in declaration order.
- `TGROUP`/`TCASE` descriptions are **string literals** (used verbatim in the report tree).
- A `TCASE` body is a `Statement*` block — the full statement grammar, plus the `expect*` builtins.
- Assertion builtins (call expressions inside a `TCASE`):
  `expectEQ(actual, expected)`, `expectNQ(actual, expected)`, `expectTrap(expr)`, `expectTrap(expr, expected)`,
  `expectNTrap(expr)`. `expectEQ`/`expectNQ` require operand types accepted by `==` and having
  `toString`; `expectTrap`/`expectNTrap` take one arbitrary expression whose evaluation is
  trap-guarded, overloaded `expectTrap` takes one arbitrary expression and the expected error code.

Grammar leaves room (unused in V1, no syntax reserved that would block them) for later `SETUP`/
`TEARDOWN` blocks inside `TGROUP`, `SKIP`/`ONLY` modifiers on `TCASE`, and a `--filter` CLI flag.

## 5. Report format (shared reference)

Streamed as cases run (flush per line):

```
* <group_description>
  * [P] <case_description>
  * [F] <case_description>
    X <failure detail>          e.g.  expected 4, got 5   (file.mfb:12)
* <group_description>
  * [P] <case_description>
  * [P] <case_description>

Tests: <total>  Pass: <passed>  Fail: <failed>
```

- A case is `[P]` iff it ran to completion with no failed assertion and no runtime trap.
- On the **first** failed `expect*` the case aborts (raises the internal test-abort trap); remaining
  statements in that `TCASE` do not run; sibling cases and groups continue.
- Failure detail distinguishes kinds: assertion → `expected <toString(expected)>, got
  <toString(actual)>`; `expectTrap` with no trap → `expected a trap, none occurred`; `expectNTrap`
  that trapped → `unexpected trap: <error message>`; genuine runtime error → the trap's message +
  `ErrorLoc`.
- Exit code: `0` iff `Fail: 0`, else non-zero.

## Layout / ABI Impact

- **Normal builds (`mfb build`): none.** `TESTING` is dropped at monomorph before codegen; the binary
  is byte-identical to one without the blocks. This is a hard acceptance gate (plan-18-A).
- **`mfb test` build**: adds a compile-time registration table (static data) and a synthesized driver
  entry, replacing the normal top-level program body. No change to any existing type's layout,
  copy/transfer semantics, or `.mfp` format for non-test symbols.
- **`mfb test --coverage` build**: adds a BSS counter array and per-statement increment ops, and a
  sidecar `slot → (file,line)` map produced by the compiler. Present only in this mode.
- **New internal error code** for the test-abort trap (registered in `mfb spec diagnostics`
  `error-codes`). It is internal (never surfaced to `errorCode::` user code) but must occupy a stable
  slot so the driver can recognize it.

## Execution phases

Land the surface + exclusion first (so `mfb build` safety is proven before any runtime exists), then
the runner, then coverage. Commit per phase on the current branch (no branches — repo policy).

| Doc | Effort | Delivers | Depends on |
|---|---|---|---|
| [plan-18-A](plan-18-A-surface-exclusion.md) — surface + build exclusion | large | lexer/parser/AST + resolver validation + monomorph drop-vs-retain; `mfb build` byte-identical, `mfb test` stub reaches codegen | — |
| [plan-18-B](plan-18-B-runner-assertions.md) — assertions + runner | large | four `expect*` builtins, `TCASE`→SUB + registration table, synthesized driver, streaming reporter, exit code | A |
| [plan-18-C](plan-18-C-coverage.md) — coverage instrumentation + HTML | large | lower-time statement counters + slot map, runtime dump at shutdown, `coverage.html` (file tree + annotated source) | A, B |

## Validation Plan (global)

- **Function tests**: `tests/func_testing_expectEQ_valid/**` + `_invalid/**`, and the same for
  `expectNQ`, `expectTrap`, `expectNTrap`, covering every overload/operand-type path the resolver
  accepts, plus the misuse diagnostics (`expect*` outside a `TCASE`, `TCASE` outside a `TGROUP`,
  non-string description, etc.).
- **Build-exclusion proof**: a fixture with `TESTING` blocks whose `mfb build` native output is
  byte-identical to the same program with the blocks deleted (the plan-18-A gate).
- **Runtime proof**: a fixture program run under `mfb test` whose streamed tree + summary + exit code
  are asserted (mix of pass, assertion-fail, `expectTrap`, and a genuine runtime error → errored
  case). Not just golden text — assert the exit status.
- **Coverage proof**: `mfb test --coverage` on a fixture where some lines are deliberately unexercised;
  assert the generated `coverage.html` marks exactly those lines uncovered and annotates the failed
  case's line.
- **Doc sync**: `src/docs/spec/language/**` (new block form), `src/docs/spec/diagnostics/**` (new
  diagnostics + internal error code), package/man pages for the four `expect*` builtins
  (`.ai/man_template.md`).
- **Acceptance**: `scripts/test-accept.sh target/debug/mfb target/accept-actual` green.

## Open Decisions

- **D1 — `TGROUP`/`TCASE` as contextual vs. hard keywords.** *Recommended:* add only `TESTING` to the
  `Keyword` enum; the `TESTING`-block parser recognizes `TGROUP`/`TCASE` by lexeme so they remain
  usable as identifiers everywhere else (honors "only valid in `TESTING`", zero collision risk).
  Alternative: make all three hard keywords (simpler parser, but `TGROUP`/`TCASE` become globally
  reserved). (§2, §4)
- **D2 — `expectEQ` operand scope in V1.** *Recommended:* accept exactly the operand types the `==`
  operator accepts today (scalars + String, plus whatever `==` already supports), reusing language
  equality; reject others with a resolver diagnostic. Alternative: define structural equality for
  collections/records now (larger, deferrable). (§1 Non-goals)
- **D3 — test-abort mechanism.** *Recommended:* reuse the existing trap propagation — a failed
  `expect*` records into the failure slot then raises an internal trap that the driver's per-case
  `TRAP` handler catches (same catch point as a runtime error; the slot disambiguates). Alternative:
  a bespoke non-local return path (more codegen, no benefit). (§3, plan-18-B)
- **D4 — coverage side-map delivery.** *Recommended:* compiler writes the `slot → (file,line,relpath)`
  map as a sidecar during the `--coverage` build; the runtime writes raw counts to a known file at
  `_mfb_shutdown`; `mfb test` post-processes both into `coverage.html`. Alternative: embed the map in
  the binary (bloats the test binary; no benefit since `mfb test` owns the whole flow). (§ plan-18-C)

## Summary

The engineering risk is concentrated in plan-18-B's trap-guarded evaluation (`expectTrap`/
`expectNTrap`) and the driver's single-catch-point unwind that separates assertion failures from
runtime errors — everything else reuses existing machinery (`==`/`toString` for EQ/NQ, `ErrorLoc`
call-site stamping, the `TRAP` region, `_mfb_shutdown`). plan-18-A is deliberately low-risk and gated
on byte-identical normal builds. plan-18-C is additive and mode-gated, hooking coverage at the one
layer (AST→IR lowering) where per-statement source lines still exist. Nothing in any phase touches
value/copy/transfer semantics or the layout of existing types.
