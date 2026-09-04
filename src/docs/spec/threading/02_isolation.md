# Isolation

`ISOLATED` means the worker is callable from a separate runtime thread without
capturing current stack locals, closures, or current-package private state.

An `ISOLATED` declaration must itself be a top-level `FUNC` — not a `SUB`, a
lambda, a closure or a local function. It is **independent of visibility**:
`PRIVATE`, `PUBLIC` (the default) and `EXPORT` are all valid. The compiler
enforces the declaration form at declaration time, reporting
`ISOLATED function `<name>` must be a top-level FUNC declaration.` for a
violation. That is a separate check from the call-site one in `thread::start`,
which requires only that the entry name an `ISOLATED FUNC`; neither check
considers where the entry is reached from. [[src/ir/shape.rs:check_builtin_call]]

An isolated worker may still call:

- Built-in package functions such as `io::print`, `fs::readText`, and
  `strings::split`.
- Public exports from packages it imports.
- Other code that is reachable through package metadata and native linking.

Package-level and module-level globals are **per-thread**, not shared. A worker
runs on its own arena, and the writable globals region lives in that arena, so
each worker gets its own copy initialized from the same declarations the main
thread runs — a global reads its declared value in a worker exactly as it does
outside one. A worker's write to a `MUT` global is therefore visible only within
that worker; the parent's copy is untouched, and values cross the boundary only
through the queues. The same applies to a native `LINK` binding's resolved
function pointers, which occupy slots in that region and are resolved per worker.
`./mfb spec threading thread-runtime-helpers` owns the mechanism.

The worker must not depend on the parent stack frame. Values passed to a thread
or through thread queues are transferred by the runtime representation rules for
their type. Immutable owned values may be shared or copied only when that is
safe for the value representation; mutable or unique resources must preserve
ownership rules.

For copyable sendable values, crossing a thread boundary deep-copies the value and
hands the copy to the receiving side. Because every non-resource value is a flat,
pointer-free block, this is a single allocation plus byte copy (see
`./mfb spec memory heap-values`); the sender keeps its own block and the receiver
owns and reclaims the copy. The boundary copy is the builder's ordinary
flat-block copy, made at the send site **in the sender's own arena** — the
arena-state register is never repointed at another thread's state, because that
thread may be allocating from it at the same instant and the allocator's free-list
pop is unsynchronized (bug-498). The queue-write helper stores the copied pointer
into the queue slot, and the receiver frees it, later, into *its* own arena: a free
only touches the freeing thread's arena state, and no arena but the main one is
ever torn down (that one only at process shutdown), so the block stays mapped and
the hand-over is race-free. [[src/codegen/cleanup/thread/builder_thread_cleanup.rs:emit_thread_send_runtime_helper_call]] [[src/codegen/memory/arena/builder_arena_transfer.rs:copy_value_to_current_arena]]

The move-consumes rule for non-copyable sendable values (including sendable
resource handles) — a successful `thread::start`/`thread::send` consumes the
source binding, and later use is an after-move error — is owned by
`./mfb spec language memory-semantics`.

## See Also

* ./mfb spec memory heap-values — the flat, pointer-free block copied across arenas
* ./mfb spec language memory-semantics — ownership, move, and copy rules
* ./mfb spec threading queue-semantics — the runtime move/copy behavior
