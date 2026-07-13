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
`worked-example` shows complete programs; and `documentation` and `test-framework`
cover `DOC` blocks and the `TESTING` / `mfb test` framework.

The compiler-internal contracts that used to live here — the canonical type-string
grammar, the inference/coercion lattice, and the resource-float decision procedure —
are now specified with the rest of the front end in the `architecture` package
(`type-name-encoding`, `type-inference`, `escape-analysis`), and the auditability
tooling in the `tooling` package. See the links below.

## See Also

* ./mfb spec architecture — how the compiler processes this language
* ./mfb spec architecture type-inference — inference, coercion, and the assignability lattice
* ./mfb spec architecture type-name-encoding — the canonical flat type-string grammar
* ./mfb spec architecture escape-analysis — the resource-float decision procedure behind §15
* ./mfb spec tooling auditability — surfacing the language's implicit fallible control flow
* ./mfb spec memory — the runtime memory model for language values
* ./mfb spec package — the package and ABI format for compiled modules
* ./mfb spec threading — isolated thread workers
* ./mfb spec linker — native linking of the emitted code
* ./mfb man — built-in package and function help
