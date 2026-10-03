# plan-169-B: Win64 fixed frames — hoist mid-body call windows into the frame

Last updated: 2026-10-03
Effort: large (3h–1d)
Depends on: plan-169-A

Windows unwinding assumes `rsp` does not move outside a function's prologue and
epilogue. Many Windows runtime helpers break that rule: around each call into a
Windows API they open a **call window** (`sub rsp, K` … `call` … `add rsp, K`)
to make room for the shadow space and stack arguments. While the program is
stopped inside that call (blocked in `ReadFile`, `WriteFile`, `Sleep`, a socket
wait), the helper's frame is `K` bytes larger than its unwind info says, so the
unwinder reads a wrong return address and the backtrace breaks at the helper.
That is exactly where a stuck or I/O-bound program spends its time.

This sub-plan removes every call window on Windows by folding the window's bytes
into the helper's fixed frame, mechanically, in `finalize_frame`. The outcome:
attached to a Windows program blocked inside a Win32 call, gdb's backtrace
passes through the runtime helper and ends at `_main`.

References: plan-169-A (prerequisites, unwind model), `mfb spec linker
windows-x86_64`, `src/codegen/engine/util/vreg_frame.rs:finalize_frame`
(`adjust_stack_instruction_offsets`, `max_outgoing_arg_offset`,
`resolve_stack_arg_sentinels`).

## Prerequisites

The prerequisites are stated in plan-169-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-169-A is complete | `ls planning/plan-169-A-*` → no match (archived) | NOT MET |

## 1. Goal

