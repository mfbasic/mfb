# Isolation

`ISOLATED` means the worker is callable from a separate runtime thread without
capturing current stack locals, closures, or current-package private state.

An `ISOLATED` declaration must itself be a project-visible `FUNC` — a `PUBLIC`
(the default) or `EXPORT` `FUNC`, not a `SUB` and not `PRIVATE`. The compiler
enforces this at declaration time, reporting `ISOLATED function `<name>` must be a
project-visible FUNC declaration (PUBLIC — the default — or EXPORT, not PRIVATE).`
for a
violation. This is independent of the call-site check in `thread::start`, which
additionally requires the entry to come from an *imported* package. [[src/syntaxcheck/builtins.rs:check_thread_builtin_call]]

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

For copyable sendable values, crossing a thread boundary copies the value into the
receiving side's arena. Because every non-resource value is a flat, pointer-free
block, this is a single allocation plus byte copy (see
`./mfb spec memory heap-values`); the sender keeps its own block and the receiver
owns and reclaims the copy. The boundary copy is the builder's ordinary
flat-block copy. At the send site the builder points the arena-state
register at the *receiver's* state (read from the control block — worker arena
state at offset 80, parent arena state at offset 88 for worker→parent sends) and
copies the message into that arena; the queue-write helper then stores the
already-copied pointer into the queue slot. [[src/target/shared/code/builder_emit_helpers.rs:emit_thread_send_runtime_helper_call]] [[src/target/shared/code/builder_arena_transfer.rs:copy_value_to_current_arena]]

The move-consumes rule for non-copyable sendable values (including sendable
resource handles) — a successful `thread::start`/`thread::send` consumes the
source binding, and later use is an after-move error — is owned by
`./mfb spec language memory-semantics`.

## See Also

* ./mfb spec memory heap-values — the flat, pointer-free block copied across arenas
* ./mfb spec language memory-semantics — ownership, move, and copy rules
* ./mfb spec threading queue-semantics — the runtime move/copy behavior
