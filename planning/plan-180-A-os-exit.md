# plan-180-A: `os::exit` replaces `EXIT PROGRAM`

Last updated: 2026-10-19
Overall Effort: x-large (1d–3d) — plan-180 A + B + C
Effort: large (3h–1d)
Depends on: nothing

`EXIT PROGRAM <code>` leaves the language. Programs end the process with an ordinary
builtin call, `os::exit(code AS Integer) AS Nothing`, usable anywhere any other builtin
call is (a statement, `RETURN os::exit(1)`, under an inline `TRAP`, inside a `TRAP`
handler). Its runtime behaviour is exactly today's `EXIT PROGRAM`: not catchable by any
`TRAP`, lexical cleanup of every live scope up to the entry point, a worker's exit ends
the process at the worker's entry function. `PROGRAM` stops being a keyword.

Observable outcome: `IMPORT os` + `os::exit(3)` exits the process with status 3 from any
call depth and through every `TRAP` form; `EXIT PROGRAM 3` no longer compiles.

References:

- `./mfb spec language control-flow` (`src/docs/spec/language/10_control-flow.md`) — the
  `EXIT PROGRAM` paragraph this plan rewrites.
- `./mfb spec language error-model` (`08_error-model.md`) — the completion checker's
  accepted proof forms (lists `EXIT PROGRAM`).
- `.ai/compiler.md`, `.ai/codegen-invariants.md`, `.ai/resources-packages.md` (registry
  helper chunks / `Body::Rewrite`), `.ai/testing-gates.md`, `.ai/man-content.md`,
  `.ai/specifications.md`.
