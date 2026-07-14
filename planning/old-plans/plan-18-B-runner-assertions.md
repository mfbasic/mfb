# plan-18-B: Assertion builtins + test runner

Last updated: 2026-07-02
Effort: large

Implements the four `expect*` assertion builtins and the `mfb test` runtime: each `TCASE` desugars to
a generated parameterless SUB, a compile-time registration table enumerates them, and a synthesized
driver entry runs every case under per-case trap isolation, streams the pass/fail tree, prints the
summary, and sets the exit code. This is where the correctness risk of the whole feature lives —
trap-guarded evaluation for `expectTrap`/`expectNTrap`, and a single trap catch point in the driver
that separates assertion failures from genuine runtime errors.

Depends on: plan-18-A (surface + `mfb test` retain). Read first: overview
[plan-18-testing.md](plan-18-testing.md) §3, §5; `mfb spec package` (builtin surface); the `error()`/
`ErrorLoc` precedent at `src/ir/lower.rs:2926`; the plan-16 trap-region machinery.

## 1. Goal

- `expectEQ(actual, expected)` / `expectNQ(actual, expected)` — lower to the language `==`/`!=`; on
  mismatch record `(file, line, toString(actual), toString(expected))` into the failure slot and raise
  the internal test-abort trap.
- `expectTrap(expr)` / `expectNTrap(expr)` — evaluate `expr` inside a trap-guarded region; pass/fail
  on whether it trapped; record the appropriate failure detail otherwise.
- Each `TCASE` → a generated parameterless SUB; a static registration table of
  `(group_desc, case_desc, fn_ptr, first_line)` in declaration order.
- Synthesized `mfb test` entry: iterate the table, print each new group header, install per-case trap
  isolation, call the SUB, catch either the test-abort trap (assertion) or a runtime trap (error),
  read the failure slot, stream `[P]`/`[F]` + detail, tally, and exit non-zero iff any failed.

### Non-goals

- No coverage (plan-18-C). No parallelism, setup/teardown, skip/only, or filtering (V1).
- `expectEQ`/`expectNQ` invent no new equality — operands must be `==`-accepted and `toString`-able
  (D2); reject others with the plan-18-A resolver diagnostic surface.

## 2. Current State

- Builtin resolution surface: `src/builtins/general.rs` (`is_general_call:40`, `resolve_call:163`,
  `arity:326`, `call_param_names:95`, `call_return_type_name:117`, `reserved_builtin_name:73`).
  New `src/builtins/testing.rs` registers the four names and is wired into these.
- Lowering of calls + `error()`/`ErrorLoc` call-site stamping: `src/ir/lower.rs` (`error_loc_value` at
  :2926, using `IrFunction::file` at `src/ir/mod.rs:93`).
- Trap regions (`TRAP`/`Propagate`/`Recover`, out-of-line ErrorLoc from plan-16) provide the
  catch/unwind primitive the driver and `expectTrap`/`expectNTrap` reuse.
- `toString` overloads exist for scalars/String (memory: overridable-builtins) — the failure-message
  stringifier.
- Buffered stdout (plan-14): `io::flush` after each printed line for live streaming.
- Entry is the top-level statement body (plan-18-A stub) — the driver replaces it in `test` mode.

## 3. Design

### 3.1 Failure slot + test-abort trap (D3)

A process-global record `TestFailure { kind: {None, Assertion, ExpectTrapMissing, ExpectNTrapTrapped,
RuntimeError}, file, line, actual_str, expected_str, message }`, cleared before each case. A failed
`expect*` writes the slot then raises an **internal test-abort trap** (a reserved error code
registered in `mfb spec diagnostics`, never surfaced to `errorCode::`). The driver wraps each SUB call
in a `TRAP` region; on catch it inspects the slot: if `kind != None` it's an assertion-style failure
(format from the slot); otherwise it's a genuine runtime error (read the propagated `ErrorLoc` +
message). One catch point, two failure classes.

### 3.2 `expectEQ` / `expectNQ`

Compiler-lowered, not a runtime SUB. `expectEQ(a, b)` lowers to roughly:

```
IF NOT (a == b) THEN
    __mfb_test_record(Assertion, <file>, <call-line>, toString(a), toString(b))
    __mfb_test_abort()          ' raises the internal trap
END IF
```

`expectNQ` uses `a != b` / detail "expected not-equal". Call-site `(file, line)` is injected exactly as
`error_loc_value` does (`src/ir/lower.rs:2926`). Because it reuses `==` and `toString`, no new
comparison or formatting codegen is needed.

### 3.3 `expectTrap` / `expectNTrap`

These need genuinely new codegen because the argument must be evaluated *inside* a trap guard (an
eager function call would trap in the caller before the builtin runs). Lower `expr` inside a
trap-guarded region built on the plan-16 machinery:

