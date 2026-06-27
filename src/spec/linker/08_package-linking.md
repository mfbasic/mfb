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

[[src/target/shared/nir.rs:merge_packages]]

Package functions are therefore ordinary merged functions by the time the linker
runs. A package-to-package call is an internal `branch26` to an `_mfb_fn_…`
symbol, resolved through package identity rather than any raw binary-representation
function id — the linker never assumes package-local function ids are globally
unique.

The only true NIR imports are native `LINK` thunks and platform symbols (see
`import-selection`); package exports are not among them.

## Native LINK thunks and the per-program initializer

Native `LINK` bindings (declared in `language native-libraries`) are wired at the
NIR/symbol level by two backend-defined internal symbols rather than by external
linkage.

* `LINK_INIT_SYMBOL` (`_mfb_linker_init`) is the per-program load-time
  initializer: it runs `dlopen`/`dlsym` to resolve every linked native symbol
  before `main`.
* `link_thunk_symbol(alias, name)` produces one marshaling thunk per `LINK`
  function, named `_mfb_linker_<alias>_<name>` (each `alias`/`name` component is
  sanitized so non-alphanumeric, non-`_` characters become `_`).

[[src/target/shared/nir.rs:LINK_INIT_SYMBOL]] [[src/target/shared/nir.rs:link_thunk_symbol]]

NIR lowering routes calls to these thunks through the ordinary import path:
`link_routing_imports` emits a synthetic `NirImport` (package `"link"`) for every
`LINK` function, mapping the qualified call name `alias.name` to its
`link_thunk_symbol`. Re-export aliases route to the same thunk as their `LINK`
target; each alias is registered under both `binding.alias` (as importers see it)
and the bare alias name (as the defining project sees it), so either form
resolves.

[[src/target/shared/nir.rs:link_routing_imports]]

The object plan does not treat these as unresolved imports. When the module has
any `LINK` functions, `plan.rs` collects `LINK_INIT_SYMBOL` plus one
`link_thunk_symbol` per function into `link_symbols`, which the plan records as
DEFINED local symbols the backend emits — not external symbols to be satisfied at
link time.

[[src/target/shared/plan.rs:link_symbols]]

## Transitive platform imports

Because a package body is real merged code, any runtime helper it uses pulls in
that helper's platform imports as if the application had called it directly. See
./mfb spec linker import-selection for the canonical treatment.

This decode-and-merge path is the same one the binary-representation topic of
`mfb spec architecture` describes.

## See Also

* ./mfb spec linker import-selection — transitive platform imports from merged
  package bodies
* ./mfb spec architecture binary-representation — the decode-and-merge package
  narrative
* ./mfb spec architecture native-ir — the NIR layer that lowers LINK call routing
  and thunk symbols
* ./mfb spec language native-libraries — the LINK binding surface these thunks and
  the initializer implement
