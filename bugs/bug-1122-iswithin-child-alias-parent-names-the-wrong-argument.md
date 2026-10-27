# bug-1122: `fs::isWithin`'s `child` parameter is aliased `parent`, the opposite of what it names

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: LOW
Class: API (registry descriptor)

Status: Open
Regression Test: —

`src/codegen/builtins/fs/func_is_within.rs:register` gives the tested path
`child` the alias `parent`, and the container `base` the alias `path`. So
`fs::isWithin(path := root, parent := candidate)` is the correct spelling and
reads backwards: `parent :=` names the path being tested, not its container.
Probe: `fs::isWithin(path := t & "/a", parent := t & "/a/b")` → `TRUE`.

**The correct behavior a fix produces:** the `parent` alias is removed (or
moved to `base`, with a deprecation path decided by the owner); the page then
drops its "also accepted as `parent`" warning.

References:

- Found by plan-125-D Phase 5, finding `fs-isWithin` 1. Alias introduced by
  `73374e779` (`git log -S'aliases: &["parent"]'`).
