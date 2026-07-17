# bug-266: arena free-list idempotency guard is bypassed by the quick-bin and large-bin park paths (double-free defense parity gap)

Last updated: 2026-07-17
Effort: small (<1h)
Severity: LOW
Class: Memory-safety (defense-in-depth)

Status: Open
Regression Test: (none yet)

`arena_free` was hardened (MEM-03) with an idempotency guard so a double-free
becomes a no-op instead of corrupting the free list — but that guard lives only
in `arena_insert_free` (the coalescing list). The now-primary fast paths — the
quick-bin push (chunks `≤ ARENA_QUICK_BIN_MAX`) and the hashed large-bin push —
head-push unconditionally, so a second free of the same live pointer sets
`ptr.next = ptr` (a self-cycle) and leaves `bin[slot] = ptr`; the next two
allocations of that size class both hand back `ptr` → aliasing / UAF / heap
corruption. This is not reachable from `.mfb` runtime inputs alone (it needs a
codegen/ownership double-free), but that is exactly the class this repo has hit
repeatedly (`trap-cleanup-double-free`, `union-drop-codegen-nondeterminism`), and
the MEM-03 guard was added to neutralize it — the bin paths now sit outside its
coverage. The single correct behavior a fix produces: an immediate double-free of
a bin-sized chunk is a no-op, matching `insert_already_free`.

References:

- `planning/audit-2-codegen-memory.md` (MEM-09).
- `src/target/shared/code/entry_and_arena.rs:1655-1662` (quick-bin park),
  `:1673-1681` (large-bin park) — unconditional head-push.
- Guard they bypass: `entry_and_arena.rs:1532-1536` (`arena_insert_free`
  idempotency no-op).

## Failing Reproduction

Not demonstrable from `.mfb` source (no runtime-input path forces a double-free).
Byte-level argument: two `arena_free(A, 48)` with no intervening allocation → the
size-48 quick bin becomes a one-node self-cycle (`A.next = A`, `bin[slot] = A`);
the next two `≤48`-byte allocations both return `A`, aliasing two live objects.
Expected: the second free is a no-op (the chunk is already free), as
`arena_insert_free` already does for coalescing-list chunks.

Contrast: a chunk that reaches `arena_insert_free` is protected today; only the
bin fast paths — which handle the common small/allocation sizes — are not.

## Root Cause

`arena_free` routes `≤ ARENA_QUICK_BIN_MAX` chunks (and larger chunks to a hashed
large bin) to an O(1) head-push (`bin_head = bin[slot]; ptr.next = bin_head;
bin[slot] = ptr`, `entry_and_arena.rs:1658-1661`/`:1678-1681`) that never checks
whether `ptr` is already the bin head. The `insert_already_free` no-op only runs
for chunks that enter the coalescing list, which bin-sized frees never do.

## Goal

- Before the bin head-push, compare `ptr` against `bin[slot]` and branch to a
  no-op on equality (catching the immediate double-free self-cycle), mirroring
  `insert_already_free`. No language-surface change; the non-double-free O(1) fast
  path is unchanged.

### Non-goals (must NOT change)

- Full free-list membership verification / poison / quarantine.
- The O(1) fast path cost for the normal (non-double-free) case.

## Fix Design

In both the quick-bin and large-bin park sequences in `entry_and_arena.rs`, emit
a `ptr == bin[slot]` compare and skip the push on equality (the same shape as the
`arena_insert_free` guard). This catches the immediate-double-free self-cycle,
the highest-value case, without walking the bin.