- Sibling sub-plans: `plan-180-B-exit-continue-unknown.md`, `plan-180-C-named-for-exit.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| No `os::exit` member exists yet | `grep -rn 'name: "exit"' src/codegen/builtins/os/` → no matches | MET (2026-10-19) |
| The keyword `exit` parses in `pkg::member` position | `/tmp` probe: `IMPORT os` + `os::exit(3)` builds to `SYMBOL_UNKNOWN_IDENTIFIER` (a resolve error, not a parse error) | MET (2026-10-19) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every
> command before you continue, and again before you decide to stop. If you stop, report the
> status of *all* prerequisites.

## 1. Goal

- `os::exit(code)` exists, is documented in `mfb man os`, and ends the process with `code`
  under every condition `EXIT PROGRAM` did (any depth, every `TRAP` form, worker threads,
  scope cleanup).
- `EXIT PROGRAM` and the `PROGRAM` keyword are gone; `program` is an ordinary identifier.
- A function whose every path ends in a plain `os::exit(...)` statement needs no `RETURN`
  (same completion proof `EXIT PROGRAM` had).

### Non-goals (explicit constraints)

- The runtime exit mechanism does not change: `IrOp::ExitProgram`/`NirOp::ExitProgram`, the
  result tag `RESULT_PROGRAM_EXIT_TAG`, `emit_program_exit_value`, the inline-TRAP
  passthrough (`emit_inline_trap_program_exit_passthrough`) and the worker-entry handling
  (`runtime_helpers_thread.rs`) are reused unchanged.
- No Binary Representation format change (the IR ops are unchanged; `BINARY_REPR_VERSION`
  stays 7 in this letter).
- No migration hint diagnostic for `EXIT PROGRAM` (owner, 2026-10-19).
- `EXIT FUNC` handling is plan-180-B's; this letter leaves `ExitTarget::Func` as it is.

## 2. Current State

- Parser: `src/ast/stmt.rs` (the `Keyword::Exit` arm) parses `EXIT FOR|DO|WHILE|FUNC|PROGRAM`;
  only `PROGRAM` takes an operand (`code: Option<Expression>`). AST
  `Statement::Exit { target: ExitTarget, code, line }` (`src/ast/types.rs:ExitTarget`
  has `For, Do, While, Func, Program`). HIR mirrors it (`src/hir/mod.rs:HirStatement::Exit`).
- Lexer: `Keyword::Program` exists only for `EXIT PROGRAM` (`grep -rn "Keyword::Program" src`
  → `src/lexer.rs` ×3 and `src/ast/stmt.rs` ×1). `PROGRAM` is in the keyword set and the
  `begins_statement_only` list (`02_lexical-structure.md` lines 9 and 91).
- IR lowering: `src/ir/lower.rs:lower_statement` maps `ExitTarget::Program` →
  `IrOp::ExitProgram { code }`.
- Checks: `src/ir/verify/ops.rs` (the `IrOp::ExitProgram` arm) emits
  `TYPE_EXIT_PROGRAM_REQUIRES_INTEGER` and `EXIT_PROGRAM_CODE_OUT_OF_RANGE` (constant via
  `src/ir/verify/mod.rs:integer_constant_value`). `src/ir/shape.rs:walk_statements_flow`
  emits `UNREACHABLE_AFTER_EXIT` after `EXIT FUNC`/`EXIT PROGRAM`.
- Completion: `src/ir/verify/resources.rs:block_always_returns` accepts
  `Return | Fail | ExitProgram`.
- Runtime: a function that executes `ExitProgram` returns tag 2 through the result
  protocol; every caller frame propagates it past any `TRAP`. The rt fixture
  `tests/rt-error/control-flow/exit-program-valid-rt` already proves a *user* helper
  `FUNC leave(code AS Integer) AS Nothing : EXIT PROGRAM code` called from `main` exits
  with the code and skips `main`'s function-level `TRAP` — `os::exit` reuses exactly that
  shape.
- Precedents mirrored:
  - Statement-position builtin interception in IR lowering: the `testing` assertion
    desugar at the top of `HirStatement::Expression` in `src/ir/lower.rs`.
  - Builtin-specific call-site check in shape: `os.getConfigOr` literal check
    (`src/ir/shape.rs`, `check_config_call_literals`, gated `!self.current_file_internal`).
  - A builtin implemented as injected MFBASIC only when used: `strings` scalar seam
    (`src/codegen/builtins/strings/helper_scalar_seam.rs`: `RegistryHelper` with
    `HelperGate::WhenUsed(&[...])` + `Body::Rewrite("__strings_isDigit")` members). A plain
    `Body::mfb` would render into every `IMPORT os` program (that file's header comment),
    moving every os-importing golden; `WhenUsed` keeps non-users byte-identical.
  - IR spelling of a `__pkg_name` helper: `#pkg_name` (`grep -rhoE '"#csv_parse"' tests
    --include='*.ir'` → present).

### Measured populations

| What | Count | Command |
|---|---|---|
| `.mfb` files using `EXIT PROGRAM` | 8 | `grep -rl "EXIT PROGRAM" --include='*.mfb' . \| grep -v '^./target'` → tools/network-server-churn/conn-record-probe, tools/recursive-value-bench/programs/regex_chain, tests/rt-error/control-flow/exit-program-valid-rt, tests/syntax/control-flow/exit-program-invalid, examples/network-client, examples/audio, examples/network-server, packages/cli/src/parse.mfb |
| `EXIT PROGRAM` occurrences in `.mfb` | 28 | `grep -r "EXIT PROGRAM" --include='*.mfb' . \| grep -v '^./target' \| wc -l` |
| `.rs` files mentioning `EXIT PROGRAM` (code, tests, comments) | 26 | `grep -rl "EXIT PROGRAM" --include='*.rs' src tests \| wc -l` |
| Doc files mentioning `EXIT PROGRAM` | 11 | `grep -rc "EXIT PROGRAM" src/docs \| awk -F: '$2>0'` → man/debug/08_report.md, man/flow/package.md, spec/tooling/09_debug-report.md, spec/language/{14_memory-semantics,08_error-model,15_resource-management,10_control-flow}.md, spec/architecture/04_ir.md, spec/diagnostics/01_rule-codes.md, spec/threading/06_thread-runtime-helpers.md, spec/package/08_ir-section.md |
| `.ast` goldens containing an exit statement (gain no `"code"` key) | 6 files / 11 statements | `grep -rl '"kind": "exit"' tests --include='*.ast' \| wc -l` → 6; `grep -r '"kind": "exit"' tests --include='*.ast' \| wc -l` → 11 |
| Runtime test binaries whose programs use `EXIT PROGRAM` | 5 | `grep -rl "EXIT PROGRAM" tests/runtime` → rt_resource_review_806, rt_exit_program_through_inline_trap, rt_thread_exit_program_in_worker, rt_native_io_runtime, rt_debug_report |

### Verified properties

- **Keyword member names parse.** `os::exit(3)` and `os::for(3)` in a `/tmp` project both
  fail only with `SYMBOL_UNKNOWN_IDENTIFIER` (release `mfb`, 2026-10-19), so `exit` is
  accepted after `::`.
- **Builtins are not first-class values.** `LET f = os::sleep` →
  `TYPE_UNKNOWN_VALUE` (probe, 2026-10-19). "Anywhere a function can appear" therefore
  means call positions only; no function-value path exists to support.
- **A `TRAP` on an infallible call is legal.** `helper() TRAP(e) ... END TRAP` on a
  `FUNC helper AS Nothing` with no `FAIL` builds (probe). So `os::exit(1) TRAP(e)` compiles
  and its handler never runs.
- **`RETURN <Nothing-call>` is legal.** `RETURN os::sleep(1)` in an `AS Nothing` function
  builds (probe).
- **Code after `RETURN` is accepted.** `RETURN 0` followed by `LET x = 1` builds with no
  diagnostic (probe); `os::exit` matches this (owner decision).
- **Rewrite happens in IR lowering.** `registry::rewrite_target` is called from
  `src/ir/lower.rs` (call lowering), so at `HirStatement::Expression` the callee is still the
  source spelling and `canonical_import_name(callee, context)` yields `os.exit`.
- **Runtime value outside 0..255.** `EXIT PROGRAM x` with `x = 300` exits 44 (probe) — the
  host truncation; `os::exit` keeps it (no new runtime check).

## 3. Design Overview

Three pieces:

1. **The member.** `os::exit` is a registry function (`src/codegen/builtins/os/func_exit.rs`):
   one implementation, param `code AS Integer`, returns `Nothing`, errors `[]`,
   `body: Body::Rewrite("__os_exit")`. A gated helper chunk registered by the same file —
   `RegistryHelper { name: "os", gate: HelperGate::WhenUsed(&["exit"]), body: Some(EXIT_HELPER), .. }`
   — declares:
   ```
   IMPORT os
   FUNC __os_exit(code AS Integer) AS Nothing
     os::exit(code)
   END FUNC
   ```
2. **Statement-position lowering.** In `src/ir/lower.rs`, `HirStatement::Expression` whose
   expression is a plain `HirExpression::Call` with canonical callee `os.exit` lowers to
   `IrOp::ExitProgram { code: <lowered argument> }` — no call. This is the form
   `EXIT PROGRAM` had, so every downstream layer (completion proof, TRAP bypass, cleanup,
   codegen) already handles it. The helper body above is itself such a statement, so
   `__os_exit` compiles to exactly the `leave` helper the rt fixture proves today.
   Every other position (`RETURN os::exit(1)`, a `Trapped` expression, a call nested in an
   expression) goes through ordinary call lowering, is rewritten to `#os_exit`, and exits
   via the result-tag protocol — again the proven `leave` shape.
