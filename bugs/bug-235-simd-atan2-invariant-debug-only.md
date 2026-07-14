# bug-235: SIMD binary-kernel "atan2-only" Inf-mask invariant is enforced only by a debug_assert

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: footgun (latent)

Status: Open

`lower_simd_float_binary` (`src/target/shared/code/builder_simd_float_math.rs:1457`)
relies on a `debug_assert!` to enforce that the only binary float kernel is
non-Inf-raising (`v24` is zeroed inside the per-iteration body, not hoisted). The
assert is compiled out in release.

Trigger: currently unreachable — `FloatBinaryKernel` has only `Atan2`. If a
future Inf-raising binary kernel (e.g. `Pow`) is wired here, release builds would
silently reduce a never-zeroed/stale `v24` mask (spurious or missed
`ErrFloatInf`) with no diagnostic.

Fix: when a second binary kernel is added, hoist the `k.v24` zero out of
`emit_float_binary_body` into the driver (as the array unary path does) rather
than relying on the debug-only assert.
