# bug-146 — thread-transfer payload fix-up walks `capacity` entries and trusts uninitialized flags

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G8).
**Severity:** MED (latent) — ~1/256-per-spare-entry chance of a wild deep-copy
on `thread::send` of a grown non-flat collection.
**Class:** memory-safety.

## Finding

`src/target/shared/code/builder_arena_transfer.rs:552-596`
(`fix_collection_transfer_payload`: loop bound `COLLECTION_OFFSET_CAPACITY`,
flags==USED check per entry). Entry tables are dense (`[0..count)` live); slots
`[count..capacity)` of a grown buffer are never initialized (grow paths copy
only `count*ENTRY` bytes), and recycled arena memory is entropy-scrubbed. The
whole-buffer copy (`copy_collection_to_current_arena` sizes by capacity) carries
those garbage entries across, and the fix loop deep-copies any spare entry whose
scrubbed flags byte happens to equal `COLLECTION_ENTRY_FLAG_USED` —
dereferencing garbage offsets in the receiver.

## Trigger

Build a non-flat collection (element with a pointer field) with appends
(headroom), where the buffer was allocated from scrubbed recycled memory;
`thread::send` it → per-spare-entry chance of a wild deep-copy → crash or
corrupted received value.

## Fix

Bound the fix-up loop at `count`, not `capacity` (spare entries are not live and
must not be walked).

## Prior art

MEM-04 in audit-1-codegen-memory.md is the adjacent unchecked-multiply issue, a
different defect.