3. **Checks.** `TYPE_EXIT_PROGRAM_REQUIRES_INTEGER` is retired: argument typing is the
   ordinary builtin signature check. The constant range check moves from the IR op to the
   call site in `src/ir/shape.rs` (beside the `os.getConfigOr` check, user files only) and is
   renamed `OS_EXIT_CODE_OUT_OF_RANGE` (same code `2-203-0080`). The shape walker's
   `statement_flow` reports a plain `os.exit` call statement as `Flow::AlwaysReturns`, so a
   handler or block ending in it is treated as diverging, and — because it is not an
   "erased exit" — statements after it get no `UNREACHABLE_AFTER_EXIT` (like `RETURN`).

Correctness risk: concentrated in piece 2's matching rule (it must match only a bare
`Call` statement, never a `Trapped` one, or a TRAP'd exit would lose its handler semantics
— harmless since the handler can't run, but the IR would no longer show the trap). The
runtime path is reused, not changed. Byte-identity: **not** this plan's gate for fixtures
that use `EXIT PROGRAM` (their source changes); it **is** expected for every fixture that
neither uses `EXIT PROGRAM` nor imports `os::exit` — the `WhenUsed` gate means no other
program's IR changes. Expected diffs: the 6 `.ast` goldens listed above (the `"code": null`
key disappears) and the two fixtures rewritten in Phase 3.

