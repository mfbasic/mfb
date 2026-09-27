# bug-750: a package that imports `io` exports `io`'s `Key`, `KeyName` and `KeyEvent` types

Last updated: 2026-09-27
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/syntax/packages/package-import-as (acceptance fixture, fails today)

Since bug-699 added `io::readKey` and its builtin types (`io.Key` record, `io.KeyName`
and `io.KeyEvent` enums), a user **package** that merely does `IMPORT io` writes those
three builtin types into its own `.mfp` type table and export list, as if it declared
and exported them. No other builtin package's types leak this way. The single correct
behavior: a package exports exactly what its own source `EXPORT`s (and re-exports it
names); importing a builtin package adds nothing to its exports.

References:

- Found while landing plan-158 (its final acceptance run on main `b1a563acd`).
- `mfb spec language modules-and-packages` (what a package exports).
- bug-699 (introduced the io key types), bug-390 (foreign/re-exported type entries).

## Failing Reproduction

On main at `b1a563acd` (and every later commit until fixed):

```
cargo build --release
bash scripts/test-accept.sh target/release/mfb /tmp/accept-io "package-import-as"
# -> mismatch: syntax/packages/package-import-as/package_import_as.mfp
target/release/mfb pkg info /tmp/accept-io/syntax/packages/package-import-as/package_import_as.mfp
# -> types: 3, exports: 5, including TYPE io.Key, ENUM io.KeyName, ENUM io.KeyEvent
```

The committed golden (correct) has `types: 0`, `exports: 2` — the fixture's own
`EXPORT FUNC byteLenAlias` / `EXPORT SUB flushStreams`
(`tests/syntax/packages/package-import-as/src/lib.mfb`, which only does
`IMPORT io AS console` / `IMPORT strings AS text`).

## Notes for the fix

- The export list is assembled into `abi.exports` and the type table in
  `src/binary_repr/{writer.rs,sections.rs,builder.rs}`; `src/ir/package.rs` feeds it.
  The io key types are the first builtin types a package can see that are declared
  in a way the writer treats as project-owned.
- An interrupted first attempt (touching `src/binary_repr/{mod,reader,sections,
  writer}.rs`, `src/binary_repr/tests/writer_tests.rs`, `src/ir/{mod,package}.rs`) was
  saved, unverified and incomplete, at `/tmp/p158/io-export-partial.patch` on the
  machine that found this; treat it as notes, not a fix.
- RED test first: a unit test that builds a package importing only `io` and asserts
  no `io.*` type is in its type table or exports; then the acceptance fixture above
  must pass byte-for-byte against its committed golden.
