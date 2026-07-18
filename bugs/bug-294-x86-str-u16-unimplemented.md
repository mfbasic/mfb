# bug-294: x86-64 `str_u16` dispatches into an always-erroring arm (plan-50-D x86 leg never implemented)

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: src/arch/x86_64/encode/tests.rs (new) — `str_u16` encodes a 16-bit store byte-exactly

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