Rejected alternatives:
- *Plain `Body::mfb` body* — injected into every `IMPORT os` program; churns every
  os-importing golden for no behaviour.
- *Native `Body::abi_function` that calls the host `exit`* — skips scope cleanup and the
  worker rule; would re-implement what `ExitProgram` already does right.
- *Lower every `os::exit` call (all positions) to `ExitProgram`* — positions are open-ended
  (`RETURN`, nested expressions, trapped forms); the helper covers them with the existing
  protocol and no per-position code.
- *An internal-only `os.exitProgram` intrinsic for the helper body* — unnecessary: the
  helper's own `os::exit(code)` statement hits the statement-position lowering.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as the
> work; `- [~]` for partial with what remains; moot tasks are struck through with evidence,
> never deleted. Fill each `Commit:` line when the phase lands. **An unticked box means NOT
> DONE.**

### Phase 1 — add `os::exit` beside `EXIT PROGRAM`

Purely additive: the new member works; nothing is removed yet, so every existing golden is
unchanged.

- [ ] `src/codegen/builtins/os/func_exit.rs` (new): descriptor (`intro`, `desc`, `example`
      per `.ai/man-content.md` — say it never returns, `TRAP` cannot catch it, cleanup runs,
      a worker's call ends the whole process, a value outside 0..255 is truncated by the
      host, a constant outside 0..255 is a compile error), `Body::Rewrite("__os_exit")`, and
      the `WhenUsed(&["exit"])` helper chunk above. Register it in
      `src/codegen/builtins/os/mod.rs`.
- [ ] `src/ir/lower.rs`, `HirStatement::Expression`: a bare `HirExpression::Call` whose
      `canonical_import_name` is `os.exit` lowers to `IrOp::ExitProgram`, placed with the
      `testing` desugar.
- [ ] `src/ir/shape.rs`: `statement_flow` returns `Flow::AlwaysReturns` for a
      `HirStatement::Expression` holding a bare `os.exit` call (canonical callee); the
      call-site walk (beside `os.getConfigOr`, user files only) emits the existing rule
      `EXIT_PROGRAM_CODE_OUT_OF_RANGE` (`2-203-0080`; renamed in Phase 2) when the `code`
      argument (positional or `code :=`) is an integer literal or a negated integer literal
      outside `0..=255`.
- [ ] Tests: unit test in `src/ir/tests.rs` that `os::exit(3)` as a statement lowers to
      `IrOp::ExitProgram` and `RETURN os::exit(3)` lowers to a call of `#os_exit`; a new
      rt fixture `tests/rt-error/control-flow/os-exit-valid-rt/` (the
      `exit-program-valid-rt` program with `os::exit(code)` in `leave`, plus a `main` path
      that calls `RETURN os::exit(...)` in an `AS Nothing` helper) with goldens generated by
      `mfb build -ast -ir` + run; a `FUNC f AS Integer` whose only statement is
      `os::exit(1)` builds (completion proof).

Acceptance: `os::exit` ends the process with its code at depth and through a
function-level `TRAP`, and an `AS Integer` function ending in `os::exit` needs no `RETURN`.
  Check: `bash scripts/test-accept.sh target/release/mfb /tmp/acc-180a os-exit-valid-rt` →
  pass; `cargo test --bin mfb ir::tests::os_exit` → pass (est. 3 min with the release build).
Commit: —

### Phase 2 — remove `EXIT PROGRAM` and the `PROGRAM` keyword

- [ ] `src/lexer.rs`: delete `Keyword::Program` (all three sites) so `program` lexes as an
      identifier; `src/docs/spec/language/02_lexical-structure.md`: drop `PROGRAM` from the
      keyword set and the `begins_statement_only` list.
