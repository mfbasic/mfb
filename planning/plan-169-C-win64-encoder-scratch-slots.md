# plan-169-C: Win64 encoder scratch in frame slots, and plan-169 closeout

Last updated: 2026-10-03
Effort: large (3h–1d, including the final suite)
Depends on: plan-169-B

The x86-64 encoder expands several single instructions into short sequences that
need a temporary register, and it preserves that register with `push`/`pop`:
the high-half multiply behind overflow checks (`umulh`/`smulh`: `push rax;
push rdx; …; pop rdx; pop rax`), bit reversal (`rbit`), the ties-away float
conversion (`fcvtas_x_from_d`), the packed f64↔i64 conversions (`fcvtzs_v`,
`scvtf_v`), and variable shifts into `rcx` (`var_shift`, `var_shift_w`). Each
`push` moves `rsp` for a few instructions. A debugger or profiler stop inside one
of those windows, or a divide fault raised there, unwinds frame `#0` with the
wrong `rsp` and loses the chain. In hot arithmetic (every overflow-checked
multiply) these windows are common.

This sub-plan makes the encoder save its scratch to a **reserved slot in the
frame** on Win64 instead, so `rsp` never moves in a body. Then it closes plan-169:
documentation and the full suite. The outcome: on box 206, twenty consecutive
gdb attaches to a busy program all give backtraces that reach `_main`.

References: plan-169-A, plan-169-B, `src/arch/x86_64/encode/emitter.rs`.

## Prerequisites

The prerequisites are stated in plan-169-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-169-B is complete | `ls planning/plan-169-B-*` → no match (archived) | NOT MET |

## 1. Goal

- On Win64 no function changes `rsp` outside its prologue and epilogues: the
  census reports 0 `push` windows and 0 `sub` windows.
- Twenty attaches on box 206 to `debug 3000 20000` (the `--debug -O0` example)
  each give a backtrace that reaches `_main` with only `kernel32`/`ntdll` frames
  past it and no `??` (plan-169-A §1: `_main` is not the outermost frame).
- The full suite passes once.

### Non-goals (explicit constraints)

- linux-x86_64 keeps its encoding (SysV has no unwind-table requirement in this
  plan): the slot form is Win64 only. linux-x86_64 `.ncode` unchanged.
- No change to arithmetic results or error behavior.

## 2. Current State

Encoder sites that `push`/`pop`, all in `src/arch/x86_64/encode/emitter.rs`
(`grep -n "push rax\|0x50 + \|push rcx\|enc_push_reg(scratch)" src/arch/x86_64/encode/emitter.rs`
→ 9 hits), by match arm or function:

| Site | Pushes | Max qwords live |
|---|---|---|
| `"rbit"` arm | `rax`, `rdx` | 2 |
| `"umulh" \| "smulh"` arm | `rax`, `rdx` (each skipped when it is `dst`) | 2 |
| `"fcvtas_x_from_d"` arm | `rax` or `rcx` (whichever is not `dst`) | 1 |
| `"fcvtzs_v"` arm | `rax`, `rdx` | 2 |
| `"scvtf_v"` arm | `rax`, `rdx` | 2 |
| `fn var_shift`, `dst == rcx` path | scratch | 1 |
| `fn var_shift`, other path | `rcx` | 1 |
| `fn var_shift_w`, `dst == rcx` path | scratch | 1 |
| `fn var_shift_w`, other path | `rcx` | 1 |

So a 16-byte slot covers every site. The encoder sees one instruction at a time
and knows nothing about the frame.

**Reads through `rsp` inside a window.** At least one site reads its own saved
value back through `rsp` while the push is live: `var_shift`'s non-`rcx` path,
when `value` is `rcx`, reloads the preserved `rcx` from `[rsp]` (`mov dst,
[rsp]`, comment "reload the preserved copy from [rsp]"; `var_shift_w` has the
same form). In the slot form that read must become `[rsp+slot]`. Phase 1 lists
every `rsp`-relative read inside a push window, at every site.

**Hand-built functions.** `_main` (`src/codegen/engine/function/entry.rs`), the
app-mode functions (`src/target/win_x86_64/app/mod.rs`), and the hand-built
trampolines (plan-169-A §2) do not pass through `finalize_frame`, so §3 step 1
never annotates them. Any pushing op they contain would keep its `push`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Call-free `push` windows on Windows | 8730 | `p169-windows.py` (plan-169-A Appendix) (plan-169-A census + brogue, dungeon, life) |
| `push`/`pop` instructions in the 26-fixture census | 16582 | `p169-bodyrsp.py` (plan-169-A Appendix) "push/pop" |
| Encoder `push`/`pop` sites | 9 | the grep above |
| Frameless functions that would need a frame to hold a slot | dungeon 23 of 555, brogue 3 of 249 (upper bound: only those that use a pushing instruction) | `p169-frames.py` (plan-169-A Appendix) "frameless" |

### Verified properties

- UNVERIFIED: which frameless functions use a pushing instruction (they gain a
  small frame). Phase 1 measures it.
- UNVERIFIED: which hand-built Windows functions use a pushing instruction.
  Phase 1 measures it.

## 3. Design Overview

1. `finalize_frame` (Win64) scans the body for instructions the encoder expands
   with a scratch push (a fixed list of ops, owned next to the encoder), and if
   any exist reserves a 16-byte scratch slot (two qwords) in the frame and
   annotates each such instruction with `scratch=<offset>`.
