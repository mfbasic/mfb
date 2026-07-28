# bug-141 — Returning a resource union truncates it to 8 bytes and double-closes it → SIGSEGV

**Status:** FIXED (commit e0fa88b8, 2026-07-11).
**Severity:** HIGH — `RETURN <resource union>` crashes and/or double-closes the
resource.
**Class:** memory-safety.

## Finding

Three sites:
- `src/target/shared/code/builder_collection_layout.rs:11-20`
  (`inline_collection_payload_size`: union size = `8*(1+max_fields)`; resource
  variants have empty `union_variant_fields` per validation.rs:285-292, so an
  all-resource union sizes to **8**, not its real 16-byte `{tag@0, ptr@8}`
  layout).
- `src/target/shared/code/builder_codegen_primitives.rs:1906-1928`
  (`emit_return_exit_inner` deactivates Thread/Resource/OwnedList cleanups but
  never a `ResourceUnion` cleanup).
- `src/target/shared/code/builder_arena_transfer.rs:345-349`
  (`copy_union_to_current_arena` uses the same wrong size for a
  thread-transferred resource union — latent, sendability rules may block it).

`RETURN <resource union>` goes through `materialize_inline_value_in_arena`'s
fixed-size path with size 8: only the tag word is copied, the resource pointer
at +8 is lost (and read out-of-block by the caller). Additionally the callee's
tag-dispatched drop still runs on return, closing the resource the caller thinks
it owns.

## Trigger (reproduced)

```
UNION Stream (File | Socket)
FUNC open() AS Stream
  RES s = fs::createTempFile()
  RETURN s
END FUNC
```
Caller `MATCH`es and writes → SIGSEGV (exit 139).

## Fix

Size resource-union payloads by their real `{tag, ptr}` layout (16 bytes), and
deactivate the `ResourceUnion` cleanup on the return path so the resource isn't
double-closed.

## Resolution

FIXED in commit e0fa88b8. resource-union payload sizes to 16 bytes; emit_return_exit_inner deactivates the ResourceUnion cleanup on return.

Regression test: `tests/rt-behavior/resources/bug141_resource_union_return` (fails on the unfixed compiler). Full
acceptance (871) and `cargo test` pass.