- [ ] `src/ast/stmt.rs`: drop the `PROGRAM` branch and the operand parse; the error text
      becomes "EXIT must be followed by FOR, DO, WHILE, or FUNC." (plan-180-B replaces it).
- [ ] `src/ast/types.rs`: delete `ExitTarget::Program` and `Statement::Exit::code`; follow
      the compiler through every match (`src/ast/{serialize,scope_privates}.rs`,
      `src/hir/{mod,resource_params}.rs`, `src/monomorph/lower.rs`, `src/ir/{lower,shape,
      fallible}.rs`, `src/resolver/resolution.rs`, `src/codegen/registry/mod.rs` (4 walkers),
      `src/audit/collect/source.rs`, `src/testing/desugar/{placement,coverage}.rs`).
      `serialize.rs` emits `{ "kind": "exit", "target": …, "line": … }` (no `"code"`).
- [ ] `src/ir/verify/ops.rs`: the `IrOp::ExitProgram` arm keeps only the value walk; delete
      both diagnostics there. Delete `src/ir/verify/mod.rs:integer_constant_value` if it has
      no other caller (`grep -rn integer_constant_value src`).
- [ ] `src/ir/shape.rs`: drop `ExitTarget::Program` from `erased_exit`; the call-site check
      now emits `OS_EXIT_CODE_OUT_OF_RANGE`.
- [ ] `src/rules/table.rs` + `01_rule-codes.md`: rename `2-203-0080` to
      `OS_EXIT_CODE_OUT_OF_RANGE` ("os::exit constant exit code is outside the host range");
      `2-203-0079 TYPE_EXIT_PROGRAM_REQUIRES_INTEGER` becomes "reserved; not emitted; code is
      not reused" (the `EXIT_SUB_IN_FUNC` precedent).
- [ ] Migrate the 28 `.mfb` occurrences in the 6 non-test files listed in §2 to
      `os::exit(...)`, adding `IMPORT os` where missing.
- [ ] Comments naming `EXIT PROGRAM` as a construct in the 26 `.rs` files: reword to
      `os::exit` / "program exit" (`grep -rn "EXIT PROGRAM" src tests --include='*.rs'` → 0
      afterward, except test text quoting old bug titles).

Acceptance: `EXIT PROGRAM 1` is rejected; `program` is usable as a variable name.
  Check: `/tmp` project with `LET program AS Integer = 1` builds and one with
  `EXIT PROGRAM 1` fails with `MFB_PARSE_UNEXPECTED_TOKEN` (est. 1 min);
  `cargo check --workspace --all-targets` clean.
Commit: —

### Phase 3 — tests and docs follow the removal

Each test below pins the removed construct. For every one, per AGENTS.md: (1) origin is
cited; (2) the behaviour it protects is stated; (3) dependents were grepped; (4) proof it
is now wrong is the owner's 2026-10-19 decision removing `EXIT PROGRAM` (Decisions). Each
keeps its runtime assertion; only the spelling of the exit changes.

- [ ] `tests/runtime/rt_exit_program_through_inline_trap.rs` (bug-841 RED, `5488be15a`;
      protects: a program exit passes every inline-TRAP form with status 7): rename to
      `rt_os_exit_through_inline_trap.rs` (Cargo.toml `[[test]]` entry too), replace each
      `EXIT PROGRAM 7` with `os::exit(7)` + `IMPORT os`, and add one case where the trapped
      call *is* `os::exit(7) TRAP(e) … END TRAP`. Assertions unchanged (exit 7, no output).
