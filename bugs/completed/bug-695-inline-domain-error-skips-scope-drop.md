# bug-695: an inline domain error that leaves its function skips the scope-drop walk, leaking every owned local

## STATUS: FIXED (e1c5d3ac1)

Fixed on `worktree-B-695`: RED test `565e81ea9`, fix `e1c5d3ac1`, test-count
corrections `bd7f24b56`, goldens `0d143a7c0`. Chose Open Decision (b), shared
landing pads. Measured per-site walk (a) at +20.84% total instructions over the
56 byte-identity cover builds (macos-aarch64 + linux-x86_64; `vector` +44%);
(b) is +2.92% (max `json` +4.92%). Deviations from the design, each recorded
below under Corrections: the pads are appended after the body rather than
emitted in place; the function-level `TRAP` route was also leaking the
statement temps and is fixed too; four owner-counting codegen suites and one
native-resource close count were corrected with proof.

Reproduction after the fix (macos-aarch64, the doc's own commands): W = a, b,
c, d each report `alloc_calls 5`, `free_calls 5`, `live_bytes 0`.

Last updated: 2026-09-25
Effort: large (3h–1d)
Severity: MEDIUM
Class: Memory-safety (leak)

Status: Fixed
Regression Test: `tests/runtime/rt_domain_error_frees_locals.rs`

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

- [x] Add `tests/runtime/rt_domain_error_frees_locals.rs`: the four shapes above
      plus a `List` and a `String` local, a failing `WITH` value, a raise inside
      a `FOR EACH` body and inside a nested block, with pending statement temps
      (`"a" & toString(n) & toString(10 / v)`), each asserting output and no
      leak. Confirm b/d-class cases fail at HEAD.
- [x] Audit raise sites reachable during cleanup emission.

Commit: 565e81ea9

Audit: `emit_resource_cleanup_call`, `emit_resource_union_cleanup_call`,
`emit_owned_list_drain`, `emit_owned_value_drop` and `emit_pending_temp_free`
contain no raise site. A failing close is recorded in the cleanup-failure
ledger, not raised, and a drop allocates nothing, so the walk is also safe on
an out-of-memory raise. The only re-entry is the trap route's own OOM fallback
(`route_current_result_to_trap` building an `Error`). It re-enters
`emit_error_register_return` with `emitting_error_route` set and still returns
bare. That flag now marks the return route too.

### Phase 2 — the fix

- [x] Route the bare return as designed; measure code size on the covers.

Commit: e1c5d3ac1

### Phase 3 — expected outputs + full validation

- [x] Regenerate the `.ncode` goldens; confirm the delta is only the added
      cleanup at raise sites.
- [x] Scoped suite (the coordinator runs the full suite after merge); re-run
      the reproduction.

Commit: bd7f24b56, 0d143a7c0

## Validation Plan

- Regression test: `tests/runtime/rt_domain_error_frees_locals.rs`.
- Runtime proof: the reproduction's table, before and after.
- Doc sync: `mfb spec memory` / `language memory-semantics` if the error exit's
  cleanup is described there.
- Full suite: `cargo test --no-fail-fast`, `scripts/artifact-gate.sh <mfb> all`.

## Open Decisions

- Per-site walk vs. shared landing pads — decide from the code-size measurement.

## Corrections

1. **The pads go after the body, not at the first site.** The first version
   emitted a pad in place at its first raise site, and later sites branched
   back to it. `cargo test --bin mfb`
   (`cli::build::tests::builtin_codegen_corpora_lower_in_process`) then
   failed. `mfb build -ncode -target linux-riscv64` of `byte-identity/math`
   panicked `rv64 v128: function needs 322 concurrent vector slots, exceeding
   the 127-slot region`, because `v128_live_ranges` reads every backward
   branch as a loop and stretches each vector value across it. Pads are now
   emitted into a side buffer when first requested, so each drop reads that
   point's locals. `append_error_return_pads` (called first in
   `run_register_allocation`) then appends them deepest-first, so every branch
   to a pad is forward. Checked over all 5 targets × every cover: 3,619
   functions, 13,531 pads, 0 backward branches, 0 fall-throughs into a pad.
2. **The trap route leaked too.** With a function-level `TRAP`, the inline
   raise ran the trap-route walk but never freed the statement's pending temps
   (`trappedTemps` case: 25 allocs / 19 frees; the same shape through a failing
   call, `trappedTempsCall`, is clean). Both routes now go through
   `emit_call_error_exit`'s temp free.
3. **Owner-count tests counted error-exit drops as owners.**
   `codegen_for_each_item_drop`, `codegen_return_interior_temp_drop` and
   `codegen_string_return_freshness` count static `_mfb_rt_drop_owned_string`
   sites. At 4ef538bb9 a loop body holding a failing CALL already counts the
   call exit's drop (String 4 vs Integer 2). So "one site = one owner" only
   held for bodies with no walking error exit. Only each file's counting helper
   changed, to skip inline-raise exits (`common::outside_inline_error_exits`).
   All three still pass unchanged against the 4ef538bb9 compiler.
   `rt_native_resource_scope_drop::native_resource_closes_on_fail` expected 1
   closing exit path. The OOM raise while building `error(...)` is a second
   one, and before this fix it left `db` open. The expected count is now 2,
   which fails at 4ef538bb9.
4. **The resource case is part of the regression test.** A `RES` File local
   leaked its 96 B record plus the handle on an inline raise at 4ef538bb9
   (`resourceLocal`: 28 allocs / 22 frees).

## Summary

The fix is one exit; the risk is the size of what it emits at every raise site,
and the golden churn that follows.
