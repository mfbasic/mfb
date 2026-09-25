# bug-695: an inline domain error that leaves its function skips the scope-drop walk, leaking every owned local

Last updated: 2026-09-24
Effort: large (3h–1d)
Severity: MEDIUM
Class: Memory-safety (leak)

Status: Open
Regression Test: none yet — `tests/runtime/rt_domain_error_frees_locals.rs` (Phase 1)

When an inline builtin or operator raises — a failing conversion (`toInt`), a
division by zero, an integer overflow, an index out of range, an allocation
failure — inside a function that has no function-level `TRAP`, the error goes
back to the caller without freeing anything the function owns. Every owned
local that is live at the raise (a record, a list, a `String`) leaks, and so
does every temporary the failing statement had built. A failing CALL and a
`FAIL` do free them, so the leak depends only on *which* thing failed.

It is silent: the program's output is right and only the `--debug` report
shows it. It is per failure, so a loop that traps a failing conversion per
iteration (`toInt` over untrusted input) grows without bound.

**Correct behavior after the fix:** an inline raise that leaves the function
frees exactly what a failing call at the same point frees — the statement's
pending temporaries and every active cleanup — then returns the error. A
program that traps such failures ends with `alloc_calls = free_calls` and
`live_bytes 0`.

References:

- `mfb spec language memory-semantics` §14.7: every live value drops on every
  scope edge. An error edge is one.
- `emit_call_error_exit` (`src/codegen/engine/control/builder_exits.rs`): the
  call-boundary form, which does this.
- Found while fixing bug-689: before that fix, its failure-atomicity test
  (`a_failing_element_update_leaves_the_list_unchanged`) leaked 1 to 4 blocks
  per failing update — the owned element copy `MUT p = get(…)` had made, and
  in the single-expression spelling the statement's temporaries too.

## Failing Reproduction

`bugs/repro/bug-695-domain-error-leaks-owned-locals.mfb`:

```
mfb init /tmp/p && cp bugs/repro/bug-695-domain-error-leaks-owned-locals.mfb /tmp/p/src/main.mfb
mfb build -q --debug /tmp/p
for w in a b c d; do W=$w /tmp/p/build/p.out 2>&1 | grep -E '^arena.0.(alloc|free)_calls|live_bytes'; done
```

Observed at `aa644f908`, macos-aarch64:

| W | failure | `alloc_calls` | `free_calls` | `live_bytes` |
| --- | --- | --- | --- | --- |
| a | a user call fails (`conv(text)`) | 5 | 5 | 0 |
| b | `toInt(text)` inline | 5 | 4 | **16** |
| c | `FAIL error(…)` | 5 | 5 | 0 |
| d | `10 / v`, `v = 0` | 5 | 4 | **16** |

- Observed: the 16-byte `D` record `q` leaks in b and d.
- Expected: `alloc_calls = free_calls`, `live_bytes 0` for every W.

The same holds for a `List` local: `LET xs = [1, 2, 3]` then a failing `toInt`
leaks its 64 bytes.

Contrast: with a function-level `TRAP` the raise routes to the handler through
`emit_current_result_exit(ExitDestination::Trap)`, which runs the trap-route
cleanups; nothing leaks.

## Root Cause

`emit_error_register_return` (`src/codegen/error/emission/builder_error_emission.rs`)
is where every inline raise (`raise_error` → `raise_error_bare` →
`emit_error_code_return`) ends. It has three exits:

- inside a raw-capture region (an inline `TRAP` on this builtin): branch to the
  capture label — correct, the statement continues;
- a function-level `TRAP` is active: `emit_current_result_exit(Trap)` — runs the
  trap-route cleanups;
- otherwise: **`self.emit(abi::return_())`** — a bare return. No
  `emit_pending_temp_frees_in_place`, no `emit_cleanups(active_cleanups)`.

`emit_call_error_exit` is the same exit for a failing call, and it does both:
it frees the statement's pending temps and then calls
`emit_current_result_exit(self.error_exit_destination())`, whose `Return` arm
walks `active_cleanups`.

## Goal

- The reproduction reports `alloc_calls = free_calls` and `live_bytes 0` for
  b and d, as it does for a and c.
- A trapped inline raise in a loop leaks nothing per iteration.

### Non-goals (must NOT change)

- The error value (code, message, `ErrorLoc`) and where it is delivered.
- The raw-capture path (an inline `TRAP` on the raising builtin itself).
- The function-level `TRAP` route.
- **Tempting wrong fix: free only the named locals.** The failing statement's
  pending temporaries leak the same way (`emit_call_error_exit`'s comment
  records the `boom(n, [])` case for calls); both halves are needed.

## Blast Radius

- `emit_error_register_return`'s bare `return_()` — fixed by this bug.
- 244 raise call sites (`raise_error(`, `raise_error_bare(`,
  `emit_error_code_return(` under `src/codegen`, by `grep -c`) — every one
  reaches the bare return, so every function with an owned local and an inline
  raise is affected. Fixed at the one exit.
- `emit_call_error_exit` — correct today (contrast).
- The raw-capture and `Trap` branches — correct today.
- Raise sites reached while emitting cleanups themselves (a close helper's
  failure, an allocation failure inside a drop) — must be audited: running the
  walk from inside the walk would double-free. `emitting_error_route` already
  guards the `Trap` branch against re-entry; the `Return` route needs the same.

## Fix Design

Route the bare-return branch through the call-boundary exit:
`emit_pending_temp_frees_in_place` (parking the error registers around it, as
`emit_call_error_exit` does) then `emit_current_result_exit(Return)`, under the
`emitting_error_route` guard.

**Where the risk is: code size.** The walk is emitted per raise site, and raise
sites are dense — every checked `Integer` `+`, `-`, `*`, every conversion, every
bounds check. A function with k owned locals and n raise sites gains O(n·k)
instructions, and every `.ncode` golden of such a function moves. Measure the
per-target code-size growth over the byte-identity covers before choosing:

- **(a) per-site walk**, as calls do today — simplest, largest;
- **(b) shared landing pads** — one label per cleanup depth that frees that
  depth's cleanup and falls through to the next, ending in the return; a raise
  parks the error registers, frees its statement temps, and branches to the pad
  for the current depth. O(k) per function instead of O(n·k).

Recommended: (b) if (a) grows any cover by more than a few percent.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/runtime/rt_domain_error_frees_locals.rs`: the four shapes above
      plus a `List` and a `String` local, a failing `WITH` value, a raise inside
      a `FOR EACH` body and inside a nested block, with pending statement temps
      (`"a" & toString(n) & toString(10 / v)`), each asserting output and no
      leak. Confirm b/d-class cases fail at HEAD.
- [ ] Audit raise sites reachable during cleanup emission.

Commit: —

### Phase 2 — the fix

- [ ] Route the bare return as designed; measure code size on the covers.

Commit: —

### Phase 3 — expected outputs + full validation

- [ ] Regenerate the `.ncode` goldens; confirm the delta is only the added
      cleanup at raise sites.
- [ ] Full suite; re-run the reproduction.

Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_domain_error_frees_locals.rs`.
- Runtime proof: the reproduction's table, before and after.
- Doc sync: `mfb spec memory` / `language memory-semantics` if the error exit's
  cleanup is described there.
- Full suite: `cargo test --no-fail-fast`, `scripts/artifact-gate.sh <mfb> all`.

## Open Decisions

- Per-site walk vs. shared landing pads — decide from the code-size measurement.

## Summary

The fix is one exit; the risk is the size of what it emits at every raise site,
and the golden churn that follows.
