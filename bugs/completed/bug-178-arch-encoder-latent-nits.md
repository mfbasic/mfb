# bug-178 — arch encoder latent nits (aarch64 x18/x29 unencodable while token-mapped, `mov dst,sp`→`xzr`, x86 LZCNT baseline comment)

Last updated: 2026-07-12
Severity: LOW (batch) — all latent (no current lowering emits the trigger).
Class: Correctness / Footgun.
Status: FIXED (2026-07-13; goal: resolve bugs 170-180; full acceptance suite green)

## Findings

**A. aarch64 `reg()` cannot encode `x18`/`x29`, but the ABI token map realizes
`%scratch9` → `x18`.** `src/arch/aarch64/encode/operand.rs:17-58`.
`abi::realize_abi_token("%scratch9") => "x18"` (`src/target/shared/abi.rs:298`),
but `reg()` jumps from `x17` straight to `x19` with no `x18`/`x29` arm, so it
would hit the `other => Err("unknown AArch64 register 'x18'")` path. A census of
`SCRATCH[N]` uses shows index 9 is never emitted today (indices 0-8, 10-18), so
it fails codegen loudly rather than miscompiling — but the token map and encoder
disagree, and `x18` is the reserved platform register on Darwin. Fix: add
`x18`/`x29` arms, or remove/repoint the `%scratch9`→`x18` entry.

**B. aarch64 `mov dst, sp` silently encodes as `mov dst, xzr` (reads 0, not SP).**
`src/arch/aarch64/encode/emitter.rs:569-571`. `reg()` maps `"sp"`/`"raw_sp"` to
31; `emit_mov` encodes `mov` as `ORR rd, xzr, rm`, where register 31 is XZR, so
`abi::move_register(dst, "sp")` would emit `dst = 0` instead of the stack
pointer. No current caller does `move_register(_, sp)` (reading SP goes through
`add dst, sp, #0`), so latent. Fix: in `emit_mov`, reject/redirect a src/dst of
31 (SP) through the `add dst, sp, #0` immediate form.

**C. x86 `clz` emits LZCNT but the baseline comment claims x86-64-v2.**
`src/arch/x86_64/encode/emitter.rs:305-318`. `clz` emits `F3 0F BD` (LZCNT); the
comment claims LZCNT is present on every x86-64-v2 CPU, but LZCNT is an ABM/BMI
feature (effectively v3). On a CPU with SSE4.x but no LZCNT the `F3` prefix is
ignored and the bytes execute as `BSR` (bit index, undefined for zero) —
silently wrong vs the aarch64 `clz` (64 on zero). The backend already emits FMA3
(v3) and SSE4.1 `roundsd` unconditionally, so it effectively requires ~v3 anyway;
the risk is the stale "v2" claim. Fix: correct the comment to state a v3 baseline
(FMA3/LZCNT), or gate/emulate LZCNT if v2 support is a goal.
