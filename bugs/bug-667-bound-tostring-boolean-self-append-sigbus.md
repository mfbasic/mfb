# bug-667: a bound `toString(<Boolean>)` grown in place frees read-only data (SIGBUS)

Last updated: 2026-09-20
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: (to add) `tests/rt-behavior/general/tostring_boolean_self_append`

`toString(<Boolean>)` returns a pointer to one of two read-only string constants
(`TRUE`/`FALSE`) rather than a fresh arena block. A `MUT` binding initialized
from it and then grown by the in-place self-append (`s = s & "!"`) treats that
pointer as its own buffer: the regrow path `arena_free`s the old buffer, which
writes the free list into read-only memory, and the program dies with SIGBUS
(exit 138) before printing anything.

**The correct behavior:** `MUT a = toString(flag); a = a & "!"` prints
`TRUE!`, and the program exits 0. A `String` produced by `toString` behaves in
every respect like any other `String` value.

References:

- Found while landing plan-140-C (`toString(ENUM)`), whose first design returned
  the member name as a read-only pointer the same way and crashed identically;
  plan-140-C Correction C-C3.
- `src/docs/spec/memory/` — ownership: every live value has exactly one owner.

## Failing Reproduction

```
IMPORT io

FUNC flag(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  MUT a = toString(flag(1))
  a = a & "!"
  io::print(a)
  RETURN 0
END FUNC
```

- Observed (main at b6a10efbc, macOS aarch64): no output, exit 138.
- Expected: `TRUE!`, exit 0.

Contrast: `MUT b = "lit"; b = b & "!"` works (a literal is recognized by
`static_string_value` and copied on bind); `MUT a = typeName(x); a = a & "!"`
works (folded to a constant first).

## Root Cause

`lower_boolean_to_string` (`src/codegen/string/repr/builder_strings.rs`) loads a
string constant and returns it with no ownership mark. The bind's owning store,
`lower_value_owned` (`src/codegen/engine/value/builder_values.rs`), copies only
when `value_needs_owning_copy` says so. That predicate knows literals
(`static_string_value`), the rodata-returning Unicode lookups
(`call_returns_rodata_string`) and parameter borrows, but not
`toString(<Boolean>)`. So the binding stores the read-only pointer as if it were
its own, and `try_inplace_concat_assign` → `lower_string_self_append_one`
(`src/codegen/collection/assign/builder_inplace_assign.rs`) regrows and
`arena_free`s it.

## Goal

- The reproduction prints `TRUE!` and exits 0; the same with `FALSE`.

### Non-goals (must NOT change)

- `toString(<Boolean>)`'s output text.
- Do not fix it by making the in-place append skip regrow-frees in general:
  that would leak every genuine buffer.

## Blast Radius

- `lower_boolean_to_string` — fixed by this bug.
- `toString(<enum>)` (plan-140-C) — unaffected: it returns a fresh marked copy
  by design, for exactly this reason.
- Every other `toString` arm — unaffected: each returns a fresh arena block
  (helper call or `copy_flat_block`).
- Other producers that return string constants without a mark — to audit:
  `grep -rn "emit_load_string_constant" src/codegen` for results returned as a
  call's value.

## Fix Design

Either (a) return a fresh copy from `lower_boolean_to_string` and
`mark_fresh_string` it, as plan-140-C does for enums (costs an allocation per
call, but no second predicate can disagree), or (b) teach
`value_needs_owning_copy` that `toString(<Boolean>)` is read-only (no
allocation, but it depends on the builder statically typing the argument as
`Boolean` at every owning site). Recommended: (a).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/rt-behavior/general/tostring_boolean_self_append` with the
      reproduction; confirm exit 138.
- [ ] Complete the producer audit above.

Acceptance: the test fails for the documented reason.
Commit: —

### Phase 2 — the fix

- [ ] Apply the chosen design in `lower_boolean_to_string`.

Acceptance: the Phase 1 test passes.
Commit: —

### Phase 3 — full validation

- [ ] `bash scripts/artifact-gate.sh target/release/mfb all`: diffs only where
      `toString(<Boolean>)` codegen changed, each inspected.
- [ ] `scripts/test-accept.sh target/debug/mfb target/accept-actual` → all pass.

Commit: —

## Summary

A one-arm ownership gap: a read-only result reaches an owning store that does
not recognize it. The risk is in the audit for other constant-returning
producers, not in the fix.
