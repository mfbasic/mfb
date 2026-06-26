# Static Versus Dynamic Output

If an encoded image has no imports, a target may emit a simpler executable with
only internal text/data and no dynamic library metadata.

If imports are present, the target linker must emit dynamic loading metadata:

- Dynamic dependency records for every imported library.
- Dynamic symbol/string tables for every imported symbol.
- Relocations that let the runtime loader fill GOT entries.
- Import stubs that generated code can branch to.

The compiler must not silently omit a required library. Missing imports are
linker or codegen errors.
