# Function Ids And Package Calls

In the Binary Representation, calls reference functions by id, as an
`IrValue::Call` or `IrValue::CallResult` node naming the target. The
`CallResult` auto-unwrap is the ordinary `Result` desugaring owned by
`./mfb spec language error-model` — not a thread-specific mechanism. [[src/ir.rs:CallResult]]

Inside one package, local function ids are package-local. They are not globally
unique across `.mfp` files.

When compiling a package against dependencies, imported exported functions are
referenced by their imported logical identity (import name plus the resolved ABI
identity recorded in `IMPORT_TABLE`/`ABI_INDEX`), not baked against another
package's local ids.

At consumption time, the executable decodes each imported package's Binary
Representation back into IR functions and **merges** them into the project IR
under each package's deterministic identity prefix (`<id>.package.symbol`). This
identity-prefixed IR merge is the generic consumption mechanic owned by
`./mfb spec architecture binary-representation`; it applies the prefix as a consistent link-time
rename of every definition and reference and de-duplicates content reached via two
dependency paths. [[src/ir.rs:prefix_package_symbols]]

The thread-relevant consequence: the consumer must not assume that package-local
function id `0` in two packages is the same function. It resolves through package
identity plus exported symbol during the IR merge, before anything is lowered, so
a worker entry point and any cross-package calls it makes name concrete prefixed
functions by the time lowering runs.

## See Also

* ./mfb spec language error-model — `CallResult` auto-unwrap (`MATCH`/`PROPAGATE`)
* ./mfb spec architecture binary-representation — the identity-prefixed package IR merge
