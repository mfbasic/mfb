# Package Linking

Installed packages are architecture-independent `.mfp` files. They are *not*
linked as external symbols at native link time. Instead they are merged into the
program IR before lowering, so package code flows through the same
`IR -> NIR -> native` codegen as the application and is emitted under the ordinary
`_mfb_fn_…` symbol namespace. There is no `_mfb_pkg_*` import namespace.

## How exports reach the executable

The merge happens in `nir::merge_packages` (`src/target/shared/nir.rs`):

1. Before native lowering, the compiler reads each installed package header and
   exported ABI metadata and registers external function signatures under
   qualified names (`packageName.exportName`) so calls survive language lowering
   with proper types.
2. For a native executable build, each installed package's binary representation
   is decoded back into IR (`binary_repr::read_package_ir_with_identity`).
3. Every package symbol is prefixed with a per-package identity
   (`ir::prefix_package_symbols`) so two packages cannot collide, and the
   functions, types, globals, and constants are merged into the application IR.
4. The consumer's `package.symbol` references are rewritten to the
   identity-prefixed definitions (`ir::apply_package_identity`).

Package functions are therefore ordinary merged functions by the time the linker
runs. A package-to-package call is an internal `branch26` to an `_mfb_fn_…`
symbol, resolved through package identity rather than any raw binary-representation
function id — the linker never assumes package-local function ids are globally
unique.

The only true NIR imports are native `LINK` thunks and platform symbols (see
`import-selection`); package exports are not among them.

## Transitive platform imports

Because a package body is real merged code, any runtime helper it uses pulls in
that helper's implementation and platform imports exactly as if the application
had called the helper directly. The final executable must include those imports
even when the app package never names the helper itself.

This decode-and-merge path is the same one the binary-representation topic of
`mfb spec architecture` describes.