2. The encoder, when an instruction carries `scratch=`, saves and restores the
   temporary with `mov [rsp+offset], reg` / `mov reg, [rsp+offset]` instead of
   `push`/`pop`. Without the field (every non-Win64 target) it encodes as today.
3. A function that was frameless but uses such an instruction gets a minimal
   frame (it now moves `rsp` in a prologue, and A's tables describe it).
4. A hand-built Win64 function that uses such an instruction reserves the slot
   in its own frame and annotates the instruction itself, through the same
   helper `finalize_frame` uses.
5. **Guard.** On Win64, the encoder errors (naming the function) on a pushing
   instruction with no `scratch=` field. This completes the plan-169-A/B guard:
   after C, any body `rsp` change other than an epilogue is an error on Win64,
   so a function nobody annotated cannot reach the image. Without it, a missed
   hand-built function would keep its `push` and the census would be the only
   thing to notice.

**Correctness risk**: the slot must not alias a live slot (it is reserved, not
reused), the restore must come back to the same register, and every read of a
saved value through `[rsp]` inside a window (§2) must be redirected to the slot;
the encoder tests cover each of the 9 sites. Expected diffs: Windows `.ncode` of every function
using a pushing instruction (most functions with an overflow-checked multiply).

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Use `- [~]` for partial work, and mark moot tasks
> `- [x] ~~text~~ (moot: <evidence>)`. Fill each `Commit:` line when the phase
> lands. **An unticked box means NOT DONE.**

### Phase 1: The pushing-op list

- [ ] List the code ops whose encoding reaches each of the 9 sites (read each
      site's match arm); put the list beside the encoder as a function
      (`needs_scratch_slot(op)`).
- [ ] Measure which frameless Windows functions use one; record the count.
- [ ] Measure which hand-built Windows functions use one; record each with its
      source site.
- [ ] List every `rsp`-relative access inside a push window, per site (the
      `var_shift`/`var_shift_w` `[rsp]` reload is one); record it here.

Acceptance: the list covers every site.
  Check: a test that encodes each listed op without a slot on Win64 and finds a
  `push` (est. 2 min).
Commit: —

### Phase 2: Slots

- [ ] `vreg_frame.rs`: reserve the slot and annotate (Win64).
- [ ] `emitter.rs`: the 9 sites use the slot when annotated, including every
      in-window `rsp`-relative access from Phase 1.
- [ ] The hand-built functions from Phase 1 reserve and annotate their slot.
- [ ] `emitter.rs`: on Win64, a pushing instruction without `scratch=` is an
      error naming the function (§3 step 5).
- [ ] Tests: each site's slot form decodes to the same arithmetic (encoder tests
      comparing results on sample operands via the existing x86 test harness),
      including `var_shift`/`var_shift_w` with `value == rcx` (the `[rsp]`
      reload); and the guard fires on Win64 and not on linux-x86_64.

Acceptance: no Windows `push` window remains.
  Check: `p169-windows.py` (plan-169-A Appendix) over a fresh Windows census →
  `('push', 'no-call') 0` (est. 5 min).
Commit: —

### Phase 3: Proof and docs

- [ ] Box 206: twenty gdb attaches to `debug 3000 20000` (`--debug -O0`), each
      `bt` reaching `_main` with only `kernel32`/`ntdll` frames past it and no
      `??`. Record the count.
- [ ] Update `mfb man debug backtraces`: remove "Windows: only the innermost
      frames", show the full Windows chain, and make the Windows attach script
      print the full `bt` (not `bt 1`). Re-run
      `bash scripts/man-debug-examples.sh target/release/mfb windows` → all pass.
- [ ] `mfb spec linker windows-x86_64`: frames are fixed; encoder scratch uses a
      frame slot on Win64.
- [ ] Artifact gate: diffs only in Windows `.ncodesum` goldens; one objdump per
      fixture confirms the slot form; re-pin with proof.

Acceptance: 20 of 20 attaches reach `_main` with only system-DLL frames past
it.
  Check: the 206 loop (est. 5 min).
Commit: —

### Phase 4: plan-169 closeout (full suite, once)

- [ ] `rustup run 1.96.0 cargo fmt --all --check` → clean;
      `cargo clippy --all-targets` → exit 0.
- [ ] Byte-identity of non-Windows output: the plan-166-A identity loop
      restricted to `macos-aarch64 linux-aarch64 linux-x86_64 linux-riscv64`
      against a release build of plan-169's base commit → no diff.
- [ ] Full suite once: `cargo test --no-fail-fast > /tmp/plan169-run.log 2>&1;
      echo EXIT=$?` → `EXIT=0`, `grep -c '^failures:'` → 0 (est. ~85 min,
      measured for plan-166 on this host; the only check that covers every runtime
      test's executable).
- [ ] Archive `plan-169-C` (A and B are archived as they complete).

Acceptance: every check in this phase passes as stated.
Commit: —

## Validation Plan

- Tests: Phase 1–2 encoder and frame tests.
- Runtime proof: Phase 3's 20-attach loop on box 206.
- Doc sync: Phase 3 (`mfb man debug backtraces`, spec).
- Final gate: Phase 4.

## Open Decisions

- None beyond plan-169-A's.

## Corrections

(Filled in during execution.)

## Summary

Small in code (9 encoder sites and one reservation), broad in effect (most
Windows functions' `.ncode` changes). After it, `rsp` is fixed in every Windows
function body, so the unwind tables from A are correct at every instruction,
which is what lets the backtraces page drop its Windows caveat.
