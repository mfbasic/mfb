# bug-378: NFC compose result allocation skips the checked-size header add

Last updated: 2026-07-23
Effort: small (<1h)
Severity: LOW
Class: Footgun

Status: Open
Regression Test: none required (defense-consistency change; no observable
behavior delta). Optionally assert the emitted instruction routes through the
checked helper via a codegen snapshot.

The Unicode-normalization (NFC) slow path computes the composed form's UTF-8 byte
length `output_len` and then allocates its result String with a **plain**
`add_immediate(return_register, output_len, 9)` for the 8-byte header + NUL,
where every sibling string builder (case-map, `graphemes`, `split`, `join`,
`replace`) routes the identical `+9` through the 64-bit-wrap-checked
`emit_checked_size_add_immediate`. This is not currently exploitable —
`output_len` is the summed UTF-8 width of the composed codepoints, bounded by
~4× the composed count, itself bounded by the (arena-bounded) input length, so
it cannot approach a `2^64` wrap — but it is the one string-allocation site where
the size-overflow guard the codebase applies everywhere else is absent. Making it
uniform removes a latent footgun and keeps the "every string allocation size is
wrap-checked" invariant total (so a future change that widens `output_len`'s
provenance can't silently reintroduce a heap-overflow-on-wrap).

The single correct behavior a fix produces: the NFC result allocation computes
its block size through the same checked-add helper as its siblings, trapping a
`2^64` wrap deterministically instead of wrapping to a small allocation.

References:

- Sibling correct pattern: case-map result alloc, `builder_strings_builtins.rs:557`
  (uses `emit_checked_size_add_immediate`).
- Found during the 2026-07-23 runtime security audit (string sweep).

## Failing Reproduction

There is no runtime-observable failure today (the wrap is unreachable). The
"defect" is a static inconsistency:

```
src/target/shared/code/builder_strings_builtins.rs:1049
    self.emit(abi::add_immediate(abi::return_register(), &scratch24, 9));
```

- Observed: plain unchecked add of the `+9` header/NUL onto the composed byte
  length before `emit_arena_alloc_call`.
- Expected: the same value routed through `emit_checked_size_add_immediate`
  (matching `:557` and the other string builders), so an (unreachable-today)
  overflow traps rather than under-allocating.

## Root Cause

`src/target/shared/code/builder_strings_builtins.rs:1043-1051` — the NFC compose
path stores `output_len` (sum of `emit_utf8_encoded_width` over the composed
codepoints) and then adds the literal `9` with a raw `add_immediate`. The
allocation-size wrap guard used at all sibling sites was simply not applied here.

## Goal

- The NFC result allocation size is computed with the checked-size add helper;
  no behavior change for any real input.

### Non-goals (must NOT change)

- The composed output bytes, the length math, or the NFC result contents.
- Do not alter the sibling sites (already correct).

## Blast Radius

- `builder_strings_builtins.rs:1049` — this bug.
- All other string result allocations (case-map, graphemes, split, join,
  replace) — already use the checked helper; unaffected.

## Fix Design

Replace the plain `add_immediate(return_register, scratch24, 9)` at `:1049` with
the checked-size add (`emit_checked_size_add_immediate`) used at `:557`,
threading its alloc-fail label. No goldens move (the checked path emits the same
success-case arithmetic; it only adds an overflow branch that is never taken for
real inputs).

## Validation Plan

- Full string/unicode suite green; NFC fixtures byte-identical.
- Doc sync: none.

## Summary

Pure hardening-consistency fix: bring the one un-guarded string allocation size
in line with the codebase-wide wrap-checked invariant. Not exploitable today; the
value is to keep the invariant total.
