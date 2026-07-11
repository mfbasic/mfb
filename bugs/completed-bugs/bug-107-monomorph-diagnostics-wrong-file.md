# bug-107 — Every monomorph diagnostic is attributed to the first file

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G2). **Reproduced with the
binary.**
**Severity:** MED — misleading diagnostics in any multi-file project.
**Class:** footgun (diagnostic misattribution).

## Finding

`src/monomorph/lower.rs:1580-1589` (`report`) joins the diagnostic line number
with `self.source.files.first()`'s path unconditionally. In any multi-file
project, a monomorph error (`TYPE_OVERLOAD_AMBIGUOUS`,
`TYPE_CALL_ARITY_MISMATCH`, `TYPE_CALL_ARGUMENT_MISMATCH`) in a later file
prints the **wrong filename** and a source excerpt from the wrong file at that
line.

## Trigger (reproduced)

A return-type-ambiguous `make()` call at `src/other.mfb:9` is reported as
`./src/main.mfb:9`, with main.mfb's (4-line) source shown around a nonexistent
line 9.

## Fix sketch

Thread the originating file index/path through the monomorph error sites (the
AST nodes carry a span/file id) instead of defaulting to `files.first()`.
