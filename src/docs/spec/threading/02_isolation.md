# Isolation

`ISOLATED` means the worker is callable from a separate runtime thread without
capturing the caller's stack locals or sharing the caller's instance of
package state. Private globals remain accessible through the worker's own arena.

An `ISOLATED` declaration must itself be a top-level `FUNC` — not a lambda, a
closure or a local function. It is **independent of visibility**:
`PRIVATE`, `PUBLIC` (the default) and `EXPORT` are all valid. The compiler
enforces the declaration form at declaration time, reporting
`ISOLATED function `<name>` must be a top-level FUNC declaration.` for a
violation. That is a separate check from the call-site one in `thread::start`,
which requires only that the entry name an `ISOLATED FUNC`; neither check
considers where the entry is reached from. [[src/ir/verify/mod.rs:collect_diagnostics_with]] [[src/ir/shape.rs:thread_start_entry_valid]]

An isolated worker may still call:

- Built-in package functions such as `io::print`, `fs::readText`, and
  `strings::split`.
- Public exports from packages it imports.
- Other code that is reachable through package metadata and native linking.

Package-level and module-level globals are **per-thread**, not shared. A worker
runs on its own arena, and the writable globals region lives in that arena, so
each worker reruns the same global initializer as the main thread. This is a
fresh evaluation of each declaration, not a snapshot of the parent's values; an
initializer that reads external state can therefore produce a different value.
A worker's write to a `MUT` global is therefore visible only within
that worker; the parent's copy is untouched, and values cross the boundary only
through the queues. The same applies to a native `LINK` binding's resolved
function pointers, which occupy slots in that region and are resolved per worker.
`./mfb spec threading thread-runtime-helpers` owns the mechanism.
[[src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_trampoline]]

The worker must not depend on the parent stack frame. Values passed to a thread
or through thread queues are transferred by the runtime representation rules for
their type. Immutable owned values may be shared or copied only when that is
safe for the value representation; mutable or unique resources must preserve
ownership rules.

For copyable sendable values, crossing a thread boundary deep-copies the value and
hands the copy to the receiving side. A flat, pointer-free value is a single
allocation plus byte copy (see `./mfb spec memory heap-values`); a value of a
recursive type (one whose declaration reaches itself, such as a tree whose nodes
hold a `List OF` the node type) is a graph of separate blocks, and is copied by
the module's one non-recursive walker, which copies a block at a time and keeps the
edges still to copy on a work stack in the arena, so a value of any depth copies
without growing the native stack. [[src/codegen/memory/arena/graph_copy.rs:lower_graph_copy_walker]]
Either way the sender keeps its own value and the receiver owns and reclaims its
copy. The copy goes through the thread's **block arena**, the arena the thread's
control block owns and which belongs to neither side: a send deep-copies the
value into the block arena under the block lock, once the enqueue is certain,
and a read deep-copies the queued value out of it into the reader's own arena.
The arena-state register is pointed at the block arena only for the length of
that copy, under the lock that serializes every allocation in it, and never at
another thread's arena — that thread may be allocating from its own at the same
instant, and the allocator's free-list pop is unsynchronized. `thread::start`'s
seed and the worker's result cross the same way. `./mfb spec threading
control-block` owns the block arena, and `queue-semantics` the queue side.
[[src/codegen/cleanup/thread/builder_thread_cleanup.rs:emit_thread_send_runtime_helper_call]] [[src/codegen/runtime/thread/control_block.rs:emit_call_in_block_arena]] [[src/codegen/memory/arena/builder_arena_transfer.rs:copy_value_to_current_arena]]

So `thread::start` and `thread::send` never consume their value: the caller keeps
it. A resource crosses only on a resource plane, by `thread::transfer`, which
moves it — the sender's binding is consumed on success and kept on failure. The
move rule is owned by `./mfb spec language resource-management` and §16
(`./mfb spec language threads`).

## See Also

* ./mfb spec memory heap-values — the flat, pointer-free block copied across arenas
* ./mfb spec language memory-semantics — ownership, move, and copy rules
* ./mfb spec threading queue-semantics — the runtime move/copy behavior
