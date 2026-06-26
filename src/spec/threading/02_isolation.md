# Isolation

`ISOLATED` means the worker is callable from a separate runtime thread without
capturing current stack locals, closures, or current-package private state.

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

For copyable sendable values, crossing a thread boundary copies or freezes the
value as required by the representation. The sender's original binding remains
usable. Because every non-resource value is a flat, pointer-free block
(`memory_layouts.md`), this copy is a single `arena_alloc` + `memcpy`
(`copy_flat_block`) into the receiver's arena — the same generic routine ordinary
value copies use, with no per-type deep-copy glue. The sender keeps its own block
and frees it on its own scope-drop; the receiver owns and reclaims the copy.

For non-copyable sendable values, including sendable resource handles, crossing a
thread boundary is an ownership move. A successful `thread::start` or
`thread::send` consumes the source binding on that control-flow path. Later use
of the moved binding is an after-move error.
