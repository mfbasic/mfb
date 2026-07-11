# bug-03 — Function-level TRAP does not catch inline-emitted failures

Last updated: 2026-07-05

A function-level `TRAP` at the bottom of a FUNC/SUB is specified to trap
**every** error from the body (`./mfb spec language error-model` §8.1/§8.3:
"On failure, control immediately transfers to the enclosing TRAP"). Today it
only traps `FAIL` statements and failures that arrive **through a call
boundary** (user FUNC/SUB calls and helper-backed built-ins such as `fs::*`).
Every error emitted **inline at the failure site** bypasses the enclosing
function-level TRAP and returns the error `Result` directly to the caller:

- conversion built-ins: `toInt`, `toFloat`, `toFixed`, `toByte`,
- checked operators: integer `/` by zero, `+`/`-`/`*`/`^` overflow, `MOD 0`,
  float observation-boundary checks, …
- inline-lowered collection/string members: `collections::get`/`set`/`insert`/
  `removeAt`, `strings::mid`, … (everything that fails via the
  `emit_*_return` family).

The spec's own §8.3 example (`readAge` with `toInt` caught by the bottom
`TRAP`) does not work as documented.

Found 2026-07-05 while verifying the code samples of the new `mfb man tour`
page. A long-standing memory note from plan-02-cleanup already recorded the
symptom ("inline-TRAP-only catches inline-lowered builtins — function-level
TRAP doesn't") without recognizing it as a spec violation.

## 1. Reproduction

```
IMPORT io

FUNC readAge(input AS String) AS Integer
  LET n = toInt(input)
  IF n < 0 THEN FAIL error(77050002, "negative")
  RETURN n

  TRAP(err)
    io::print("Bad age: " & err.message)
    RETURN 0
  END TRAP
END FUNC

SUB main()
  io::print(toString(readAge("nope")))
END SUB
```

Expected (spec §8.3): prints `Bad age: invalid format` then `0`, exit 0.
Actual: the error skips `readAge`'s TRAP, propagates out of `readAge`, and —
`main` having no TRAP — the process dies:
`Code: 77050003 Message: invalid format`, exit 255.

Same shape reproduces with `LET q = a / b` (b = 0 → exit 255 with 77050002)
and `collections::get(xs, 9)` (exit 255 with 77050001). The `FAIL` path and a
failing **user-function call** inside the same body route to the TRAP
correctly; an **inline TRAP** on the same expressions also works.

## 2. Root cause

`emit_error_register_return`
(src/target/shared/code/builder_codegen_primitives.rs:677, tail at ~722) —
the terminal step of every inline error emitter (`emit_invalid_format_return`,
`emit_overflow_return`, `emit_index_out_of_range_return`, …) — ends with:

```rust
if let Some(label) = self.raw_result_capture.clone() {
    self.emit(abi::branch(&label));   // inline-TRAP raw capture
} else {
    self.emit(abi::return_());        // ALWAYS returns to the caller
}
```

It never consults `error_exit_destination()`
(builder_codegen_primitives.rs:996), which is how call-site auto-propagation
routes an error `Result` to the enclosing function-level trap
(`ExitDestination::Trap`). So the inline failure path is compiled as
"return error to caller" regardless of whether the function has a bottom
TRAP. Cross-call failures still get trapped because the **call site's**
propagate branch (emit_current_result_exit + error_exit_destination) runs in
the caller — which is why one TRAP level up appears to work.

## 3. Goal

- Any error raised by an inline-emitted failure path inside a FUNC/SUB body
  with a function-level TRAP routes to that TRAP, exactly like `FAIL` and
  call-boundary failures. The spec §8.3 `readAge` example works as written.
- Cleanup semantics on that path match §8.1: live RES/owned values in scopes
  being exited are dropped before the handler runs (whatever
  `ExitDestination::Trap` already does for call-site propagation).
- No behavior change for functions without a TRAP (error still returns to the
  caller), for inline TRAP raw capture, or for code inside the TRAP body
  itself (`in_trap_body` — errors there must still propagate out, §8.6).

### Non-goals (explicit constraints)

- No language-surface change; no new diagnostics.
- No change to the error `Result` ABI, `Error`/`ErrorLoc` layout, or the
  `_mfb_make_error_result` helper contract.
- plan-21 (inline TRAP **on** inline-lowered builtins) stays independent; this
  bug is about the **function-level** TRAP.

## 4. Current state / notes for the fix

- `error_exit_destination()` already encodes the correct routing decision
  (Trap unless `in_trap_body`); `raw_result_capture` must keep highest
  precedence.
- The trap route presumably needs the pending error in the standard result
  registers plus the scope-drop walk — see how `emit_current_result_exit`
  lowers `ExitDestination::Trap` (builder_emit_helpers.rs call sites) and
  reuse that path rather than duplicating it.
- Beware TRAP-shared cleanup double-run: trap-routed frees are deferred for
  function-level owned locals (see memory note "TRAP cleanup double-free",
  tests/datetime-parse-trap-rt). The new route must go through the same
  deferral.
- `emit_error_register_return` is called from ~every builder file; the fix is
  one choke point, but **native code shifts for every function that has a
  function-level TRAP and any checked op** → full golden regeneration +
  `scripts/artifact-gate.sh` first, then full `scripts/test-accept.sh`.

## 5. Phased fix (test-first)

1. **Tests first.** `tests/trap-function-inline-errors-rt` (runtime proof):
   function-level TRAP catching (a) `toInt` invalid format, (b) integer `/ 0`,
   (c) `collections::get` out of range, (d) control: same ops inside the TRAP
   body still propagate to the caller; plus a RES-cleanup case (open File live
   when the inline failure routes to the TRAP → closed exactly once). Expect
   them to fail today.
2. Route the non-capture branch of `emit_error_register_return` through the
   function-trap exit (same deferral/cleanup path as call-site propagation).
3. Audit sibling terminal emitters (`emit_error_register_return` is the shared
   tail, but grep for any other `abi::return_()` on an error path, e.g. the
   allocation-error path) — the completeness claim requires the whole-tree
   audit.
4. Regenerate goldens; artifact gate; full acceptance; update the plan-02
   memory note.
