# bug-900: a runtime helper called only in a module-level initializer fails the build with "NIR runtime call requires undeclared helper"

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/general/global-init-only-runtime-helper-rt (proposed)

A valid program that calls a helper-backed built-in (`os::getEnv`, `fs::exists`,
`fs::readText`, ...) only inside a module-level `LET`/`MUT` initializer fails to build.
The error comes from inside the compiler and has no source location:

```
error: NIR runtime call requires undeclared helper 'os'
```

The build succeeds as soon as any function body also calls the same package. That
makes the failure depend on unrelated code elsewhere in the program.

**Correct behavior:** helpers used only in global initializers are counted as
required, so the program builds and runs. The spec says `required_helpers` "walks
the IR".

References:

- `src/docs/spec/memory/07_runtime-helper-abi.md` § Required-Helper Analysis:
  "`required_helpers(ir)` walks the IR and returns the de-duplicated set of helper
  families a program actually needs"; § Validation Invariants: "used but not
  declared -> NIR runtime call requires undeclared helper".
- `src/docs/spec/memory/08_program-startup.md` step 3 (the global initializer is a
  normal part of the program).
- Found during spec hardening batch B09 (runtime-helper ABI / startup).

## Failing Reproduction

```
mfb init nv
# nv/src/main.mfb
IMPORT io
IMPORT os

LET H AS String = os::getEnv("HOME")

FUNC main AS Integer
  io::print(H)
  RETURN 0
END FUNC

mfb build nv -q
```

- Observed: `error: NIR runtime call requires undeclared helper 'os'` (no file or
  line), and no executable is written. The same happens with
  `LET H AS Boolean = fs::exists("/tmp")`, which reports `... helper 'fs'`.
- Expected: `Wrote executable to nv/build/nv.out`. Running it prints `$HOME`.

Contrast: change `main` to `io::print(H & os::getEnv("USER"))`. It then builds and
prints `/Users/<me><me>`. In t7 (B09 scratch), `LET CFG AS String =
fs::readText(...)` built only because `main` also called `fs::deleteFile`.

## Root Cause

`src/target/shared/runtime/usage.rs:required_helpers` scans only
`for function in &ir.functions { push_op_helpers(&function.body, …) }`. It never
looks at `ir.bindings` (`src/ir/types.rs:IrProject::bindings`), where the global
initializers are. The validator
`src/target/shared/validate/mod.rs:validate_nir` walks the lowered NIR. In the NIR
the initializer is an ordinary function, so the validator sees the `os.getEnv` call,
finds no matching declaration, and returns the hard error.

## Goal

- `required_helpers` also covers every binding initializer, in the executable and in
  merged package IR. The repro builds and prints `$HOME`.
- The two sets match in both directions, so no "declared but unused" error
  appears either.

### Non-goals (must NOT change)

- The exact-equality validator. It caught this, and it must not be loosened to
  a subset check.

## Blast Radius

- `required_helpers`: fixed here.
- Other IR walks that list `ir.functions` and skip `ir.bindings` (thread-family
  detection for the arena-teardown deferral in
  `src/codegen/engine/builder/mod.rs:lower_module_for_platform`, stdin/term/mouse
  usage gates) may miss uses in global initializers in the same way. Grep each
  `program_uses_*` helper.

## Partly fixed in batch 21 (2026-10-11)

bug-1030's fix made `required_helpers` walk `ir.bindings` (its own reproduction needed it). Measured
on 2026-10-11 with the batch 21 compiler: the program above builds and prints `$HOME`, and the
`fs::exists("/tmp")` form prints `TRUE`; main's compiler (dcf790e9a) still fails both with
`requires undeclared helper`. Not checked, so still open: merged package IR, and the other IR walks
listed under "Blast Radius".