- [ ] `tests/runtime/rt_thread_exit_program_in_worker.rs` (bug-898 RED, `b49b963b7`;
      protects: a worker's program exit ends the process with its code): rename to
      `rt_thread_os_exit_in_worker.rs` (+ Cargo.toml), spell the exit `os::exit(code)`.
- [ ] `tests/runtime/{rt_resource_review_806,rt_native_io_runtime,rt_debug_report}.rs`:
      spell their programs' exits `os::exit(...)`; assertions unchanged.
- [ ] `tests/rt-error/control-flow/exit-program-valid-rt` (added in the `b5cb72b51`
      reorganization; protects: a callee's exit skips the caller's function-level `TRAP`):
      deleted — `os-exit-valid-rt` (Phase 1) carries the same program and assertion.
- [ ] `tests/syntax/control-flow/exit-program-invalid` (`b5cb72b51`; protects the two
      retired/renamed diagnostics): replace with `tests/syntax/control-flow/os-exit-invalid`
      — `os::exit("bad")` (ordinary argument type error) and `os::exit(300)`
      (`OS_EXIT_CODE_OUT_OF_RANGE`); golden `build.log` generated by
      `mfb build -ast -ir`.
- [ ] Unit tests naming `ExitTarget::Program` / `EXIT PROGRAM` in `src/ir/tests.rs`,
      `src/ast/tests.rs`, `src/codegen/builtins/tests/{nir_validation,optimizer_globals,
      corpus}.rs`: rewrite each program to `os::exit`; assertions unchanged.
- [ ] Docs (the 11 files in §2): `10_control-flow.md` drops the `EXIT PROGRAM` paragraph
      and points to `os::exit` (keeping the worker/cleanup/TRAP text, now about the call);
      `08_error-model.md` completion forms read "`RETURN`, `FAIL` and a plain `os::exit(...)`
      statement"; `04_ir.md` §4 lowering map: `ExitProgram` comes from a statement-position
      `os::exit`; `08_ir-section.md` op 13 description; the rest reword in place.
      `mfb man os` renders `exit` (`./target/release/mfb man os exit`).
- [ ] Re-baseline only the 6 `.ast` goldens whose sole diff is the dropped `"code": null`
      key (inspect each diff; any other line moving is a bug to fix, not to re-baseline).

Acceptance: every touched test passes with its original assertion.
  Check: `cargo test --test rt_os_exit_through_inline_trap --test rt_thread_os_exit_in_worker
  --test rt_resource_review_806 --test rt_native_io_runtime --test rt_debug_report` → pass;
  `bash scripts/test-accept.sh target/release/mfb /tmp/acc-180a 'os-exit-*'` → pass
  (est. 6 min).
Commit: —

## Validation Plan

- Tests: Phase 1 unit + rt fixture; Phase 3 runtime binaries and `os-exit-invalid`.
- Coverage check: `grep -rn "os::exit" tests/rt-error tests/syntax tests/runtime` must hit
  every TRAP form, the worker case and the statement/RETURN positions.
- Runtime proof: `/tmp` program `FUNC main AS Integer : os::exit(5) : END FUNC` exits 5;
  the inline-TRAP binary exits 7 in every case.
- Doc sync: the 11 doc files; `mfb man os exit`; `mfb spec language control-flow`.
- Final gate (once, after Phase 3): `bash scripts/test-fast.sh`; then, because goldens are
  re-baselined, `bash scripts/artifact-gate.sh target/release/mfb all` (15–20 min — only the
  full gate proves no unexpected golden moved) and a full `bash scripts/test-accept.sh
  target/release/mfb /tmp/acc-180a-full` (both required by AGENTS.md before any re-baseline).
  Expected diffs: the 6 `.ast` goldens and the replaced fixtures; anything else is a bug.

## Decisions

- `EXIT PROGRAM` is removed; `os::exit(Integer) AS Nothing` replaces it — owner, 2026-10-19.
- `os::exit` is usable anywhere a builtin call is, including under `TRAP` (the handler never
  runs) — owner, 2026-10-19.
- `PROGRAM` is dropped from the keyword set — owner, 2026-10-19.
- `EXIT_PROGRAM_CODE_OUT_OF_RANGE` is renamed `OS_EXIT_CODE_OUT_OF_RANGE` (same code);
  `TYPE_EXIT_PROGRAM_REQUIRES_INTEGER` is retired to a reserved code — owner, 2026-10-19.
- Statements after `os::exit(...)` are allowed, as after `RETURN` — owner, 2026-10-19.
- No migration hints or back-compat — owner, 2026-10-19.
- Phase 1 keeps emitting the old rule name; the rename lands with the removal in Phase 2, so
  no phase carries two names for one check — author.

## Corrections

## Summary

Risk sits in the statement-position match in `lower.rs` and in keeping the helper chunk
gated. The runtime exit path, IR ops and package format are untouched.
