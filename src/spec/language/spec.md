# ⟪MFBASIC⟫ — Language Specification

## Modern Functional Basic (MFB)

A modern, functional dialect of BASIC. Immutable by default, no objects, package-level imports, and an implicit error model. Every function call **produces its value on success and fails with an `Error` on failure**; the value auto-unwraps and the `Error` auto-propagates. Errors auto-route to an inline `TRAP` on the failing expression, to a function-level `TRAP`, or propagate to the caller — no `TRY`, no `GOTO`, no exceptions. The language is designed for memory-safe implementation through owned values, explicit resource ownership, and lexical cleanup.

## Reading order

The topics below follow the language from the small to the large. `design-principles`
and `lexical-structure` set the philosophy and surface syntax; `templates` covers
monomorphized generics; `types` defines the primitive, record, union, enum,
collection, and thread types plus inference, defaults, and comparability.
`bindings-and-scope`, `functions`, and `subs` specify `LET`/`MUT`/`RES` binding,
value-producing functions, and effect-only subs. `error-model`, `pattern-matching`,
`control-flow`, and `operators` give the implicit-failure model, `MATCH`,
structured control flow, and operator precedence. `collections`, `modules-and-packages`,
`memory-semantics`, `resource-management`, and `threads` cover owned collections,
visibility and package resolution, the ownership/copy/move/drop model, resource
handles, and isolated thread workers. `native-libraries` specifies `LINK` bindings;
`builtin-functions` lists the built-ins; `grammar` is the abridged EBNF;
`worked-example` shows complete programs; and `build-artifacts`, `tooling-and-auditability`,
and `documentation` cover `.mfp` packages and verification, the audit/format/test
tooling, and `DOC` blocks.
