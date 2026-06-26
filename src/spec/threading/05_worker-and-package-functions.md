# Worker And Package Functions In The Single Codegen

Worker functions are ordinary IR carried in the package's Binary Representation. There is no
separate package binary representation-to-native bridge and no `lower_package_export_function`
path: once the consumer decodes and merges a package's IR, **every** package
function — including thread workers — is lowered through the same
`IR -> NIR -> native` path as the executable's own code.

Consequently package functions automatically get every language feature the
executable path has: full control flow (`IF`/`WHILE`/`FOREACH`/`MATCH`),
function-level and inline `TRAP`, all built-ins, and inline-`TRAP`-on-a-built-in.
A worker body's `CallResult` of a built-in is just an IR node; there is no flat
built-in dispatch to fail on.

Each merged package function still receives a stable internal native symbol so
the linker can resolve cross-package and worker entry points:

```text
_mfb_pkg_<package>_<export>
```

Characters outside ASCII letters, digits, and underscore are sanitized to
underscore. Cross-package calls and worker entry points resolve to these symbols
after the IR merge, with `Nothing` results initialized to the canonical zero
value, the same as for the executable's own functions.
