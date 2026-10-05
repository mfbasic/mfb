# bug-903: a user `FUNC __mfb_init_globals_<project>` collides with the compiler's global initializer — unlocated internal error

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/syntax/functions/reserved-global-initializer-name-valid (proposed)

The compiler names a project's module-level initializer `__mfb_init_globals_<fragment>`
(`src/target/shared/nir/symbols.rs:global_initializer_name`) and puts it in the
ordinary user function namespace (`_mfb_fn___5F_5Fmfb_5Finit…`). That name is a
valid identifier, `[A-Za-z_][A-Za-z0-9_]*`. A project with globals that also
declares a function with exactly that name fails to build with an internal error
that gives no file or line.

**Correct behavior:** compiler-internal functions take names user code cannot type,
for example the reserved `#` sigil or the `$` sigil used for other internal
functions. The user function then compiles like any other.

References:

- `src/docs/spec/language/02_lexical-structure.md`: `#` "is rejected in user
  identifiers ... precisely so the compiler can use it as the untypeable internal
  sigil".
- `src/docs/spec/threading/05_worker-and-package-functions.md`: "Compiler-internal
  sigil-prefixed functions use a reserved `_mfb_ifn_` namespace instead, which user
  and package functions can never reach". The global initializer is
  compiler-internal but has no sigil, so this protection does not apply to it.
- Found during spec hardening batch B09 (program startup: disassembly of the entry
  showed `bl _mfb_fn__5F_5Fmfb_5Finit_5Fglobals_5Ffr`).

## Failing Reproduction

```
mfb init gx
# gx/src/main.mfb
IMPORT io
LET X AS Integer = 5
FUNC __mfb_init_globals_gx AS Integer
  RETURN 1
END FUNC
FUNC main AS Integer
  io::print(toString(X))
  RETURN 0
END FUNC

mfb build gx -q
```

- Observed: `error: NIR function '__mfb_init_globals_gx' is declared more than once`
  (no location), and no executable.
- Expected: the build succeeds and running prints `5`.

Contrast: `FUNC mfb_init_globals_gx` (one character different) builds and prints `5`.

## Root Cause

`global_initializer_name` builds an identifier-shaped name, so the NIR uniqueness
check in `src/target/shared/validate/mod.rs:validate_nir` (`unique_function_names`)
is the first place that sees the clash.

## Goal

- The internal initializer name cannot be spelled in source (use a sigil, or the
  `_mfb_ifn_` namespace). The repro builds.

### Non-goals (must NOT change)

- The order or semantics of global initialization.

## Blast Radius

- `global_initializer_name`: fixed here.
- Any other identifier-shaped compiler-generated NIR function names: grep `format!("__mfb_`
  in `src/target` and `src/codegen`.
