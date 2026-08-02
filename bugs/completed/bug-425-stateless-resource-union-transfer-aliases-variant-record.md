# bug-425: transferring a resource union across a thread aliased the sender's variant record (UAF)

Last updated: 2026-08-02
Effort: small (fixed as part of plan-75)
Severity: HIGH (memory safety — use-after-free across a thread boundary)
Class: Codegen / Runtime memory safety

Status: Fixed on the plan-75 branch (`worktree-P-75`, merged to main). This defect was
discovered and fixed while implementing plan-75; it is recorded here because it is a
latent memory-safety defect independent of plan-75's STATE feature — it was true for a
**stateless** resource union as much as a stateful one.

## Symptom

`thread::transfer` of a resource union (`UNION Stream { File, Socket }`, value layout
`{tag@0, variant-record-ptr@8}`) copied only the 16-byte `{tag, ptr}` block into the
receiver's arena. The `+8` variant-record pointer was copied **verbatim**, so the
receiver's union still pointed at the *sender's* 80-byte File/Socket record. When the
sender thread tore down its arena, the receiver's live union aliased freed memory — a
classic bug-257-class use-after-free. (Before plan-75 the transfer failed to compile at
all for a stateful union, but the *stateless* aliasing was a real, silent miscompile
the moment the compile path was opened.)

## Root cause

`CodeBuilder::copy_union_fields_into_existing`
(`src/target/shared/code/builder_arena_transfer.rs`) deep-copied a **data** union's
inlined variant record (at `+16`) but had no branch for a **resource** union: resource
variants (`File`/`Socket`) carry no record fields in `union_variant_fields`, so the
non-data-union field loop iterated zero fields and left the `+8` pointer aliasing the
source. The concrete-resource path (`copy_resource_to_current_arena`) already did the
correct deep-copy for a plain `RES File`; the union path never dispatched to it.

## Fix

Add a resource-variant branch to `copy_union_fields_into_existing`: for the active
resource variant, load the source record pointer at `+8`, deep-copy the variant record
(and its uniform STATE payload, if any) via `copy_resource_to_current_arena`, and repoint
the receiver copy's `+8` at the fresh record. The source record is flagged
`moved|closed` by that helper, so the handle is closed exactly once. This severs the
cross-arena alias for both stateless and stateful resource unions.

## Regression coverage

`tests/rt-behavior/threads/thread-transfer-union-state-rt` (stateful, asserts the STATE
`99` survives the boundary) and the stateless resource-union transfer fixture landed by
plan-75. A 20× repeat run of each proves sender/receiver payload independence (no
shared-arena UAF).
