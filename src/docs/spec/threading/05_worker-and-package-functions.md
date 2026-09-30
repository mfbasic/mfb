# Worker And Package Functions In The Single Codegen

Local workers are ordinary project IR functions; imported workers are ordinary
IR carried in the package's Binary Representation. Once the consumer decodes
and merges a package's IR, **every** package function — including thread workers — is lowered through the same
`IR -> NIR -> native` path as the executable's own code.

Consequently package functions automatically get every language feature the
executable path has: full control flow (`IF`/`WHILE`/`FOREACH`/`MATCH`),
function-level and inline `TRAP`, all built-ins, and inline-`TRAP`-on-a-built-in.
A worker body's `CallResult` of a built-in follows ordinary IR lowering.

Each merged package function receives a stable internal native symbol so
the linker can resolve cross-package and worker entry points. There is **no**
separate `_mfb_pkg_*` namespace: every merged package function — like the
executable's own functions — routes through the ordinary `_mfb_fn_` namespace.
The symbol is `_mfb_fn_<fragment>`, where `<fragment>`
is the function's merged IR name with every byte outside ASCII letters and
digits — including `_` itself — escaped to an unambiguous `_XX` two-hex-digit
form. [[src/target/shared/nir/symbols.rs:function_symbol]] [[src/target/shared/nir/symbols.rs:symbol_fragment]] Escaping the interior `_` keeps a mangled overload
name such as `f$List$OF$Integer` distinct from a user function literally named
`f_List_OF_Integer`. For a merged IR name `abc.package.symbol`, the resulting
symbol is `_mfb_fn_abc_2Epackage_2Esymbol`: the dots are escaped too.
Compiler-internal sigil-prefixed functions use a
reserved `_mfb_ifn_` namespace instead, which user and package functions can
never reach. [[src/target/shared/nir/symbols.rs:function_symbol]]

Cross-package calls and worker entry points resolve to these symbols after the IR
merge, with `Nothing` results initialized to the canonical zero value, the same
as for the executable's own functions.

## See Also

* ./mfb spec linker symbols-and-relocations — how `_mfb_fn_*` symbols are resolved at link time
