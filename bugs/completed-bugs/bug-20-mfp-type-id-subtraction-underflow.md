# bug-20: Untrusted `.mfp` type id 0 or 9 underflows `id - FIRST_TABLE_TYPE_ID` in `serialize_type` → debug/test decoder panic

Last updated: 2026-07-08
Effort: small (<1h)

`src/binary_repr/reader.rs::AbiSerializer::serialize_type` (`:1295-1313`) resolves
a type id to a `TypeTable` entry via
`self.types.entries.get((id - FIRST_TABLE_TYPE_ID) as usize)` (`:1312`), an
**unchecked u32 subtraction**. Primitive ids (1–8, plus the large `0xffff_*`
handle/term sentinels) return early at `:1296`; cached ids return at `:1303`. But
ids **0** and **9** are neither primitives nor `>= FIRST_TABLE_TYPE_ID` (which is
10, `mod.rs:65`), so they fall through to the subtraction. `0u32 - 10` / `9u32 -
10` **underflow**. Under overflow-checks (Cargo debug/test default — the crate
sets no overriding `[profile]`) this **panics** `attempt to subtract with
overflow`, aborting the decoder on attacker-controlled input.

This is reached decoding an untrusted `.mfp`: `read_package_*` →
`validate_abi_index` → `function_sig_hash` → `serialize_type(id)` for any exported
function whose `return_type` or a `param.type_id` is 0 or 9 (9 is the freed
`TerminalSize` slot). In release (overflow-checks off) `9u32 - 10` wraps to a huge
`usize`, `.get` returns `None`, and the code yields a clean `"unknown type id"`
error — so the panic is **debug/test-only**.

The single correct behavior a fix produces: a sub-primitive / out-of-range type id
yields the same clean `"unknown type id"` error on **all** build profiles, never a
panic.

Severity LOW: a decoder DoS panic limited to overflow-checked (debug/test) builds;
release degrades gracefully. Filed because it is reachable from the untrusted
`.mfp` trust boundary and the sibling decode path already handles the same ids
cleanly.

References:

- `src/binary_repr/reader.rs:1295-1313` (`serialize_type`) — `:1312` (unchecked
  `id - FIRST_TABLE_TYPE_ID`), `:1296` (primitive early-return, 1–8 + sentinels),
  `:1303` (ref cache).
- `src/binary_repr/mod.rs:65` (`FIRST_TABLE_TYPE_ID = 10`), `:47-64` (primitive id
  values 1–8 and `0xffff_*`).
- Sibling that handles it cleanly: `decode_type_name`/`decode_type_name_body`
  (`src/binary_repr/util.rs:~681`) uses `raw.get(&id)` → clean "unknown type id".
- Trust-boundary context: audit-1 PKG-02 (decoded IR not fully re-validated).
- Found during goal-01 review of `src/binary_repr/**`.

## Failing Reproduction

Craft a `.mfp` whose FUNCTION_TABLE gives an exported function a `return_type` (or
`param.type_id`) of `9` (or `0`), then decode it in a debug/test build:

- Observed: `thread panicked: attempt to subtract with overflow` at
  `reader.rs:1312` during `validate_abi_index`.
- Expected: `Err("unknown type id 9")` — the same result the release build and the
  `decode_type_name` path already produce.

Contrast cases correct today:

- A primitive id (1–8) returns at `:1296`.
- An `id >= 10` that is out of range subtracts to a valid large `usize`; `.get`
  returns `None` → clean error.
- Release builds (overflow-checks off): 0/9 wrap → `.get` None → clean error.

## Root Cause

`serialize_type` assumes any id reaching `:1312` is `>= FIRST_TABLE_TYPE_ID`, but
the primitive filter (`:1296`) only excludes 1–8 and the `0xffff_*` sentinels, not
0 or 9. The raw `id - FIRST_TABLE_TYPE_ID` therefore underflows for those two ids.

## Goal

- `serialize_type(id)` for any `id < FIRST_TABLE_TYPE_ID` that is not a primitive
  returns a clean `"unknown type id"` error on every profile.

### Non-goals (must NOT change)

- Primitive-id and in-range table-id behavior (correct today).

## Blast Radius

- `serialize_type` (`reader.rs:1312`) — fixed by this bug. A sweep for other
  `- FIRST_TABLE_TYPE_ID` / raw-id subtractions on the decode path should confirm
  none share the pattern.

## Fix Design

Replace `(id - FIRST_TABLE_TYPE_ID) as usize` with
`id.checked_sub(FIRST_TABLE_TYPE_ID).and_then(|i| self.types.entries.get(i as
usize)).ok_or_else(|| format!("unknown type id {id}"))?` so sub-primitive ids
produce the clean error uniformly.

## Phases

### Phase 1 — failing test + audit

- [ ] Add a `reader.rs` test decoding a package with an export type id of 9 (and
      0); assert `Err("unknown type id …")` and no panic. Confirm it panics today
      under `cargo test` (overflow-checks on).
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] `checked_sub` at `reader.rs:1312`.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh`; the new decode-rejection test passes on debug.

## Validation Plan

- Regression test(s): the crafted-package decode test above.
- Runtime proof: decode the crafted `.mfp` and observe the clean error.
- Full suite: `scripts/test-accept.sh`.

## Summary

A one-line unchecked u32 subtraction panics the decoder on two specific untrusted
type ids in debug/test builds; `checked_sub` makes all profiles agree on the clean
error.
