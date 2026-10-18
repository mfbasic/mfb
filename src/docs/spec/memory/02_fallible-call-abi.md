# Fallible-Call Result ABI

A native fallible call returns its outcome in four MFB result roles,
`%retMFB0` through `%retMFB3`. The register names below are their AArch64
realizations; the other targets' banks are specified in
`./mfb spec memory native-calling-convention`.
[[src/codegen/error/constants/error_constants.rs:RESULT_TAG_REGISTER]]

```text
x0  tag       0 = success, 1 = loose error, 2 = program exit, 3 = parked error
x1  value     success: the result value (unspecified for Nothing); error: the Error code
x2  message   error: pointer to the error message string
x3  source    error: pointer to the origin ErrorLoc (0 = no origin)
```

The tags are `RESULT_OK_TAG` (`0`), `RESULT_ERR_TAG` (`1`),
`RESULT_PROGRAM_EXIT_TAG` (`2`), and `RESULT_ERR_BLOCK_TAG` (`3`);
the four registers are `RESULT_TAG_REGISTER`
(= the return register `x0`), `RESULT_VALUE_REGISTER` (`x1`),
`RESULT_ERROR_MESSAGE_REGISTER` (`x2`), and `RESULT_ERROR_SOURCE_REGISTER` (`x3`). [[src/codegen/error/constants/error_constants.rs:RESULT_OK_TAG]]
The program-exit tag is checked before the success/error split: at the program
entry it routes `x1` (the exit code) to the process return register and jumps to
exit, so it is distinct from both a normal success and an error. A program exit
carries nothing in `x2`/`x3`, so no consumer may treat it as an error. An inline
`TRAP` site checks it before it builds the trapped `Result`: it leaves the frame
the way an untrapped failing call does (the statement's temporaries and the
frame's cleanups, then a return with the same four registers), so the handler
never runs. [[src/codegen/engine/control/builder_exits.rs:emit_inline_trap_program_exit_passthrough]]

On success only `x0`/`x1` are meaningful. A `Nothing` success leaves `x1` unspecified: a `Nothing` return writes no value register. [[src/codegen/engine/control/builder_exits.rs:emit_return_exit_inner]] A caller that uses the call's value produces `NOTHING` (`0`) itself — for the call, and for the payload of a trapped call's `#Result OF Nothing` — and never reads `x1`. [[src/codegen/engine/value/builder_values.rs:lower_value]] On error all of `x1` (code), `x2`
(message), and `x3` (source) are set. A fresh error stamps `x3` with an
`ErrorLoc` built from the originating expression's `(file, line, char)`; a runtime
helper error is stamped at its call site; a propagated error forwards `x3`
unchanged so the origin is preserved. A null `source` (`x3 == 0`) is a valid,
origin-less error (an OOM-degraded error built when no `ErrorLoc` could be
allocated, or a native `LINK` thunk's gate/marshaling failure — a thunk has no
MFBASIC expression to stamp, so it sets `x3 = 0` on **every** exit rather than
leaving the argument register it shares as-is). Setting `x3` is not optional: a
caller that materializes the loose error reads it, so a stale argument value
there is an invalid `ErrorLoc` pointer, not a missing origin. [[src/codegen/link/thunk/link_thunk.rs:lower_link_thunk]]

In the **registers**, `x2` and `x3` are **absolute pointers**. In the in-arena
`Error`/`ErrorLoc` records, however, `message` and `source` are stored as
**block-relative offsets** (see *Native Heap Value Layouts*), with offset `0` as
the null sentinel. For a loose error (tag `1`), trapping materializes a
3-field `Error` record from `x1`/`x2`/`x3` (absolute → offset), and
`FAIL <error>` / `emit_load_error_fields` loads `x1`/`x2`/`x3` back from the
`Error` value's `code`/`message`/`source` fields (offset → absolute, mapping a
0 offset back to a null pointer). [[src/codegen/error/emission/builder_error_emission.rs:emit_load_error_fields]]

