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
passes through the runtime helper, reaches `_main`, and continues only through
`kernel32`/`ntdll` frames (plan-169-A §1: `_main` is not the outermost frame).

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
  by a runtime helper reaches `_main`, with only system-DLL frames past it and
  no `??`.

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
- UNVERIFIED: every window size `K` is a multiple of 16. Deleting a window of
  `K ≡ 8 (mod 16)` flips the alignment of `rsp` at every call inside it. At
  least one such window exists: `emit_finish` in `src/target/win_x86_64/app/mod.rs`
  opens `subtract_stack(0x28)` (`grep -rn "subtract_stack(0x28)" src`). Phase 1
  measures it; an odd-8 window is not simple.
- Windows nest. `Win64Backend::emit_arena_start_time`
  (`src/target/win_x86_64/code.rs`) opens a 16-byte buffer window
  (`subtract_stack(16)`, the FILETIME out-buffer at `[rsp+0]`) and, inside it,
  `call_external` opens its `SHADOW_FRAME` (0x20) window. Read
  `emit_arena_start_time` and `call_external`. That one is in `_main`
  (hand-built), but the shape (a data-buffer window around a call that opens
  its own shadow window) is what §3 must handle in any function.
- UNVERIFIED: all 123 functions get their frame from `finalize_frame`. Those that
  do not (hand-built) need the same rewrite by hand. Phase 1 lists them.

## 3. Design Overview

A Win64-only pass in `finalize_frame`, run before the frame size is fixed:

1. Find every window: a `subtract_stack(K)` in the body and its matching
   `add_stack(K)`, with its nesting depth. At any instruction, the open windows
   form a stack `K_1` (outermost) … `K_n` (innermost); their **depth sum** is
   `S = K_1 + … + K_n`, exactly how far `rsp` sits below the body `rsp` there.
2. Reserve `W = max S` over all instructions (the deepest nesting, *not* the
   largest single `K`) at the bottom of the frame (the same place the outgoing
   stack-argument tail lives; take the max of both). With `max K` instead, a
   nested pair such as `emit_arena_start_time`'s (16-byte buffer around a 0x20
   shadow window) would put the outer buffer inside the inner callee's shadow
   space, which the callee may overwrite.
3. Delete each window's `sub`/`add` pair, keeping the windows' original stacked
   layout at the frame bottom: the innermost open window at `[rsp+0..K_n)`, the
   next outward above it, and so on. An `rsp`-relative access at offset `d`,
   made while windows of depth sum `S` are open, addressed `body_rsp − S + d`
   before the pass. Body `rsp` moves down by `W`, so the access becomes
   `d + W − S`, whichever window (or the frame above them all) it lands in.
   - At the deepest point (`S = W`) every offset is unchanged, so the callee
     sees `[rsp+0..K_n)` as its shadow and argument area, exactly as before.
   - In a shallower window (`S < W`) offsets move up by `W − S`, and the
     window's bytes sit at the same depth below body `rsp` as before.
   - Outside windows (`S = 0`) accesses receive the normal body shift `W`.
   This covers `rsp` address arithmetic too (`mov rcx, rsp; add rcx, X`), which
   `adjust_stack_instruction_offsets` already rewrites; it now needs the
   per-instruction `S`.
4. A call inside a shallower window now runs with `rsp` `W − S` bytes lower than
   before. The callee still finds its shadow space at `[rsp+0..0x20)` and its
   stack arguments where the caller stored them (the stores are rewritten by the
   same rule), so the call contract holds. `W` is a multiple of 16 when every
   `K` is (step 5), so alignment holds.
5. Refuse (§Design uncertainty) any window with `K % 16 ≠ 0`: deleting it changes
   call-site alignment. Rewrite those at source (e.g. `emit_finish`'s 0x28 frame
   is really a hand-built function frame and moves into its prologue).

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
- [ ] Report every window with `K % 16 ≠ 0` and its source site
      (`emit_finish`'s `subtract_stack(0x28)` is one).
- [ ] Report the maximum nesting depth seen and every function where it exceeds
      1, with the window sizes at each level.
- [ ] Re-measure the 123 / 2139 counts from the final code (not `--debug` builds).

Acceptance: both lists recorded, each entry with its source site.
  Check: the analysis test prints the lists (est. 5 min).
Commit: —

### Phase 2: The pass

- [ ] `vreg_frame.rs`: the window-hoisting pass (§3), Win64 only, refusing an
      unprovable window with an error naming the function.
- [ ] Rewrite at source every non-simple window and hand-built frame from Phase 1.
- [ ] Widen plan-169-A's encoder guard: on Win64, any `rsp`-changing body
      instruction that is not an epilogue `add rsp, imm; ret` and not an encoder
      scratch push (C's scope) is an error naming the function.
- [ ] Tests in `vreg_frame.rs`: an access below `K`, at `K`, above `K`, `rsp`
      address arithmetic, two windows of different `K`, nested windows
      (the `emit_arena_start_time` shape: a 16-byte buffer window whose address
      is taken, around a call that opens a 0x20 shadow window, with a read of
      the buffer after the inner window closes; assert `W` is the depth sum and
      the buffer does not overlap `[rsp+0..0x20)`), and both refusals (an
      unprovable window; a `K % 16 ≠ 0` window).

Acceptance: no Windows function keeps a `sub rsp` window.
  Check: `p169-windows.py` (plan-169-A Appendix) over a fresh Windows census → `('sub', 'call') 0`,
  `('sub', 'no-call') 0` (est. 5 min).
Commit: —

### Phase 3: Behavior on Windows

- [ ] Run every Windows runtime test the suite has (`cargo test` targets that
      execute on box 206, and `scripts/man-debug-examples.sh … windows`) → pass.
- [ ] Box 206: a program blocked in `os::sleep(60000)`; attach gdb, `bt` → the
      chain passes `_mfb_rt_os_os_sleep`, reaches `_main`, then only
      `kernel32`/`ntdll` frames, no `??`. Record it.
- [ ] Artifact gate: diffs only in Windows `.ncodesum` goldens of fixtures with a
      window; objdump one function per fixture to confirm the change is the hoist;
      re-pin those sums with the proof in the commit.

Acceptance: the blocked-in-Win32 backtrace reaches `_main`, with only
system-DLL frames past it.
  Check: the 206 transcript (est. 5 min).
Commit: —

## Validation Plan

- Tests: Phase 2 unit tests; Phase 3 Windows runtime and harness runs.
- Runtime proof: Phase 3 transcript.
- Doc sync: `mfb spec linker windows-x86_64` (frames are fixed: no `rsp` change
  outside prologue/epilogue except C's scope; nested call windows are laid out
  at the frame bottom by depth sum).
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
