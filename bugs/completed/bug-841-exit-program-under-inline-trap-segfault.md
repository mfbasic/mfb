# bug-841: EXIT PROGRAM in a callee whose call carries an inline TRAP segfaults instead of exiting

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: FIXED (bs16-agent-a; see Resolution)
Regression Test: tests/runtime/rt_exit_program_through_inline_trap.rs

When a function runs `EXIT PROGRAM n` and its caller invoked it under an inline
`TRAP` (`LET v = bye() TRAP ... END TRAP`, a bare `bye() TRAP ... END TRAP`, or an
indirect `FUNC`-value call under an inline TRAP), the process dies with SIGSEGV
(exit 139) at a null-pointer load instead of terminating with exit code `n`. It
happens at every optimization level (-O0..-O3). The function-level `TRAP` path
handles it correctly (exit 7), as does an untrapped call.

**Correct behavior:** `EXIT PROGRAM` is not catchable by `TRAP`; the inline handler
must be bypassed, lexical cleanup run, and the process exits with code `n`
(here 7).

References:

- `src/docs/spec/language/10_control-flow.md`: "`EXIT PROGRAM` is not catchable by
  `TRAP`: at each returning frame it bypasses any handler, so it propagates past
  every `TRAP` to the entry point."
- Found during spec hardening batch B03 (language 09-15).

## Failing Reproduction

```
mfb init /tmp/x6   # replace src/main.mfb with:
FUNC bye() AS Integer
  EXIT PROGRAM 7
END FUNC
FUNC main() AS Integer
  LET v = bye() TRAP
    RECOVER 1
  END TRAP
  RETURN v
END FUNC

/tmp/hardening/mfb build /tmp/x6 && /tmp/x6/build/x6.out; echo $?
```

- Observed: `Segmentation fault: 11`, exit `139` (same at `-O 0`, `-O 1`, `-O 2`, `-O 3`).
  lldb: `EXC_BAD_ACCESS (code=1, address=0x0)` at `ldr x8, [x23]` followed by
  `add x8, x8, #0x9` — a String-length load through a null message pointer.
- Expected: exit `7`, no output.

Same crash: `bye() AS Nothing` + `bye() TRAP / RECOVER / END TRAP`; the trapped
call one frame up (`mid()` traps `bye()`, `main` calls `mid()`); indirect call
`LET g = bye` / `LET v = g(1) TRAP ...`.

Contrast (works, exit 7): no TRAP at all; a function-level `TRAP(e)` in the
callee or in the caller (`route_current_result_to_trap` checks the tag);
the existing fixture `tests/rt-error/control-flow/exit-program-valid-rt`
(function-level TRAP only — no fixture covers the inline form;
`grep -rl "EXIT PROGRAM" tests` lists no inline-TRAP case).

## Root Cause

`EXIT PROGRAM` unwinds as a `Result` with tag `RESULT_PROGRAM_EXIT_TAG` ("2")
and the message register zeroed (`src/codegen/engine/control/builder_exits.rs:emit_program_exit_value`).
The function-level trap route (`builder_exits.rs:route_current_result_to_trap`)
compares the tag against `RESULT_PROGRAM_EXIT_TAG` and returns instead of
entering the handler. The inline-TRAP lowering does not: it hands the tag to
`src/codegen/memory/arena/builder_arena_transfer.rs:emit_trapped_error_result` /
`emit_trapped_error_block`, where `trapped_error_tag_class` classifies
`RESULT_PROGRAM_EXIT_TAG` as `LooseRegisters` ("os::exit unwinding through a
fallible call") and rebuilds an `Error` from the registers — treating message
register 0 as a String pointer → null dereference. Hypothesis to confirm: the
inline-trap call sites in `builder_values.rs` (direct and indirect callee) and
`materialize_current_result` lack the PROGRAM_EXIT branch-out that
`route_current_result_to_trap` has.

## Goal

- Every inline-TRAP call site re-propagates a PROGRAM_EXIT result (running this
  frame's cleanups) rather than entering the handler; the repro exits 7.

### Non-goals (must NOT change)

- Ordinary error delivery to inline handlers, the ERR_BLOCK adoption path, and
  function-level TRAP behavior.

## Blast Radius

- Inline-trapped direct user call — fails (repro).
- Inline-trapped indirect `FUNC`-value call — fails (repro).
- Inline-trapped builtin HOF (`materialize_current_result` path) — FAILS too:
  `LET v = collections::transform([1, 2, 3], bye) TRAP / RECOVER [] / END TRAP`
  where `bye` runs `EXIT PROGRAM 7` on element 2 → SIGSEGV (exit 139); the same
  call without the inline TRAP exits 7.

## Fix Design

Before building the trapped `Error`, compare the tag with
`RESULT_PROGRAM_EXIT_TAG` and, on equal, take the frame's normal return/cleanup
exit (same shape as `route_current_result_to_trap`).

## Validation Plan

- Regression: rt-error fixture with the repro and its variants, expecting exit 7.
- Full suite per project rules.

## Summary

Silent-safety bug: a documented "uncatchable" exit crashes the process when an
inline TRAP is on the stack.

## Resolution

Hypothesis confirmed: none of the three inline-`TRAP` lowerings (direct callee and
`FUNC`-value callee in `builder_values.rs`, `materialize_current_result` for
runtime helpers / builtin HOFs) checked the program-exit tag before building the
trapped `Error`. Fix: `CodeBuilder::emit_inline_trap_program_exit_passthrough`
(`src/codegen/engine/control/builder_exits.rs`), called at all four places the
raw result registers are parked (both forms of `materialize_current_result`, the
direct and the indirect callee) — before the unboxed (bug-778) split and before
the worker-error tag remap. On tag 2 it reloads `x0 = 2`, `x1 = code`,
`x2 = x3 = 0` and leaves through `emit_call_error_exit` (statement temporaries,
frame cleanups, return; under a function-level `TRAP` the trap route, which
already returns on tag 2). `trapped_error_tag_class` keeps its total partition
(the unit test pins it); its doc now says tag 2 never reaches the assembly.

Regression test: `tests/runtime/rt_exit_program_through_inline_trap.rs` — eight
shapes (direct `LET`, `Nothing` statement, handler that reads `e.message`,
`String` result, one frame up, inline `TRAP` inside a function-level `TRAP`,
`FUNC`-value call, `collections::transform` callback) at `-O0` and `-O3`, each
must exit 7 with no output. RED against the base compiler: 16/16 SIGSEGV.
Cleanup proof: a `--debug` build of a caller holding a `List OF String` and a
`String` local reports `arena.0.alloc_calls 4`, `free_calls 4`,
`live_bytes 0` and exits 7 (`-O0` and `-O3`).

Spec: `mfb spec memory fallible-call-abi` states that a program exit carries
nothing in `x2`/`x3` and that an inline-`TRAP` site re-propagates it. Man:
`mfb man flow` says no `TRAP` catches `EXIT PROGRAM`.

## Golden impact

Every fixture with an inline `TRAP` on a call, an indirect call, or a raw runtime
helper / builtin HOF gains a tag compare and a cold exit branch per site, so their
native-code goldens (`.ncode`/`.ncodesum`/byte-identity covers) are expected to
change. AST/IR goldens and run outputs do not change (the behavior change only
affects programs that `EXIT PROGRAM` under an inline `TRAP`, which crashed before).
