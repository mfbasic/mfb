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
records the dependency in the package's Binary Representation: imported packages
and used public symbols (`IMPORT_TABLE`), exported/dependency ABI hashes
(`ABI_INDEX`), the package's own functions with structured bodies in the `IR`
section (`FUNCTION_TABLE`), and exported-name→function-id mapping (`EXPORT_TABLE`).
The on-disk encoding of these sections is specified by
`./mfb spec package metadata-encoding`.

The package format remains architecture-independent. Native symbols are derived
later by the executable native backend.

## See Also

* ./mfb spec package metadata-encoding — IMPORT_TABLE / ABI_INDEX / FUNCTION_TABLE / EXPORT_TABLE encoding