- No Windows function changes `rsp` between its prologue and its epilogues
  except through encoder scratch pushes (C's scope): the census reports 0
  `sub`-windows.
- On box 206, a backtrace taken while the program is blocked in a Win32 call made
  by a runtime helper ends at `_main`.

### Non-goals (explicit constraints)

- No change to non-Windows targets (the pass runs only for the Win64 backend).
- No change to what each helper does or to the Win64 calling convention at the
  call: the callee still sees its shadow space and stack arguments at the same
  offsets from its own `rsp`.
- Not this sub-plan: encoder `push`/`pop` scratch (C).

## 2. Current State

The call windows are open-coded where each helper is built, for example
`src/target/win_x86_64/code.rs` (`emit_ioctl_fionbio` and the `SHADOW_FRAME`
uses), `src/target/win_x86_64/app/mod.rs`, and the shared builtins with Windows
paths (`process/gen_windows.rs`, `audio/gen_windows_io.rs`,
`tls/gen_schannel*.rs`, `crypto/*`). Inside a window the code addresses the
caller-side frame at `[rsp + K + x]` (comments such as "loaded from `fd_offset`
BEFORE the frame adjust" show authors compensating by hand). Most of these helpers'
frames are built by `finalize_frame` (their prologue is the standard
`sub; mov r12; mov r14` shape, e.g. `_mfb_rt_os_os_sleep`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Call-spanning `sub rsp` windows | 2139 | `p169-windows.py` (plan-169-A Appendix) over the plan-169-A census objdumps + brogue, dungeon, life |
| Distinct functions with such a window | 123 | same, "distinct functions with a window spanning a call" |
| Call-free `sub rsp` windows | 2 | same |
| `subtract_stack`/`add_stack` call sites, all targets | 345 | `grep -rn "abi::subtract_stack(\|abi::add_stack(" src --include='*.rs' \| grep -v "/tests\|vreg_frame.rs" \| wc -l` |

### Verified properties

- UNVERIFIED: every window is **structurally simple**: one `sub rsp, K` and one
  `add rsp, K` on every path through it, properly nested, no label inside that is
  reached from outside, no branch out of it. Phase 1 measures this. It decides
  whether the pass can be mechanical.
- UNVERIFIED: all 123 functions get their frame from `finalize_frame`. Those that
  do not (hand-built) need the same rewrite by hand. Phase 1 lists them.

## 3. Design Overview

A Win64-only pass in `finalize_frame`, run before the frame size is fixed:

1. Find every window: a `subtract_stack(K)` in the body and its matching
   `add_stack(K)`.
2. Reserve `W = max K` bytes at the bottom of the frame (the same place the
   outgoing stack-argument tail lives; take the max of both).
3. Delete each window's `sub`/`add` pair. Inside a window, an `rsp`-relative
   access at `d < K` addressed the window itself, and with `W` at the frame bottom
   it keeps the same offset `d`. An access at `d ≥ K` addressed the frame above
   the window, so it becomes `d − K` and then receives the normal body shift.
   Outside windows, accesses receive the normal body shift. This covers `rsp`
   address arithmetic too (`mov rcx, rsp; add rcx, X`), which
   `adjust_stack_instruction_offsets` already rewrites.
4. The callee sees `[rsp+0..W)` as its shadow and argument area, exactly as before.

**Correctness risk** is in the offset rewrite: a missed or double-shifted access
corrupts a helper's data, silently. Every Windows `.ncode` golden that contains a
window changes (expected), so the gate is behavioral: the Windows runtime tests
and the `man debug` harness on box 206, plus a unit test per access form.

**Design uncertainty** is whether every window is structurally simple (§2). Phase
1 measures it first. Windows that are not simple are rewritten at their source
site to use the frame's outgoing area directly, and the pass refuses (build
error naming the function) any window it cannot prove simple.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Use `- [~]` for partial work, and mark moot tasks
> `- [x] ~~text~~ (moot: <evidence>)`. Fill each `Commit:` line when the phase
> lands. **An unticked box means NOT DONE.**

### Phase 1: Audit the windows (uncertainty first)

- [ ] A test-only analysis over every Windows `CodeFunction` the corpus lowers
      (reuse the codegen corpus tests' plans): report windows that are not
      structurally simple, and functions with windows whose frame is hand-built.
      Record both lists here.
- [ ] Re-measure the 123 / 2139 counts from the final code (not `--debug` builds).

Acceptance: both lists recorded, each entry with its source site.
  Check: the analysis test prints the lists (est. 5 min).
Commit: —

### Phase 2: The pass

- [ ] `vreg_frame.rs`: the window-hoisting pass (§3), Win64 only, refusing an
      unprovable window with an error naming the function.
- [ ] Rewrite at source every non-simple window and hand-built frame from Phase 1.
- [ ] Tests in `vreg_frame.rs`: an access below `K`, at `K`, above `K`, `rsp`
      address arithmetic, nested windows, two windows of different `K`, and the
      refusal.

Acceptance: no Windows function keeps a `sub rsp` window.
  Check: `p169-windows.py` (plan-169-A Appendix) over a fresh Windows census → `('sub', 'call') 0`,
  `('sub', 'no-call') 0` (est. 5 min).
Commit: —

### Phase 3: Behavior on Windows

- [ ] Run every Windows runtime test the suite has (`cargo test` targets that
      execute on box 206, and `scripts/man-debug-examples.sh … windows`) → pass.
- [ ] Box 206: a program blocked in `os::sleep(60000)`; attach gdb, `bt` → the
      chain passes `_mfb_rt_os_os_sleep` and ends at `_main`. Record it.
- [ ] Artifact gate: diffs only in Windows `.ncodesum` goldens of fixtures with a
      window; objdump one function per fixture to confirm the change is the hoist;
      re-pin those sums with the proof in the commit.

Acceptance: the blocked-in-Win32 backtrace ends at `_main`.
  Check: the 206 transcript (est. 5 min).
Commit: —

## Validation Plan

- Tests: Phase 2 unit tests; Phase 3 Windows runtime and harness runs.
- Runtime proof: Phase 3 transcript.
- Doc sync: `mfb spec linker windows-x86_64` (frames are fixed: no `rsp` change
  outside prologue/epilogue except C's scope).
- Final gate: in plan-169-C.

## Open Decisions

- None beyond plan-169-A's.

## Corrections

(Filled in during execution.)

## Summary

The work is one mechanical rewrite in the frame finalizer plus whatever windows
the audit finds that it cannot prove simple. The risk is silent data corruption
from a wrong offset, so the gates are behavioral (Windows runtime tests, a real
blocked-call backtrace), and every changed golden is root-caused.
