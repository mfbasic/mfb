# bug-138 — G9 dead-code / nit cluster: dead FloatBinaryKernel::Pow machinery, stale vector comment, dead crypto self-move

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G9). Dead-code and docs nits,
batched per goal-02.

## 1. Dead `FloatBinaryKernel::Pow` machinery

`src/target/shared/code/builder_simd_float_math.rs:1359-1376` (Pow body), :937
(`emit_exp_body_lo`'s `lo` param), :975 (`emit_log_body`'s `keep_lo`) — all
reachable only via `FloatBinaryKernel::Pow`, which no caller constructs (scalar
pow → `emit_pow_scalar`, array pow → `lower_pow_array`; the array driver
debug-asserts atan2-only). The double-double `exp(y·log x)` SIMD pow path is
fully wired but unreachable and silently rots (it would also need the v24
zeroing hoist the assert warns about). Partial dup of bug-68 item (2)/(3):
bug-68's fix added the assert and error-set plumbing but left the dead kernel
body. Fix: delete the Pow kernel body and the now-unused `lo`/`keep_lo`
parameters.

## 2. Stale contradictory comment + dead self-move

- `src/target/shared/code/builder_vector_inline.rs:313-318` — comment says
  `distance` "is left to the FUNC" directly above the arm that inlines it.
  Comment-only fix.
- `src/target/shared/code/crypto.rs:60` — `move_register(x0, x0)` no-op emitted
  into every randomBytes helper. Drop the dead instruction.
