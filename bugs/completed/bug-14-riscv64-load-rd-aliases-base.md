# bug-14: rv64 out-of-range integer load materializes its address into `rd`, corrupting `base` when `rd == base`

Last updated: 2026-07-08
Effort: small (<1h)

`src/arch/riscv64/encode/emitter.rs::emit_load` handles an integer load with a
displacement `> 2047` by materializing the address **into the destination
register `rd` itself**:

```
self.emit_li(rd, offset)?;              // rd = offset
self.emit_r(OP, 0b000, 0, rd, base, rd)?; // add rd, base, rd
self.emit_word(i_type(0, rd, funct3, rd, LOAD)) // ld rd, 0(rd)
```

If the register allocator has placed `rd` and `base` in the **same** physical
register (a normal linear-scan outcome when `base`'s live range ends at the load —
e.g. `p = p.someFarField`), the first `emit_li(rd, offset)` destroys `base` before
the `add`, so the `add` computes `offset + offset` and the load reads from a
garbage address `2 * offset` → SIGSEGV or silent wrong data.

The three sibling memory emitters — `emit_store`, `emit_load_fp`, `emit_store_fp`
(`emitter.rs:511-536`) — all stage the address in the reserved scratch `T0`, so
they are immune. Only integer `emit_load` uses `rd`, and its comment reasons only
about avoiding `t0` (to protect live v128 lanes), not about `rd` aliasing `base`.

The single correct behavior a fix produces: an out-of-range integer load computes
the correct effective address and loads the correct value **regardless of whether
`rd == base`**.

Severity MEDIUM: silent wrong value / crash, but reachable only when the offset
exceeds 2047 **and** the allocator coalesces `dst == base`. Small records/frames
never reach the `> 2047` path, so it escapes ordinary tests.

References:

- `src/arch/riscv64/encode/emitter.rs:499-509` (`emit_load`, the `offset > 2047`
  branch that materializes into `rd`), `:511-536` (the three sibling emitters that
  stage the address in `T0` and are immune).
- `.ai/compiler.md` — register-lifetime conventions.
- Found during goal-01 review of `src/arch/riscv64/encode/**`.

## Failing Reproduction

No end-to-end `.mfb` was constructed (needs a record/pointer field at byte offset
`> 2047`, i.e. ~256+ 8-byte fields, plus the allocator assigning `dst == base`).
The defect is demonstrable at the emitter: `emit_load(funct3=ld, rd=x10, base=x10,
offset=4096)` emits `li x10,4096; add x10,x10,x10; ld x10,0(x10)` — it loads from
`8192` instead of `base+4096`.

- Observed: address = `2 * offset` (base lost) when `rd == base`.
- Expected: address = `base + offset` regardless of register aliasing.

Contrast cases correct today:

- `offset <= 2047`: single `ld rd, off(base)` reads `base` and writes `rd`
  atomically — safe even when `rd == base`.
- Spill reloads (`emit_reload`): `base = sp`, `dst != sp` — never aliases.
- `emit_store`/`emit_load_fp`/`emit_store_fp`: address staged in `T0`, and their
  value/base are never `T0` — safe.

## Root Cause

`emit_load` (`emitter.rs:503-508`) deliberately uses `rd` as the address scratch
"never `t0`" to protect live v128 lanes, but overlooks that `rd` can be the same
register as `base`. The `emit_li(rd, …); add rd, base, rd` sequence is only
correct when `rd != base`.

## Goal

- `emit_load` produces `base + offset` for every `offset` and every register
  assignment, including `rd == base`, without clobbering a live v128 scratch.

### Non-goals (must NOT change)

- Do not change the `offset <= 2047` single-instruction path.
- Do not reintroduce use of `t0` if that would corrupt live v128 lanes — see the
  fix note below (genuine large-offset program loads are not interleaved inside a
  scalarized v128 lane sequence).

## Blast Radius

- `emit_load` only — the other three memory emitters already stage via `T0`.

## Fix Design

Stage the address in `T0` like the three sibling emitters
(`emit_li(T0, offset); add T0, base, T0; ld rd, 0(T0)`). The v128 concern that
motivated using `rd` does not apply: scalarized v128 lane sequences use `T2`-based
capped offsets and `LdrQ`/`StrQ` route through `T1`; a genuine large-offset
program load is not emitted mid-lane. If preserving `t0` is deemed mandatory,
alternatively guard `if rd == base` and fall back to a different scratch.

## Phases

### Phase 1 — failing test + audit

- [ ] Add an rv64 encoder test: `emit_load` with `rd == base` and `offset = 4096`
      produces a sequence that computes `base + offset` (assert the emitted words
      do not alias-destroy base). Confirm it fails today.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Route the `> 2047` integer-load address through `T0` (or guard `rd == base`).

### Phase 3 — validation

- [ ] `scripts/artifact-gate.sh` (codegen). Confirm goldens only move where a
      `> 2047` load with `rd == base` was previously mis-emitted (likely none
      today → byte-identical) and rv64 runtime validation still byte-matches
      native aarch64.

## Validation Plan

- Regression test(s): the rv64 `emit_load` aliasing test.
- Runtime proof: rv64 hardware run (ssh Alpine riscv64) of a program with a
  large-offset field load, byte-identical to aarch64.
- Full suite: `scripts/artifact-gate.sh` + rv64 validation.

## Summary

The risk is confirming the `T0` staging does not disturb v128 lanes; the fix
makes integer `emit_load` match its three already-safe sibling emitters.
