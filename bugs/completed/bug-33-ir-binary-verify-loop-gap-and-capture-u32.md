# bug-33: Two LOW `ir/binary.rs` nits — `verify_package` skips `For`/`DoUntil` bodies, and `Capture.index` is silently truncated `usize`→`u32` on encode

Last updated: 2026-07-08
Effort: small (<1h)

Two small LOW-severity defects in `src/ir/binary.rs` (IR (de)serialization + the
structural `verify_package` walk). Batched (same file, both LOW, both
defense-in-depth/latent).

**(1) `verify_package`'s structural walk skips `For` and `DoUntil` bodies.**
`verify_ops` (`binary.rs:1337-1361`) recurses into `If`/`While`/`ForEach`/`Trap`/
`Match` bodies, but its single-body arm (`:1347`) enumerates only
`While|ForEach|Trap` — `IrOp::For` and `IrOp::DoUntil`, which also carry `body`
vecs, fall into `_ => {}` and are never recursed. So the structural empty-`Match`
invariant (`:1350-1355`) is not applied inside `For`/`DoUntil` loops. **Masked**
today: the semantic pass `verify/mod.rs::check_ops` (`:1096-1102`) does recurse into
`For` (`:1234`) and `DoUntil` (`:1249`), so an empty `Match` nested in those loops
is still rejected end-to-end — this is a defense-in-depth gap in `verify_package`,
not a soundness hole.

**(2) `Capture.index` (`usize`) is silently truncated to `u32` on encode.**
`encode_value` (`binary.rs:1013`) writes `put_u32(out, *index as u32)` while
`decode_value_body` (`:1181`) reads `r.u32()? as usize` — an asymmetric round-trip
for any `index > u32::MAX` (low 32 bits kept). Latent: no real path produces >4
billion captures.

The single correct behavior a fix produces: `verify_package` recurses into every
op that carries a body (including `For`/`DoUntil`), and `Capture.index`
serialization is range-checked (or documented as `u32`-bounded) so encode/decode
cannot silently disagree.

Severity LOW for both.

References:

- `src/ir/binary.rs:1337-1361` (`verify_ops`; single-body arm at `:1347` omits
  `For`/`DoUntil`), `:1350-1355` (empty-`Match` invariant).
- Masking pass: `src/ir/verify/mod.rs:1096-1102`, `:1234`, `:1249`.
- `src/ir/binary.rs:1013` (`encode_value`, `*index as u32`), `:1181`
  (`decode_value_body`, `u32 as usize`).
- Found during goal-01 review of `src/ir/binary.rs`.

## Failing Reproduction

(1) An empty `Match{cases:[]}` nested in a `For`/`DoUntil` body → not rejected by
`verify_package` alone (only the later semantic pass catches it).
(2) A `Capture{index}` with `index > u32::MAX` → encode writes the low 32 bits;
decode reads a different value.

- Observed: (1) `verify_package` does not recurse into the loop body; (2)
  asymmetric round-trip.
- Expected: (1) `verify_package` recurses and applies the invariant; (2) range-check
  or documented bound.

Contrast: `While`/`ForEach`/`Trap` bodies ARE recursed by `verify_package`.

## Root Cause

(1) `verify_ops` omits `For`/`DoUntil` from its body-recursion arm.
(2) `encode_value` narrows a `usize` to `u32` with an unchecked `as` cast.

## Goal

- `verify_package` recurses into `For`/`DoUntil` bodies.
- `Capture.index` encode is range-checked (error on `> u32::MAX`) or the IR type is
  documented/narrowed to `u32`.

### Non-goals (must NOT change)

- The semantic-pass behavior (already correct).
- The `u32` wire width (only add the guard).

## Blast Radius

- `verify_ops` (`binary.rs:1347`); `encode_value`/`decode_value_body` capture-index
  path.

## Fix Design

(1) Add `IrOp::For { body, .. } | IrOp::DoUntil { body, .. }` to the single-body arm
at `:1347`. (2) In `encode_value`, `u32::try_from(*index)` and error on overflow (or
change `Capture.index` to `u32` throughout if it is genuinely bounded).

## Phases

### Phase 1 — failing test + audit

- [ ] (1) A `verify_package` unit test with an empty `Match` inside a `For` body
      asserts rejection at the structural pass. (2) A capture-index round-trip test
      at the boundary.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Add `For`/`DoUntil` to `verify_ops`; range-check capture-index encode.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh` — IR (de)serialize goldens byte-identical.

## Validation Plan

- Regression test(s): the two tests above.
- Full suite: `scripts/test-accept.sh`.

## Summary

A missing loop-body case in the structural verifier (masked by the semantic pass)
and an unchecked `usize`→`u32` capture-index narrowing; both fixes are one-liners
that harden `ir/binary.rs`.
