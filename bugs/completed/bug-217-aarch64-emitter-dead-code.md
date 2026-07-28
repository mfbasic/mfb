# bug-217: aarch64 emitter dead code — unreachable mov_imm zero-fallback and .2d shift-by-64 handling

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: dead-code

Status: Fixed (2026-07-15) — deleted emit_mov_imm's unreachable `if !emitted { movz #0 }` fallback (index 0 always emits the base movz, including for value == 0), and narrowed emit_v_shift_imm's sshr/ushr `.2d` guards from >64 to >63 to match the shared shift() operand parser (which rejects any amount >= 64), so no unreachable shift-by-64 case remains.

Two dead-code items in `src/arch/aarch64/encode/emitter.rs`:

- `emit_mov_imm` (`:598-600`): the `if !emitted { movz rd, #0 }` fallback is
  unreachable — index 0 of the fixed `[0,16,32,48]` loop always executes and sets
  `emitted = true` (the `value == 0` case already emits `movz #0` at index 0).
  Fix: delete the `if !emitted` block.
- `emit_v_shift_imm` (`:501-523`): the code accepts a `.2d` right-shift amount of
  64 (guards allow it), but the shared `shift()` operand parser
  (`operand.rs:95-103`) rejects any value `>= 64`, so the 64 case is unreachable
  and a legitimate `.2d` sshr/ushr by exactly 64 fails-loud at operand parse.
  Latent (no shipped source path emits a 64-bit-lane shift of 64; aarch64 facet
  of completed bug-16). Fix: drop the unreachable 64 handling, or widen the
  operand path if a shift-by-64 ever becomes reachable.
