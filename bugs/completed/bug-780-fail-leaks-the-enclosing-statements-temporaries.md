# bug-780: a `FAIL` leaks the pending temporaries of the statements that enclose it

Last updated: 2026-09-28
Effort: small (<1h)
Severity: HIGH — an unbounded leak under retry (one block per failure) on the most
common failure shape, `IF <condition with a temporary> THEN FAIL …`
Class: Memory-safety (leak)

Status: Fixed
Regression Test: `tests/runtime/rt_fail_frees_enclosing_temps.rs`

## STATUS: FIXED

`emit_error_value_exit` (and its two direct paths) now frees the pending temporaries
below the `FAIL` statement's watermark — the enclosing statements' temps — before
leaving, as a failing call's exit already does. The `FAIL`'s own temps (above the
watermark) are still adopted by the catcher. Found while writing bug-777's leak test:
its callbacks' `IF s = longKey() THEN FAIL …` leaked 64 B per failure at every site,
including the ones bug-777 does not touch.

`IF s = longKey() THEN FAIL error(…) END IF` holds `longKey()`'s block as a pending
temporary of the `IF` statement, freed at the `IF`'s end. The `FAIL` jumps past that
end. A failing call at the same point frees it (`emit_call_error_exit` →
`emit_pending_temp_frees_in_place`), and so does a `RETURN` (`emit_return_exit` frees
before its `ret`), but `FAIL`'s exit — `builder_exits.rs:emit_error_value_exit`,
`builder_error_emission.rs:emit_direct_error_return` / `emit_direct_error_route_to_trap`
— never touched the pending set, so every failure leaked the condition's block. In the
-O1 `.ncode` of the failing function the out-of-memory branches inside the `Error`
record build (which exit through `emit_call_error_exit`) called
`_mfb_rt_drop_owned_string`; the normal `FAIL` path did not.

**The single correct behavior:** a program that fails through a `FAIL` N times ends
with `alloc_calls = free_calls` and `live_bytes 0`, whichever `TRAP` catches it.

## Failing Reproduction (measured, `--debug`, 50 failures, pre-fix compiler)

```basic
FUNC longKey() AS String
  RETURN "fail-fail-fail-fail-fail-fail-fail-fail-fail!"
END FUNC
FUNC check(s AS String) AS Integer
  IF s = longKey() THEN
    FAIL error(77050002, "boom")
  END IF
  RETURN 0
END FUNC
' tryOnce(s) = check(s) under a function-level TRAP returning -1; main calls it 50×
```

| case | pre-fix leak | fixed |
|---|---|---|
| return to the caller's `TRAP` | 50 blocks, 3200 B | 0 |
| … with an owned local (cleanup path) | 50 / 3200 B | 0 |
| route to the failing function's own `TRAP` | 50 / 3200 B | 0 |
| … with an owned local | 50 / 3200 B | 0 |
| nested `IF`s, each with a temporary | 100 / 6400 B | 0 |
| `WHILE` condition (guard) | 0 | 0 |

Contrast (no leak before the fix): `RETURN` in the same `IF`; a propagating failing
call in the same `IF`; the value bound by `LET` first; a temporary only in the `FAIL`'s
message.

## Non-goals

No language-spec or memory-model change: this makes `FAIL` obey the scope-edge rule
`mfb spec memory fallible-call-abi` already states. The `FAIL`'s own `Error` block is
still parked and adopted by the catcher (design "b"), never freed here.

## Blast Radius

`emit_error_value_exit` has one caller, the `NirOp::Fail` arm of
`builder_control.rs`; the fix covers its three paths (direct return, direct trap
route, cleanup-bearing). Every other error exit already freed the pending set
(`emit_call_error_exit`, `emit_inline_error_exit`).

## Phases

- [x] RED: `tests/runtime/rt_fail_frees_enclosing_temps.rs` (5 cases leaked on the
      pre-fix compiler via `MFB_TEST_EXE`).
- [x] Fix: `emit_pending_temp_frees_in_place_below(watermark)` on all three `FAIL` exit
      paths; the `FAIL` arm passes its statement watermark.
- [x] Full validation with bug-777 (same branch).
