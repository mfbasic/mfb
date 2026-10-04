# bug-802: macOS executables list an imported libSystem symbol once per importing helper

Last updated: 2026-10-03
Effort: small (fix) + golden churn (every macOS native artifact that imports a shared symbol)
Severity: LOW
Class: Codegen quality (symbol table bloat)

Status: Open
Regression Test: none yet

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
