# Isolation

`ISOLATED` means the worker is callable from a separate runtime thread without
capturing current stack locals, closures, or current-package private state.

An `ISOLATED` declaration must itself be an exported `FUNC` (not a `SUB`, and not
private/package-visible). `typecheck.rs` enforces this at declaration time,
reporting `ISOLATED function `<name>` must be an exported FUNC declaration.` for a
violation. This is independent of the call-site check in `thread::start`, which
additionally requires the entry to come from an *imported* package. [[src/typecheck.rs:check_thread_builtin_call]]

An isolated worker may still call:

- Built-in package functions such as `io::print`, `fs::readText`, and
  `strings::split`.
- Public exports from packages it imports.
- Other code that is reachable through package metadata and native linking.

The worker must not depend on the parent stack frame. Values passed to a thread
or through thread queues are transferred by the runtime representation rules for
their type. Immutable owned values may be shared or copied only when that is
safe for the value representation; mutable or unique resources must preserve
ownership rules.

For copyable sendable values, crossing a thread boundary copies the value into the
receiving side's arena. Because every non-resource value is a flat, pointer-free
block, this is a single allocation plus byte copy (see
`./mfb spec memory heap-values`); the sender keeps its own block and the receiver
owns and reclaims the copy. The boundary copy is *not* the builder's
`copy_flat_block`: the queue-write helpers hand-emit `arena_alloc` plus a byte-copy
loop, using the receiver's arena state read from the control block (worker arena
state at offset 80, parent arena state at offset 88 for worker→parent sends). The
conceptual model (flat block, single alloc + copy) holds. [[src/target/shared/code/mod.rs:thread_queue_write_helper]]

The move-consumes rule for non-copyable sendable values (including sendable
resource handles) — a successful `thread::start`/`thread::send` consumes the
source binding, and later use is an after-move error — is owned by
`./mfb spec language memory-semantics`.

## See Also

* ./mfb spec memory heap-values — the flat, pointer-free block copied across arenas
* ./mfb spec language memory-semantics — ownership, move, and copy rules
* ./mfb spec threading queue-semantics — the runtime move/copy behavior
