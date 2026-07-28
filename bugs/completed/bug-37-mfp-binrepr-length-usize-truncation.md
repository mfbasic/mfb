# bug-37: `.mfp` `binaryRepresentationLength` (u64) is narrowed to `usize` with `as`, silently truncating on 32-bit targets

Last updated: 2026-07-08
Effort: small (<1h)

`src/manifest/package.rs:158` reads the `.mfp` body length as
`let binary_repr_length = read_u64(&bytes, offset)? as usize;` — a silent `as`
narrowing. On a 32-bit build, a malformed/hostile `.mfp` declaring a
`binaryRepresentationLength` with high bits set (e.g. `0x1_0000_0000`) truncates to
a small `usize`. The subsequent `offset.checked_add(binary_repr_length)` and the
trailing `offset != bytes.len()` structural check are then computed against the
**truncated** value, so a length field that does not describe the real body could
pass the structural check (defeating a validation the surrounding `checked_add`
reads are careful to enforce).

The single correct behavior a fix produces: a `binaryRepresentationLength` that
does not fit `usize` is rejected with a clean error, never truncated.

Severity LOW / **latent / defense-in-depth**: `usize` is 64 bits on all of the
project's runtime-validated (64-bit) platforms, so no truncation occurs there and
the exact `offset != bytes.len()` check fully validates the length. Only a 32-bit
target is exposed. Filed because it is an unchecked narrowing on the untrusted
`.mfp` decode path.

References:

- `src/manifest/package.rs:158` (`read_u64(...)? as usize`), `:159-161` and the
  later `offset != bytes.len()` trailing check that consumes it.
- Contrast: the signature-length path uses `read_u32 as usize` (safe on ≥32-bit);
  only the u64 body length can exceed `usize::MAX` on 32-bit.
- Trust-boundary context: audit-1 PKG-02, bug-20 (untrusted `.mfp` decode).
- Found during goal-01 review of `src/manifest/**`.

## Failing Reproduction

On a 32-bit build, a `.mfp` with `binaryRepresentationLength = 0x1_0000_0000`:

- Observed: truncates to `0`; the structural length check is computed against `0`
  and may pass despite the field not describing the real body.
- Expected: `Err("invalid .mfp binary representation length")`.

Contrast: on 64-bit targets (all supported platforms) `usize == u64`, no truncation,
and the `offset != bytes.len()` check fully validates.

## Root Cause

`read_u64(...) as usize` truncates rather than using a checked conversion.

## Goal

- A `binaryRepresentationLength` exceeding `usize::MAX` is rejected with a clean
  error on any target.

### Non-goals (must NOT change)

- 64-bit behavior (already correct).

## Blast Radius

- `package.rs:158` (and any sibling `read_u64(...) as usize` on the decode path —
  sweep for others).

## Fix Design

Replace with `usize::try_from(read_u64(&bytes, offset)?).map_err(|_| "invalid .mfp
binary representation length".to_string())?`.

## Phases

### Phase 1 — failing test + audit

- [ ] (32-bit-conditional) test that an oversized length is rejected; sweep for
      other `read_u64(...) as usize` decode sites.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] `usize::try_from` at `package.rs:158` (and any sibling site).

### Phase 3 — validation

- [ ] `scripts/test-accept.sh` — 64-bit behavior byte-identical.

## Validation Plan

- Regression test(s): the oversized-length rejection (or a `try_from` unit test).
- Full suite: `scripts/test-accept.sh`.

## Summary

An unchecked u64→usize narrowing on the untrusted `.mfp` decode path; `usize::try_from`
makes it reject rather than truncate. Latent on 64-bit; defense-in-depth for 32-bit.
