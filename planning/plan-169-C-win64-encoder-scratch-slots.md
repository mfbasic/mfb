# plan-169-C: Win64 encoder scratch in frame slots, and plan-169 closeout

Last updated: 2026-10-03
Effort: large (3h–1d, including the final suite)
Depends on: plan-169-B

The x86-64 encoder expands several single instructions into short sequences that
need a temporary register, and it preserves that register with `push`/`pop`:
`mul` for overflow checks (`push rax; push rdx; …; pop rdx; pop rax`), the
ties-away float conversion, variable shifts into `rcx`, and division. Each
`push` moves `rsp` for a few instructions. A debugger or profiler stop inside one
of those windows, or a divide fault raised there, unwinds frame `#0` with the
wrong `rsp` and loses the chain. In hot arithmetic (every overflow-checked
multiply) these windows are common.

This sub-plan makes the encoder save its scratch to a **reserved slot in the
frame** on Win64 instead, so `rsp` never moves in a body. Then it closes plan-169:
documentation and the full suite. The outcome: on box 206, twenty consecutive
gdb attaches to a busy program all give backtraces that end at `_main`.

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
  each give a backtrace ending at `_main`.
- The full suite passes once.

### Non-goals (explicit constraints)

- linux-x86_64 keeps its encoding (SysV has no unwind-table requirement in this
  plan): the slot form is Win64 only. linux-x86_64 `.ncode` unchanged.
- No change to arithmetic results or error behavior.

## 2. Current State

Encoder sites that `push`/`pop`, all in `src/arch/x86_64/encode/emitter.rs`
(`grep -n "push rax\|push rdx\|enc_push_reg\|0x50 + \|push rcx" …`): lines 451–452,
542–545 (`push rax; push rdx` around `mul`), 951 (`fcvtas_x_from_d`), 1299 and
1313 (`push rax; push rdx`, division forms), 2009 and 2086 (`push scratch`,
variable shift into `rcx`), 2029 and 2104 (`push rcx`). The encoder sees one
instruction at a time and knows nothing about the frame.

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

**Correctness risk**: the slot must not alias a live slot (it is reserved, not
reused) and the restore must come back to the same register; the encoder tests
cover each of the 9 sites. Expected diffs: Windows `.ncode` of every function
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

Acceptance: the list covers every site.
  Check: a test that encodes each listed op without a slot on Win64 and finds a
  `push` (est. 2 min).
Commit: —

### Phase 2: Slots

- [ ] `vreg_frame.rs`: reserve the slot and annotate (Win64).
- [ ] `emitter.rs`: the 9 sites use the slot when annotated.
- [ ] Tests: each site's slot form decodes to the same arithmetic (encoder tests
      comparing results on sample operands via the existing x86 test harness).

Acceptance: no Windows `push` window remains.
  Check: `p169-windows.py` (plan-169-A Appendix) over a fresh Windows census →
  `('push', 'no-call') 0` (est. 5 min).
Commit: —

### Phase 3: Proof and docs

- [ ] Box 206: twenty gdb attaches to `debug 3000 20000` (`--debug -O0`), each
      `bt` ending at `_main`. Record the count.
- [ ] Update `mfb man debug backtraces`: remove "Windows: only the innermost
      frames", show the full Windows chain, and make the Windows attach script
      print the full `bt` (not `bt 1`). Re-run
      `bash scripts/man-debug-examples.sh target/release/mfb windows` → all pass.
- [ ] `mfb spec linker windows-x86_64`: frames are fixed; encoder scratch uses a
      frame slot on Win64.
- [ ] Artifact gate: diffs only in Windows `.ncodesum` goldens; one objdump per
      fixture confirms the slot form; re-pin with proof.

Acceptance: 20 of 20 attaches end at `_main`.
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
