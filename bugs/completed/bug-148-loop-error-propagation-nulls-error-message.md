# bug-148 — error routed to a function-level TRAP reads a null `Error` (segfault) when the body has an inline `TRAP(e)`

**Status:** FIXED 2026-07-11 (aarch64 host). Root cause was a trap-local
stack-slot desync from an inline `TRAP(e)` reusing the shared name `e`, **not** a
register clobber of the message (the original diagnosis below was a misread — see
"Actual root cause"). The `-regalloc`/spilling angle was a red herring.

## Actual root cause (2026-07-11)
The faulting instruction is `ldr x9,[x8,#8]; add x9,x8,x9` = `message = e +
[e+8]` — i.e. **`x8` is the `Error` block pointer `e`, and it is null**, not
`e.message`. The trap error local `e` was never populated on the executed path.

Why: the function-level `TRAP(e)` allocates its error local `e` once, at a fixed
slot (`[sp+0x30]` in the repro), recorded in `self.locals["e"]`. An **inline**
`TRAP(e)` in the body (`LET picked = toInt(key) TRAP(e) RECOVER 0`) reuses the
*same name* `e`; its lowering is a plain `NirOp::Bind` which unconditionally
`allocate_stack_object`s a **fresh** slot (`[sp+0x1188]`) and overwrites
`self.locals["e"]` — and never restores the outer binding. Consequences:
- Error routes emitted **before** the inline trap (e.g. the first loop statement
  `term::terminalSize()`) store the built `Error` to the original slot
  `[sp+0x30]` (via `self.locals["e"]`, then still correct).
- The inline `TRAP(e)` rebinds `self.locals["e"]` → `[sp+0x1188]` permanently.
- The function-level handler (emitted last) resolves `e.message` from
  `self.locals["e"]` = `[sp+0x1188]` — the wrong slot, holding the prologue's
  zero-init. So `e` reads as null and `e + [e+8]` faults at `[null+8]`.

Confirmed under lldb: at the handler entry `[sp+0x1188]` is `0` (only ever
written by the prologue zero-init — a hardware watchpoint on that slot caught no
other write), while the route for the failing call stored `e` to `[sp+0x30]`.

**Layout-sensitive because** it requires an inline `TRAP(e)` that reuses `e`
*and* at least one function-level error route emitted before that inline trap.
The prior minimal repros (a bare `FAIL`-in-loop → function trap) had no inline
`TRAP(e)`, so no name collision, so no desync — which is exactly why they passed.
Register pressure / spilling was incidental.

## Fix
Pin the function-level trap local's slot in `TrapState.stack_offset` (captured
when the local is allocated in `function_lowering`) and resolve it from there —
never from `self.locals[name]`, which the inline `TRAP(e)` shadows:
- `route_current_result_to_trap` and `emit_direct_error_route_to_trap` store the
  built `Error` to `TrapState.stack_offset`.
- Entering the function-level `NirOp::Trap` body re-inserts `self.locals[name]`
  with the pinned slot, so the handler's `e.message` reads it back.

Validated on the canonical repro: `printf 'q\n' | ./repro.out >/dev/null` now
exits `1` with `life: unsupported operation` on stderr (was exit 139, SIGSEGV,
no output). Full `cargo test` green.

---

## Original report (diagnosis partially wrong — kept for history)

**Layout-sensitive** — see the caveat below; a small reproducer does not trigger it.

## Symptom
When a fallible call **inside a `DO WHILE` loop** fails and its `Error`
auto-propagates to the **function-level `TRAP(e)`** at the bottom of the same
function, the propagated `Error`'s **message pointer is null** in the handler.
The program does not report the error — it **segfaults** (`SIGSEGV`,
`EXC_BAD_ACCESS code=1 address=0x8`) the moment the handler reads `e.message`
(e.g. `io::printError("prefix: " & e.message)`), because the null String payload
is dereferenced at offset 8 (`ldr x9, [x8, #0x8]` with `x8 = 0`).

Discovered while writing `examples/life`: `term::terminalSize()` correctly
fails with `ERR_UNSUPPORTED_OPERATION` when stdout is not a TTY, but instead of
the function-level trap printing that message and returning 1, the process died
with exit 139.

## Reproduction
`bugs/repro/bug-148-loop-trap-propagation.mfb` is the canonical repro (it is
`examples/life/src/main.mfb` with the shipped workaround removed — the
`term::terminalSize()` call reverted to a bare call under the function-level
trap, and the startup terminal guard deleted). Build it as an executable and run
it with stdout redirected to a non-TTY so `terminalSize` fails:

```
printf 'q\n' | ./repro.out >/dev/null   # exit 139 (SIGSEGV), no trap output
```

The relevant shape:

```basic
FUNC main AS Integer
  term::on()
  MUT running AS Boolean = TRUE
  DO WHILE running
    LET size AS TermSize = term::terminalSize()   ' fails on a non-TTY
    ' ... use size ...
    running = FALSE
  LOOP
  RETURN 0

  TRAP(e)
    io::printError("life: " & e.message)          ' <-- e.message is NULL here -> segfault
    RETURN 1
  END TRAP
END FUNC
```

