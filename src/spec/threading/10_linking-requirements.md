# Linking Requirements

Executable native linking receives:

- The app's native module.
- The installed `.mfp` package files listed by the app manifest.
- Package exports from every installed package.
- Package import and ABI metadata from every installed package.
- Runtime helper symbols requested by app and package IR.

The native backend runs the ordinary executable link pipeline — decode and merge
each installed package's IR under its identity prefix, lower every merged function
(including thread workers) through `IR -> NIR -> native`, resolve app and
inter-package calls via `IMPORT_TABLE`/ABI validation, and link so worker calls,
package calls, and runtime helpers all resolve to native symbols. The pipeline and
import-selection mechanics are owned by `./mfb spec linker pipeline` and
`./mfb spec linker import-selection`. The thread-specific obligation is one step:
runtime helper imports must be added for built-ins used **inside package IR**, not
just the app's.

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

## See Also

* ./mfb spec linker pipeline — the native link pipeline stages
* ./mfb spec linker import-selection — package call and runtime-helper import resolution
