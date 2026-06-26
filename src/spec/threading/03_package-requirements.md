# Package Requirements

Thread entry functions live in `.mfp` packages. The executable imports the worker
package, and the worker package may import additional packages.

For example:

```text
app
  imports thread_import_worker

thread_import_worker
  exports callImpPrint
  imports thread_imp_print

thread_imp_print
  exports impPrint
```

If `callImpPrint` starts in a thread and calls `thread_imp_print::impPrint`, that
call must be resolved from package metadata, not from app source text.

Package builds do not merge dependency IR into the generated `.mfp`.
Instead, a package build compiles against installed dependency ABI metadata and
records the dependency in the package's Binary Representation:

- `IMPORT_TABLE` records imported packages.
- `IMPORT_TABLE.usedSymbols` records the imported public symbols used while
  compiling the package.
- `ABI_INDEX` records the ABI hashes for exported symbols and dependency used
  symbols.
- `FUNCTION_TABLE` describes the package's own functions; their bodies are the
  structured Binary Representation carried in the `IR` section.
- `EXPORT_TABLE` maps exported source names to package-local function ids.

The package format remains architecture-independent. Native symbols are derived
later by the executable native backend.
