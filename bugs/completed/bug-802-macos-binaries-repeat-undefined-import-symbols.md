# bug-802: macOS executables list an imported libSystem symbol once per importing helper

Last updated: 2026-10-03
Effort: small
Severity: LOW
Class: Codegen quality (symbol table bloat)

Status: Fixed (2026-10-03, bug-staging-6)
Regression Test: `tests/link/link_macos_unique_imports.rs`

Found while writing bug-800's regression test.

## Reproduction (bug-staging-2 `target/release/mfb`, macos-aarch64)

```basic
IMPORT io
IMPORT fs
FUNC main() AS Integer
  io::print(fs::canonicalPath("."))
  io::print(toString(fs::isWithin(".", ".")))
  RETURN 0
END FUNC
```

`nm -u build/<name>.out` lists `___error` three times and `_realpath` twice.
`nm -u build/<name>.out | sort | uniq -d` → `___error _access _realpath _signal _stat
_write`. A binary that `ld64` links lists each undefined symbol once.

## Likely cause

`src/target/macos_aarch64/plan.rs` pushes one `PlatformImport` per runtime helper that
needs a symbol (`canonicalPath` and `isWithin` each push `_realpath` and `___error`),
and the Mach-O writer emits one undefined `nlist` per entry instead of one per symbol.

## Fix

Deduplicate imports by `(library, symbol)` before the Mach-O symbol table is laid
out, keeping every `required_by`. Expect every macOS `.ncode`/`.ncodesum` golden whose
program imports a shared symbol to change; localize one to confirm only the import
table moved, then regenerate.

## Resolution (2026-10-03)

`arch/encode_plan.rs:encode_plan` keeps the first `EncodedImport` of each
`(library, symbol)`, in order. The fix is in the shared encoder, so ELF, Mach-O
and PE all get one stub, GOT or IAT slot per import. The `.ncode` plan dump is
unchanged, so no golden moves (the predicted churn did not happen: goldens record
the plan, not the linked image).

Verified: the reproduction's `nm -u | sort | uniq -d` is empty and the program runs.
The linker (304) and encoder (258) unit tests pass. `rt-behavior/fs` runs 61/61 on
linux-aarch64 (box 2226), `rt-behavior/process` 15/15 on linux-riscv64 (2229), and
`rt-behavior/fs` on Windows hardware (206) gives the same 53/61 with the same
Unix-only failures as main. `link_macos_unique_imports` fails on main (duplicates)
and passes now. Spec: `linker/03_import-selection.md`.
