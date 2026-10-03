# bug-750: a package that imports `io` exports `io`'s `Key`, `KeyName` and `KeyEvent` types

Last updated: 2026-10-03
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: `src/binary_repr/tests/writer_tests.rs:an_imported_builtin_type_is_not_exported`; tests/syntax/packages/package-import-as (acceptance `.mfp` golden)

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

## Resolution (2026-10-03)

**Scope was wider than filed.** Every builtin package with types leaked, not only
`io`. Probe: a one-function package that does `IMPORT <pkg>` and nothing else
exports `datetime.*` (9 types), `net.*` (4), `json.*` and `encoding.Codepage`,
`csv.*`, `regex.*`, `color.*`, `term.*`, `big.*`, `crypto.*`. A `udp` importer
exports `net.Url`, `net.Address` and the other `net` types, which is bug-466's
"an unrelated import makes a foreign record's fields readable" defect reached
through a package.

**Why the fixture passed.** a76df3d54 ("resync 31 stale committed .mfp copies")
re-baselined `package_import_as.mfp` with the leak in it (`types: 3`,
`exports: 5`).

**Root cause.** `TypeTable::reserve_source_type` marked every `ir.types` entry
whose visibility is `export`. Builtin types reach a package's IR from the builtin's
own injected source, where they are declared `EXPORT`.

**Fix.** `reserve_source_type` never marks a builtin-owned type
(`ir::is_builtin_type_name`: package-qualified or sigil-internal) as an export.
The type stays in the type table, which consumers de-duplicate by its
compiler-owned spelling. Spec: `mfb spec package type-table`.

**Test corrected (four questions).** `packages/timezones/runtime-smoke.sh`
(afbde58a3, plan-135-D) read `z.dateTime.date` without `IMPORT datetime`, and its
comment claimed that should work. It worked only because timezones exported
`datetime.DateTime` through this leak. The claim is disproved by
`mfb spec language modules-and-packages` ("Imports remain non-transitive for
names") and by bug-466's contract (a builtin record's fields need the file's own
import). Fix: the consumer adds `IMPORT datetime`, and the round-trip assertions
are unchanged. A separate commit also repaired the cli, logger and timezones smoke
manifests, which predated `direct`/`requiredBy`.

Proof: the new unit test fails on main
(`left: ["io.KeyEvent", "datetime.Weekday", "Mode"]`) and passes with the fix.
`mfb pkg info` of the fixture now shows `exports: 2` (`byteLenAlias`,
`flushStreams`). The committed `.mfp` copies of packages that import a typed
builtin are regenerated at the bug-staging batch gate
(`scripts/sync-package-mfp.sh`).