## Parked error ownership

Tag `3` carries the same code/message/source register view as tag `1`, plus
an owned flat `Error` block in the current arena's `ARENA_CURRENT_ERROR_OFFSET`
slot. Register pointers can point into that block. The catcher distinguishes
this tag: it adopts the block and clears the slot rather than reconstructing
an error from the loose registers. A function-level trap uses the adopted
block as its error local. An inline trap whose `Result` temporary lives in frame
slots (`./mfb spec memory heap-values`, *`Result`*) keeps the adopted block as the
`Error` its handler binds, and frees it at once when the handler does not read
the error. An inline trap with a heap `Result` embeds the block in it and reclaims
the adopted outer block after copying. Other tags must not adopt the slot.
The `Error` is built only on the error path, so in the frame-slot form a
successful trapped call allocates nothing for its outcome.
[[src/codegen/error/emission/builder_error_emission.rs:emit_adopt_current_error_block]]
[[src/codegen/engine/control/builder_exits.rs:route_current_result_to_trap]]
[[src/codegen/memory/arena/builder_arena_transfer.rs:emit_trapped_error_result]]
[[src/codegen/error/result/unboxed_trap.rs:finish_unboxed_trap_result]]

Error propagation preserves the parked ownership until a catcher takes it.
A runtime helper's parked error is restamped with the call-site origin by
replacing the parked block and updating its register view; a loose helper
error is stamped through the loose-register path. An allocation failure can
leave an origin-less loose error, so consumers support both forms.
[[src/codegen/error/emission/builder_error_emission.rs:emit_stamp_runtime_helper_error]]
[[src/codegen/error/emission/builder_error_emission.rs:emit_park_error_block_from_registers]]

## Leaving a function on an error

An error that leaves a function is a scope edge (`./mfb spec language
memory-semantics` §14.7): before the four registers reach the caller, or the
function-level `TRAP`, the function frees the failing statement's pending
temporaries and drops every active cleanup. The drop does not depend on what
failed. A failing call does it at the call site. [[src/codegen/engine/control/builder_exits.rs:emit_call_error_exit]]
So does an inline raise (a failing conversion, a division by zero, an overflow,
a bounds check), unless an inline `TRAP` on that builtin captures it. [[src/codegen/error/emission/builder_error_emission.rs:emit_error_register_return]]

Inline raise sites are dense, so their return route does not repeat the walk
at every site. The site stores `x0`–`x3` in the function's pending-result
slots and frees its own statement's temporaries. It then branches to a
landing pad shared by every site with the same cleanup stack. The pad for
`[c0..ck]` drops `ck` and continues into the pad for `[c0..ck-1]`. The empty
stack's pad reloads the registers and returns. So a function emits one drop
per distinct cleanup-stack prefix. [[src/codegen/engine/control/builder_exits.rs:emit_inline_error_exit]]
The pads are placed after the function body, deepest prefix first, so every
branch into a pad and between pads is forward. A backward branch would read as
a loop to the live-range analyses. [[src/codegen/engine/control/builder_exits.rs:append_error_return_pads]]
A check's own raise block (a bounds, overflow or shift-count check in a compiled
function) that leaves by `ret` or by a branch to a pad is placed out of line too,
between the body and the pads, so the passing path falls through a check that is
one untaken branch and every branch into or out of the block stays forward. A
raise block that branches anywhere else — a `TRAP` handler, an inline `TRAP`'s
capture point — stays inline. [[src/codegen/engine/builder/builder_emit_helpers.rs:emit_cold_block]]
The trap route keeps its per-site walk. The one exception to all of this is
the trap route's own out-of-memory fallback. It re-enters the raise while that
route is still dropping, so it returns directly instead of starting a second
walk.

## See Also

* ./mfb spec language error-model — the `Result` and error semantics this register ABI implements
* ./mfb spec memory runtime-helper-abi — the runtime-helper calling convention this pairs with
* ./mfb spec memory native-calling-convention — the general native register conventions
