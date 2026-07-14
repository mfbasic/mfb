# bug-229: linux_x86_64 code.rs module header still describes plan-00-H "Phase 1" stubbed state

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: docs

Status: Open

The module header of `src/target/linux_x86_64/code.rs:1-8` still describes
plan-00-H "Phase 1" state — claiming the io/fs/net/term runtime-helper methods
"return a `Phase 1: <name> not yet implemented` error" and are "unreachable" —
but every one of those `CodegenPlatform` methods is now fully implemented
(plan-00-H complete, per `mod.rs:26-39`).

Trigger: a maintainer consulting the header believes the runtime surface is
stubbed/unreachable and reasons incorrectly about the file; no runtime effect.

Fix: update the header to reflect the completed console runtime surface (drop the
"not yet implemented / unreachable" paragraph).
