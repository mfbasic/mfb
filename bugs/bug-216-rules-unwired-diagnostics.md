# bug-216: three registered diagnostic rules have no emit site (dead registry rows)

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: dead-code

Status: Open

Three compiler-diagnostic rule entries in `src/rules/table.rs` have no emit site
anywhere in the tree, so the conditions they name can never surface their
diagnostic:

- `MFB_PARSE_INVALID_FUNCTION_HEADER` (`:132`)
- `SYMBOL_NOT_VALUE` (`:252`)
- `TYPE_VARIANT_CONSTRUCTOR_AMBIGUOUS` (`:564`)

`grep -rn` for each name returns only `table.rs` — no `show_diagnostic` /
`PendingDiagnostic` / verify registration, no test. These are parser/type-check
conditions expected to be emitted (ambiguous variant constructor, symbol in value
position, invalid FUNC header) whose checks were removed or never wired.

(The seven orchestration codes `BUILD_FAILED`/`PACKAGE_VERSION_UNSUPPORTED`/
`NATIVE_MANIFEST_INVALID`/`VERIFICATION_FAILED`/`TARGET_UNSUPPORTED`/
`LINK_FAILED`/`LOCKFILE_MISMATCH` are also unreferenced but appear to be
deliberate subsystem-partition reserves.)

Fix: wire the missing checks to emit these three names, or drop the stale
entries; if intentionally reserved, add a `// reserved` comment like the existing
retired-code notes.
