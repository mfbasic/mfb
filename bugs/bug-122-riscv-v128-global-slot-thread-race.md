# bug-122 — riscv64 v128 slot region is a process-global → worker threads racing float kernels corrupt each other

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G6).
**Severity:** MED — silent wrong numeric results when ≥2 threads run v128 code
concurrently on riscv64.
**Class:** correctness (data race).

## Finding

`src/arch/riscv64/v128.rs:31-39` (`V128_SLOTS_SYMBOL = "_mfb_rt_v128_slots"`,
one global 128-slot region) and module doc :20-22 ("single-threaded /
non-reentrant"). Every scalarized v128 op on riscv stages its lanes in this one
global data object (addressing `global + slot*16 + h*8`, no TLS, no per-thread
base). The module doc addresses only reentrancy across calls, not concurrency:
two OS threads (`thread::start` workers) executing any v128-using code
simultaneously — transcendental `math::` kernels, `math::` array kernels,
`vector::` — interleave loads/stores to the same slots.

## Trigger

linux-riscv64 program where ≥2 worker threads concurrently evaluate e.g.
`math::sin`/`math::exp` (inline SIMD kernels) — lane values from one thread
bleed into the other's Horner evaluation → silently wrong, timing-dependent
results.

## Fix sketch

Make the slot region thread-local (per-thread base, e.g. off the `%thread`
block or a TLS slot), or push/pop the lanes on the stack instead of a global.

## Prior art

Distinct from bug-86 (its failing worker is integer-only, no v128 ops).
