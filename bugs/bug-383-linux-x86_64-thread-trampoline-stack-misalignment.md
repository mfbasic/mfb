# bug-383: linux-x86_64 thread trampoline is 16-byte stack-misaligned (latent)

Status: OPEN (latent — no known crash today; a real SysV ABI violation)
Found: while fixing plan-47-H (the Windows twin of this bug, commit acfeeab6c).

## Claim

`lower_thread_trampoline` (`src/target/shared/code/runtime_helpers.rs`) subtracts a
16-byte-multiple frame (`FRAME_SIZE = 80`) and assumes it is entered with the stack
16-aligned to `sp % 16 == 0`. That assumption holds on aarch64 but NOT on x86-64 SysV,
so on **linux-x86_64** every call the trampoline makes (arena-init, the worker body, the
`pthread_*` shims) is issued with `sp % 16 == 8`, and each callee sees a 16-misaligned
stack — an AMD64 SysV ABI violation.

## Mechanism (why aarch64 is fine and x86-64 is not)

The pthread library calls the thread start-routine (our trampoline) via a normal call:

- **aarch64** `bl` writes the return address to LR and leaves `sp` unchanged, so the
  trampoline is entered at `sp % 16 == 0`; `sub 80` keeps `sp % 16 == 0`; its calls are
  correctly aligned. No bug.
- **x86-64** `call` *pushes* the 8-byte return address, so the trampoline is entered at
  `sp % 16 == 8` (the standard "callee entry" state); `sub 80` (80 ≡ 0 mod 16) leaves
  `sp % 16 == 8`; before each of its own calls `sp % 16 == 8` instead of the required
  `== 0`, so the callee it invokes sees `sp % 16 == 0` where the ABI guarantees `== 8`.

This is the identical mechanism as the program *entry* (`entry.rs`
`entry_stack_misaligned_on_entry` adds one `sub rsp, 8` for the same reason) and as the
Windows trampoline bug fixed in 47-H (folding 8 bytes into the frame on Windows).

## Why it does not crash today

Linux glibc/musl callees the worker reaches (memcpy uses `movdqu`, etc.) do not assert
16-alignment via aligned-SSE-on-a-stack-local. The Windows twin DID crash because a
Windows file API dispatches through ntdll `SbSelectProcedure`, which does
`movaps [rbp+0x170], xmm0` and #GPs on the misalignment. A linux-x86_64 worker that
calls a routine using aligned SSE/AVX on an rsp-relative local (some glibc/handwritten
SIMD, or `-mavx` codegen) would fault the same way. It is a latent correctness bug, not
a style nit.

## Fix

The 47-H fix in `lower_thread_trampoline` is `let win_realign = if family == Windows { 8 }
else { 0 }`. The correct general predicate is "x86-64 (SysV or Win64), where `call`
pushes 8" — i.e. the realignment should apply to **linux-x86_64** as well, not just
Windows. Extend the fixup to fold 8 bytes into the trampoline frame on every x86-64
target (gate on the arch/family, not on Windows alone). aarch64/riscv64 keep the 0
realign.

**Cost:** this changes the emitted bytes of the trampoline on linux-x86_64, so its
`.ncode`/`.mir` byte-identity goldens churn. That is expected and correct — re-baseline
only the trampoline-bearing linux-x86_64 goldens, with this bug cited as the proof.

## Repro (to confirm before fixing)

Build any threaded fixture for linux-x86_64, disassemble the `runtime.thread.trampoline`
symbol, and confirm the frame `sub rsp` is a 16-multiple with no compensating `sub rsp, 8`
— then check `rsp % 16` at any of its `call` sites is 8, not 0. Or force the fault: a
worker body compiled `-mavx` that spills a `__m256`/`__m128` to a stack local with
`vmovaps`/`movaps` should #GP.
