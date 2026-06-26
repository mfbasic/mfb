# Linking Requirements

Executable native linking receives:

- The app's native module.
- The installed `.mfp` package files listed by the app manifest.
- Package exports from every installed package.
- Package import and ABI metadata from every installed package.
- Runtime helper symbols requested by app and package IR.

The native backend must:

1. Read all installed package exports.
2. Decode each installed package's Binary Representation and merge its IR functions into the
   project IR under the package identity prefix.
3. Lower every merged package function through `IR -> NIR -> native`, deriving
   each package export symbol.
4. Add runtime helper imports for built-ins used inside package IR.
5. Resolve app calls to imported package exports.
6. Resolve package calls to other imported package exports by using the
   importing package's `IMPORT_TABLE` and the installed package set.
7. Validate ABI hashes before treating an installed package export as satisfying
   an imported used symbol.
8. Emit OS-specific runtime helper implementations or imports.
9. Link the final executable so worker calls, package calls, and runtime helpers
   all resolve to native symbols.

For Linux, runtime helpers used only inside package IR must still add the
same platform dynamic imports as helpers used by the app package. For example,
a worker package that calls `fs::readText`, `io::print`, or `thread::start`
must cause the final Linux executable to import the required libc, libm, or
libpthread symbols even if the app source does not call those helpers directly.

It is not valid to make a package-to-package call by preserving a raw
package-local function id and hoping the executable package order makes it
correct. Function ids are scoped to Binary Representation payloads; the IR merge resolves
them through package identity plus exported symbol, and native symbols plus ABI
metadata define the executable-level call graph.
