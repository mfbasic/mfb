# bug-294: x86-64 `str_u16` dispatches into an always-erroring arm (plan-50-D x86 leg never implemented)

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: src/arch/x86_64/encode/tests.rs::str_u16_encodes_the_operand_size_prefixed_store

Commit 628faca7 (plan-50-D) added `"str_u16" => mem_store(instruction,
MemWidth::U16)` to the x86 emitter, claiming "the 0x66 operand-size prefix path
already exists, driven by ldr_u16" — but that claim is false twice: `ldr_u16` uses
`movzx` (0F B7), not a 0x66 path, and both `mem_store` U16 arms still
`return Err("x86 encode: str_u16 unsupported")`. aarch64 (STRH) and riscv64 (sh)
encode `str_u16`; x86 is the only backend that fails. It fails loudly (sizing also
errors), so no silent wrong code today — but it is a latent build break: `str_u16`
is emitted by `abi::store_u16`, currently `#[allow(dead_code)]` until plan-50-E
wires CInt16/CUInt16 struct-field marshaling, which will break linux-x86_64 the day
it lands.

The single correct behavior a fix produces: `str_u16` encodes a correct 16-bit store
on x86-64 (`0x66` operand-size prefix), matching aarch64/riscv64, with a byte-exact
test.

References:

- Commit 628faca7 (plan-50-D); the memory note "plan-50 LINK struct ABI … proven on
  all 7 combos" (getFormats exercised no u16 store).
- Found during goal-06 review of `src/arch/x86_64/encode/emitter.rs`.

## Failing Reproduction

Any x86 stream emitting `str_u16` → build error `x86 encode: str_u16 unsupported`
(from both sizing and emit). Latent because the only producer (`abi::store_u16`) is
dead-code-gated until plan-50-E.

- Observed: hard error on any `str_u16`.
- Expected: a 16-bit store is encoded.

## Root Cause

`src/arch/x86_64/encode/emitter.rs:615` (dispatch), `:1761` and `:1791-1795`
(`mem_store` `MemWidth::U16` arms return `Err`). The register-arm's "unreachable via
the public CodeInstruction path" comment is stale — the mnemonic exists and
`CodeOp::StrU16` is registered.

## Goal

- Implement both U16 `mem_store` arms:
  - register source: `0x66 [REX] 0x89 /r`
  - zero-token source: `0x66 [REX] 0xC7 /0 imm16(0)`
  with REX only when src/base ≥ 8, plus a byte-exact test; drop the stale comments.

### Non-goals (must NOT change)

- `ldr_u16` (movzx) or the other MemWidth arms.
- aarch64/riscv64 `str_u16`.

## Blast Radius

- The two x86 `mem_store` U16 arms — fixed here.
- `abi::store_u16` consumers (plan-50-E) — unblocked, not changed here.
- Stale test comments at `encode/tests.rs:814, 1819` — update.

## Fix Design

Encode the 0x66-prefixed `mov r/m16, r16` and `mov r/m16, imm16` forms with correct
REX/ModRM/SIB (reuse the existing width-parameterized helpers). Rejected
alternative: routing through `ldr_u16`'s movzx — that's a load, not a store.

## Phases

### Phase 1 — failing test
- [ ] Byte-exact `str_u16` test (currently errors).
### Phase 2 — the fix
- [ ] Implement both arms; drop stale comments.
### Phase 3 — validation
- [ ] Arch encode suite + artifact gate green.

## Validation Plan

- Regression: byte-exact encoder test for register and imm forms.
- Full suite: `scripts/artifact-gate.sh` + arch unit tests.
- Doc sync: none.

## Summary

A plan-50-D dispatch entry points at an unimplemented arm; implementing the two
0x66-prefixed store forms completes the x86 leg before plan-50-E makes it reachable.
Low risk, well-scoped.

## Resolution

Both `MemWidth::U16` arms of `mem_store` now encode instead of returning an error:

- register source: `0x66` operand-size prefix, then the same `0x89 /r` form the
  32-bit store uses, with REX only when `src`/`base` >= 8 (the prefix precedes REX);
- zero-token source: `0x66` + `0xC7 /0` with a 16-bit immediate zero, matching how
  the other widths handle `abi::ZERO`.

### The stale comments were disproved, not assumed

Three places asserted that `str_u16` "has no CodeOp mnemonic" and that the arm was
unreachable — one `coverage:off` block in the emitter and two test comments, one of
which had a whole test built around the claim. That claim is what let plan-50-D ship
an x86 leg that could only ever fail.

It was checked rather than argued: `ops.rs` maps `CodeOp::StrU16 <-> "str_u16"` in
both directions, and the emitter has dispatched `"str_u16" => mem_store(…, U16)`
since 628faca7. A probe encoding a `str_u16` instruction returned
`"x86 encode: str_u16 unsupported"` — the *arm's own* error, not the dispatcher's
`"unsupported op"` — which is direct evidence that dispatch reached it. Only then
were the comments replaced.

`str_u8_extended_and_u16_unsupported` was renamed to `..._encode` and now asserts
the encoding rather than the error.

The byte-exact test covers the plain form, REX.R (high source), REX.B (high base),
the rsp SIB escape, and the zero-token immediate form. Sizing needed no change:
x86's `instruction_size` is `encode_instruction(...).bytes_len()`, so it agrees with
the encoder by construction.

Nothing reachable changed — `abi::store_u16` is still dead-code-gated until
plan-50-E — which the artifact gate confirms: 1169 goldens across 989 tests, 0
diffs. The x86 leg is now ready for plan-50-E instead of breaking linux-x86_64 the
day it lands.
