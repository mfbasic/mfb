# bug-153 — untrusted `.mfp` deep type-chain decode overflows the stack (no recursion depth cap)

Last updated: 2026-07-12
Effort: small (<1h)
Severity: MEDIUM
Class: Security (untrusted-input DoS)

Status: Open
Regression Test: _(none yet)_

Decoding the TYPE_TABLE of a `.mfp` package walks composite type references
recursively. The PKG-04 cycle guard rejects a type id that references *itself*
(directly or via a cycle), but there is **no depth limit**, so a crafted package
whose type table is a long *linear* chain of distinct composite types
(id 10 → List OF id 11 → List OF id 12 → … → primitive) recurses one native
stack frame per link. Each TYPE_TABLE entry is ~24 bytes (20-byte header +
4-byte List payload), so a package of a few hundred KB to a few MB yields
hundreds of thousands of frames and overflows the stack (SIGSEGV/abort) before
any signature is trusted. The correct behavior is to reject an over-deep type
graph with a decode error, mirroring the existing PKG depth caps.

References:

- `audit-1 package-decode` (PKG-01..07): added depth caps, cycle guards,
  bounded alloc, dup-section reject — this is the gap they missed (depth cap was
  added to *section* nesting, not the type-graph walk).
- goal-03 review.

## Failing Reproduction

Craft a `.mfp` whose TYPE_TABLE holds a chain of N (e.g. 500_000) distinct
`kind == 4` (List) entries, each whose 4-byte payload points at the next id, the
last pointing at a primitive. Feed it to any untrusted-package entry point, e.g.
`mfb pkg validate <file>` / `read_package_info` / `read_binary_repr_package`.

- Observed: stack overflow (SIGSEGV / `thread ... has overflowed its stack`).
- Expected: a graceful `"type graph too deep"` decode error.

Contrast: a *cyclic* chain (id N → id N) correctly errors `"cyclic type id N"`
via the `in_progress` guard — only the acyclic-but-deep case is unguarded.

## Root Cause

`src/binary_repr/reader.rs:650` `decode_type_name` → `:677`
`decode_type_name_body` → `:772` `read_payload_type` → back to
`decode_type_name`, with no depth counter. `in_progress` (`:667`) only rejects
re-entry to an id already on the stack (cycles), not a deep chain of distinct
ids. `AbiSerializer::serialize_type` (`src/binary_repr/reader.rs:1345`) has the
identical shape: its `type_refs` set blocks only cycles and it recurses through
list/map/result/thread/function element types unboundedly. Both are reachable
from untrusted decode: `read_binary_repr_package` calls `type_entry_names`
(`:355` → `decode_type_name`) and `validate_abi_index` (`:414` →
`function_sig_hash`/`type_sig_hash` → `serialize_type`) before any signature is
verified.

## Goal

- A crafted deep-but-acyclic type graph decodes to a bounded error, not a crash.

### Non-goals (must NOT change)

- The cycle-guard behavior and its error text.
- The decoded type-name strings for any legitimate (shallow) package.

## Blast Radius

- `decode_type_name` / `decode_type_name_body` / `read_payload_type` /
  `decode_function_type` (`reader.rs:650-782`) — fixed by this bug.
- `AbiSerializer::serialize_type` (`reader.rs:1345`) — same hazard, in scope.

## Fix Design

Thread a depth counter (or an explicit work-stack) through
`decode_type_name`/`decode_type_name_body`/`read_payload_type`/
`decode_function_type` and `AbiSerializer::serialize_type`, returning a decode
error past a fixed cap (reuse the PKG depth-cap constant / pick a generous bound
like 256 — real type graphs are shallow). Add a regression fixture with a deep
synthetic type table.

## Summary

Low-effort hardening of the untrusted-`.mfp` decode boundary: add a depth cap to
the two type-graph recursions that today have only a cycle guard.