## Diagnosis
Two controlled variants pin the mechanism exactly:

1. **Handler that does NOT read `e.message`** (`io::printError("trap fired") :
   RETURN 7`) → runs correctly, exit 7, "trap fired" on stderr. So control
   *does* reach the function-level trap; the propagation jump is fine.
2. **Handler that reads `e.message`** → segfault at `[null + 8]`.

So the failure is that the propagated `Error`'s **message pointer/length is
clobbered (nulled) on the propagation-from-inside-the-loop path**, and the
handler's `& e.message` reads a null String header. This is the caller-saved
**error-register-lifetime** class: the `RESULT_ERROR_MESSAGE` value (and/or the
`Error` payload pointer) is not preserved across the loop-scope cleanup that runs
between the failing call and the bottom trap (dropping the loop-local bindings /
arena frees, plus `term::` state), analogous to the arena_alloc clobber in
[bug-86](bug-86-riscv-thread-worker-numeric-overflow.md).

`term::terminalSize()` is a good trigger because its error path allocates (it
calls `_mfb_arena_alloc` on the success side and sets a message on the error
side; see `emit_terminal_size` in `src/target/shared/code/term.rs`), but the bug
is about the *propagation* seam, not `terminalSize` specifically.

## Layout sensitivity (why the minimal repros pass)
Reduced programs with the same shape do **not** crash:
- `FAIL error(...)` from inside a `DO WHILE` → function-level trap reading
  `e.message`: **works** (exit 1, message printed).
- `term::terminalSize()` inside a loop with a handful of `MUT`s and a small
  first-seed block: **works**.

The crash only appears at roughly `examples/life` scale (many helper FUNCs, large
list-literal patterns, several nested loops in the loop body). Adding/removing
code shifts register allocation and masks or unmasks it — exactly the
layout-sensitivity `.ai/compiler.md` warns about and that bug-86 exhibits. A
passing acceptance run therefore does **not** prove a fix.

## Investigation findings (2026-07-11, session)
Confirmed under lldb on the repro: the crash is `ldr x9, [x8,#8]` with `x8 = 0`,
where `x8 = [sp+0x1188] = e.message` — the built `Error`'s message FIELD is null,
so `route_current_result_to_trap` assembled the trap `Error` with a null message
(`RESULT_ERROR_MESSAGE`/x2 was 0 at the route). Traced every seam and they are all
register-safe by construction: `emit_terminal_size` returns a valid static message
pointer in x2; the runtime-helper propagate path (`emit_runtime_helper_call`) →
`emit_stamp_current_error_source` (spills/reloads the message around its arena
alloc) → `emit_current_result_exit` (`store_pending`/`load_pending` cover
message+source) → `route_current_result_to_trap` (spills immediately). A targeted
fix (spill x2 right after the call, reload right before the exit) did NOT resolve
it, so the clobber is deeper than these seams. It only manifests at a scale that
forces SPILLING — `-regalloc bump` (the fixed-pool, no-spill oracle) cannot even
compile the program (exhausts registers), so the bug lives in the linear-scan
spill/eviction interaction with the fixed error-result register (x2) lifetime,
possibly related to the eviction path (cf. bug-127.2) using x2 as a scratch while
it holds the live message. Needs an lldb hardware-watchpoint on the
`[sp+0x1188]`-equivalent slot at life scale to catch the exact write of 0.
Workaround shipped; not a correctness regression from this session's work.

## Fix direction (deferred — needs a layout-sensitive audit)
On the auto-propagate path from a failing call to the enclosing function-level
`TRAP`, spill the `Error` payload registers (`RESULT_ERROR_MESSAGE` /
`RESULT_ERROR_SOURCE` and any pointer/length derived from them) to stack slots
before running loop-scope lexical cleanup (the `bl` to arena free / drop
helpers), and reload them at the trap entry. Same register-lifetime remedy as
bug-86, applied to the structured `PROPAGATE`-to-function-trap seam rather than
the `thread::waitFor` finalization seam. Validate with a **no-`io::print`-in-body**
reproducer at life scale that asserts the handler sees a non-null `e.message`
(exit `1`, message on stderr), not just that the process exits.

## Workaround (shipped in examples/life)
Handle the fallible call with an **inline `TRAP`** so the error is consumed
before any scope unwind, instead of letting it propagate to the function-level
trap from inside the loop:

```basic
LET size AS TermSize = term::terminalSize() TRAP(e)
  EXIT DO
END TRAP
```

`examples/life` also guards at startup (`io::isOutputTerminal` /
`io::isInputTerminal`) so the TUI never runs — and `terminalSize` never fails —
outside an interactive terminal. Both together make the example crash-free on
every invocation (TTY: normal play; pipe/file: clean "run this from an
interactive terminal." message, exit 1).
