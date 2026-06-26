# Function Ids And Package Calls

In the Binary Representation, calls reference functions by id. A call is an `IrValue::Call`
or `IrValue::CallResult` node naming the target; auto-unwrapping is the ordinary
`Result` desugaring (a `MATCH`/`PROPAGATE` over the `CallResult`), not an opcode
pair.

Inside one package, local function ids are package-local. They are not globally
unique across `.mfp` files.

When compiling a package against dependencies, imported exported functions are
referenced by their imported logical identity (import name plus the resolved ABI
identity recorded in `IMPORT_TABLE`/`ABI_INDEX`), not baked against another
package's local ids.

At consumption time, the executable decodes each imported package's Binary Representation
back into IR functions and **merges** them into the project IR under each
package's deterministic identity prefix (`<id>.package.symbol`). The merge applies
that prefix as a consistent link-time rename of every definition and every
reference, driven by the resolved dependency graph, and resolves logical
inter-package references to concrete prefixed names. Identical content reached via
two dependency paths shares one prefix and de-duplicates.

The consumer must not assume that package-local function id `0` in two packages is
the same function. It resolves through package identity plus exported symbol
during the IR merge, before anything is lowered.
