# 1. Design Principles

1. **Readable over terse** — English keywords, `END X` blocks, line-oriented.
2. **Functional, no OOP** — plain data (records/unions) + free functions. No classes, methods, `self`, or inheritance.
3. **Immutable by default** — `LET` binds, `MUT` opts into reassignment. No implicit globals, no hidden aliasing.
4. **Optional ceremony** — a 3-line script needs no module header; structure exists when you want it.
5. **Errors as values, invisibly plumbed** — every function call yields its value or fails with an `Error`; success auto-unwraps, errors auto-route to an inline `TRAP` on the failing expression, to a function-level `TRAP`, or propagate. No exceptions, no unwinding.
6. **Package-owned closed domains** — a package owns the unions it defines and the free functions that operate on them. Extension is package layering through explicit composition (`UNION ... INCLUDES ...`), not open inheritance, traits, or retroactive interface implementation.
7. **Predictable memory** — designed for memory-safe implementation through formal ownership, move, copy, freeze, resource, and lexical drop rules. No GC, no refcounting, no manual `free`.
