# bug-77: string self-append regrow abandons the old buffer without freeing it

Last updated: 2026-07-10
Effort: small (<1h)
Severity: MEDIUM (unbounded leak in a common loop)

`lower_string_self_append_one` (`src/target/shared/code/builder_inplace_assign.rs`,
around `:527-529`) installs the grown buffer over the old pointer without freeing
the old one. This is the bug-01 / bug-47 leak class at a site neither of those
fixes covered.

`s = s & "x"` in a loop therefore leaks the previous buffer on every regrow.

The single correct behavior a fix produces: a string self-append that regrows frees
the buffer it abandons, and RSS is flat across iterations.

## Discovery

Found during the bug-47 audit (commit 70cc6e06), which owned
`builder_collection_mutate.rs` / `builder_control.rs` and scoped this file out.

## Failing Reproduction

```basic
IMPORT io

FUNC main AS Integer
  MUT s AS String = ""
  FOR i AS Integer = 1 TO 200000
    s = s & "x"
  NEXT
  io::print(toString(len(s)))
  RETURN 0
END FUNC
```

RSS grows with the number of regrows rather than tracking the final string length.

## Root Cause

The regrow path allocates a larger buffer, copies, and stores the new pointer. The
old pointer is simply dropped.

## Goal

- The abandoned buffer is freed on regrow; RSS is flat in the iteration count.

### The complication — why bug-47 did not just fix it

The free is entangled with the **static-string vs arena-string** distinction that
bug-06 (commit 76be7640) established: a `String` may point at rodata (a literal, or
the result of a fold), which must never be handed to `arena_free`. The self-append
path can be reached with either carrier. A naive free here is a crash, not a leak.

## Blast Radius

- `lower_string_self_append_one`, and any sibling in `builder_inplace_assign.rs`
  that regrows a string buffer in place.

## Fix Design

Reuse whatever discriminator bug-06 introduced to tell an arena string from a
static one (or arrange that the self-append path has already copied a static source
into the arena, in which case the buffer is unconditionally freeable). Then free
the old buffer before installing the new one, spilling the new pointer across the
`bl` per the register-lifetime rules. Follow `emit_free_pre_grow_buffer`
(bug-47) as the model.

## Phases

### Phase 1 — failing test

- [ ] Measure RSS across 200k self-appends; confirm linear growth.
- [ ] Establish which carriers reach this path (static vs arena), with a test each.

### Phase 2 — the fix

- [ ] Free the abandoned buffer, guarded on the carrier.

### Phase 3 — validation

- [ ] RSS flat across a 10x iteration increase; no double-free under Guard Malloc.
- [ ] `scripts/test-accept.sh`.

## Summary

`s = s & "x"` leaks the old buffer on every regrow. The fix is bug-47's, but it
must first distinguish an arena buffer from a static one — freeing rodata would
turn a leak into a crash.
