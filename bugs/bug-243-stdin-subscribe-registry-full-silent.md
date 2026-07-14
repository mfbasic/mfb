# bug-243: stdin subscribe on a full registry silently "succeeds", so later reads fail with a misleading ErrInvalidContext

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: footgun

Status: Open

When the 128-slot subscriber registry is full, `lower_stdin_subscribe`
(`src/target/shared/code/stdin_broadcast.rs:683-691`, the `find` loop) silently
branches to `unlock` and returns success without registering the thread, so that
thread's later stdin reads fail with a misleading `ErrInvalidContext` ("not
subscribed") and its stdin bytes are lost with no diagnostic.

Trigger: a program spawns more than `STDIN_LOG_MAX_SUBSCRIBERS` (128)
concurrently-live threads that each `thread::openStdIn(...)`; the 129th+ never
subscribes and its `readByte`/`readChar` returns `-2` → `ErrInvalidContext`.

Fix: on registry-full, surface a distinct capacity error (or document the cap and
return a dedicated diagnostic) instead of a no-op that later masquerades as "not
subscribed".
