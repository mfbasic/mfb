# bug-408: x86-64 Linux thread-trampoline override double-applies the SysV realign — glibc worker frame is 88 bytes (bug-385 proved 88 SIGSEGVs), musl is 96

Last updated: 2026-07-28
Effort: small (<1h) code change; requires box validation on 2228 (glibc) + 2227 (musl)
Severity: HIGH
Class: Memory-safety / Correctness (stack-ABI misalignment; regression of bug-385)

Status: Open
Regression Test: tests/rt-behavior/threads/* run on a real glibc AND musl x86-64
box (the frame size is invisible to macOS-host acceptance).

The shared thread-worker trampoline `lower_thread_trampoline`
(`src/target/shared/code/runtime_helpers.rs:955`) was fixed by bug-385 to apply the
SysV realign **per-libc, flavor-gated**:

```rust
const FRAME_SIZE: usize = 80;
let needs_realign = platform.arch() == "x86_64"
    && (platform.is_windows() || platform.libc() == Some(Libc::Musl));
let x86_realign = if needs_realign { 8 } else { 0 };
let frame = FRAME_SIZE + x86_realign;   // glibc x86-64 → 80; musl/win x86-64 → 88
```

bug-385 (with box proofs on 2228) established that glibc x86-64 needs an **80-byte**
frame and that an **88-byte** frame SIGSEGVs on glibc, while musl needs **88**.

But `LinuxArch::emit_thread_trampoline` in `src/target/linux_x86_64/code.rs:196`
*overrides* the default: it calls the shared (already-realigned) trampoline and then
inserts an **unconditional** second `sub_sp(8)` (`code.rs:225`), popped before each
`Ret`. The two realign mechanisms compound:

- glibc x86-64: shared 80 + override 8 = **88** — the exact frame bug-385 proved
  SIGSEGVs on glibc.
- musl x86-64: shared 88 + override 8 = **96** — also mis-aligned (entry sp%16==8,
  −96 keeps %8, so callees enter %0 and fault on the first `movaps` to a stack local).

aarch64/riscv64 have no such override (they use the default, which just calls the
shared trampoline), so only x86-64 is affected.

### Confirmed in the emitted HEAD binary (verified 2026-07-28)

Built `tests/rt-behavior/threads/thread-bounded-queues --target linux-x86_64` with
`target/debug/mfb` and byte-scanned the trampoline (`_thread_trampoline`) at file
offset `0x4cfba`:

- `…-glibc.out`: `48 81 EC 50 00 00 00` (`sub rsp,80`) **immediately followed by**
  `48 81 EC 08 00 00 00` (`sub rsp,8`) — compound found → 88-byte frame.
- `…-musl.out`: `48 81 EC 58 00 00 00` (`sub rsp,88`) + `48 81 EC 08 00 00 00`
  (`sub rsp,8`) — 96-byte frame.

Since HEAD emits the 88-byte glibc frame bug-385 proved fatal, **threaded x86-64
glibc programs SIGSEGV** (the first SSE-spilling callee in the worker call tree —
`pthread_create`/`fstatat`/… — faults on a `movaps [rsp+K]`).

The override (`code.rs:225`, commit 10b5d0fb4a, Jul 6) *predates* bug-383
(48b030d6a) / bug-385 (16cc93c93, Jul 25), which only edited
`runtime_helpers.rs`. When bug-385 moved the realign into the shared, flavor-gated
path, this override was left behind and now compounds. **Open question the fix must
resolve:** bug-385's box proof on 2228 reported the 80-byte frame runs — reconcile
that with HEAD emitting 88 (e.g. the proof built a tree with the override removed,
or exercised a non-thread path). The fix must be re-validated on a real box.

References:

- `src/target/linux_x86_64/code.rs:196` (`emit_thread_trampoline` override), `:225`
  (the unconditional `subtract_stack(8)`).
- `src/target/shared/code/runtime_helpers.rs:970-1014` (bug-385 flavor-gated realign).
- bug-383 / bug-385 (`bugs/completed/`); memory note
  `glibc-musl-thread-entry-alignment`. Found during goal-07.

## Failing Reproduction

```
target/debug/mfb build tests/rt-behavior/threads/thread-bounded-queues --target linux-x86_64
# byte-scan the -glibc.out trampoline (offset 0x4cfba):
#   sub rsp,80 (48 81 EC 50 00 00 00) immediately followed by sub rsp,8 (48 81 EC 08 00 00 00)
```

- Observed (HEAD, verified): glibc frame = 88 bytes; musl = 96 bytes.
- Expected (bug-385): glibc = 80, musl = 88.
- Runtime (per bug-385's box proof, not re-run here — needs an x86-64 Linux box):
  an 88-byte glibc worker frame SIGSEGVs.

Contrast: aarch64/riscv64 (no override) emit the correct shared frame.

## Root Cause

`linux_x86_64/code.rs`'s `emit_thread_trampoline` adds an unconditional `sub_sp(8)`
on top of the shared trampoline, which since bug-385 already applies the correct
per-libc realign. The two stack biases compound.

## Goal

- The x86-64 worker trampoline frame is exactly the bug-385 size: 80 (glibc) /
  88 (musl/windows) — no second, unconditional realign.

### Non-goals (must NOT change)

- The shared `lower_thread_trampoline` flavor gating (bug-385) is correct — do not
  touch it. aarch64/riscv64 trampolines must be unaffected.

## Blast Radius

- `src/target/linux_x86_64/code.rs:196-238` — the override. Simplest fix: **delete
  the override entirely** so x86-64 uses the default `LinuxArch::emit_thread_trampoline`
  (which just returns the shared, flavor-gated trampoline), matching aarch64/riscv64.
  Then box-validate threaded fixtures on 2228 (glibc) and 2227 (musl).