```
expectTrap(expr):
    <trap-guard start>
        eval expr            ' result discarded
    <normal completion>  → __mfb_test_record(ExpectTrapMissing, file, line, …); __mfb_test_abort()
    <guard caught a trap> → swallow, continue (pass)

expectNTrap(expr):
    <trap-guard start>
        eval expr
    <normal completion>  → pass
    <guard caught a trap> → __mfb_test_record(ExpectNTrapTrapped, file, line, <err msg>); __mfb_test_abort()
```

The guard catches *any* trap from `expr` and routes to the local decision rather than propagating —
distinct from the driver's per-case guard.

### 3.4 `TCASE` → SUB + registration table

Each `TCASE` lowers to a generated parameterless SUB `__test_<g>_<c>` whose body is the case
statements (with `expect*` lowered as above). Emit a static registration table (array of
`{group_desc_ptr, case_desc_ptr, fn_ptr, first_line}`) in declaration order — a compile-time constant
in the `test`-mode plan.

### 3.5 Driver entry

Replaces plan-18-A's stub in `test` mode. Pseudocode:

```
fail_total = 0
for entry in registration_table:
    if entry.group != last_group: print "* " + entry.group; last_group = entry.group
    clear failure slot
    TRAP: call entry.fn_ptr()
    on trap →
        f = read failure slot
        print "  * [F] " + entry.case
        print "    X " + format_detail(f)   ' assertion vs ExpectTrap* vs runtime-error+ErrorLoc
        fail_total += 1
        io::flush
        continue
    ' completed with no trap:
    print "  * [P] " + entry.case; io::flush
print ""
print "Tests: N  Pass: N-fail_total  Fail: fail_total"
exit(fail_total == 0 ? 0 : 1)
```

Per-line `io::flush` gives the streaming tree (plan-14). The exit code is load-bearing for CI.

## Phases

### Phase 1 — Assertion builtin surface + EQ/NQ lowering

- [ ] `src/builtins/testing.rs`: register `expectEQ`/`expectNQ`/`expectTrap`/`expectNTrap`
      (arity, param names, return `Nothing`, reserved); wire into `src/builtins/general.rs`.
- [ ] `__mfb_test_record` + `__mfb_test_abort` runtime helpers + failure-slot global; register the
      internal test-abort error code in `src/docs/spec/diagnostics/**`.
- [ ] Lower `expectEQ`/`expectNQ` in `src/ir/lower.rs` (desugar to `==`/`!=` + record + abort,
      call-site stamped via the `error_loc_value` path).
- [ ] Tests: `tests/func_testing_expectEQ_valid/**` + `_invalid/**` and same for `expectNQ`
      (every `==`-accepted operand type; reject unsupported operands).

Acceptance: a `TCASE` using `expectEQ`/`expectNQ` compiles; a failing one records the right slot and
aborts (observable once the driver lands — cross-checked in Phase 3). Commit: —

### Phase 2 — Trap-guarded `expectTrap` / `expectNTrap`

- [ ] Trap-guarded lowering for both in `src/ir/lower.rs` on the plan-16 trap-region machinery.
- [ ] Tests: `tests/func_testing_expectTrap_valid/**` + `_invalid/**` and `expectNTrap` — including a
      case that *does* trap and one that doesn't for each.

Acceptance: `expectTrap(f())` passes iff `f()` traps; `expectNTrap(f())` inverts; a wrong outcome
records the correct detail. Commit: —

### Phase 3 — `TCASE`→SUB, registration table, driver, reporter (highest-risk)

- [ ] Desugar each `TCASE` to a generated SUB; build the static registration table in `test` mode
      (`src/monomorph.rs` / lowering).
- [ ] Synthesized driver entry replacing the plan-18-A stub, with per-case trap isolation, the
      group/case streaming tree, summary line, and exit code.
- [ ] Runtime-proof fixture: a program mixing a passing case, an `expectEQ` failure, an `expectTrap`
      case, and a case that hits a genuine runtime error → asserts the streamed tree, summary, and
      **exit status**.

Acceptance: `mfb test fixture.mfb` prints the exact tree + `Tests/Pass/Fail` summary and exits
non-zero iff any case failed; the runtime-error case is reported as a failure (not a crash) and
siblings still run. Commit: —

## Validation Plan

- Function tests for all four builtins, valid + invalid, every operand path.
- Runtime proof (Phase 3 acceptance) asserting tree + summary + exit code, including the
  runtime-error-becomes-failure and abort-stops-case-continues-siblings behaviors.
- Doc sync: man/package pages for the four `expect*` builtins (`.ai/man_template.md`); diagnostics
  topic for the internal test-abort code.
- Acceptance: `scripts/test-accept.sh` green.

## Open Decisions

- D2 (operand scope) and D3 (trap-based abort) — resolved per overview recommendations.

## Summary

Risk concentrates in Phase 2 (trap-guarded evaluation) and Phase 3 (the driver's single-catch-point
unwind separating assertion vs runtime failure). EQ/NQ are low-risk because they reuse `==` +
`toString`. Nothing here changes existing type layout or non-test codegen — the driver and table exist
only in `test`-mode plans.
