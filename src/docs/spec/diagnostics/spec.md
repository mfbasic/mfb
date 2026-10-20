# Diagnostics and Error Codes

This package specifies the compiler diagnostic rule registry, diagnostic rendering, and
the runtime error-code registry. Compiler rule names identify structured build
diagnostics; runtime error codes are integers carried in `Error.code`. A diagnostic's
numeric display code is not necessarily unique. [[src/rules/mod.rs:Rule]]
[[src/rules/table/mod.rs:rules]] [[src/codegen/builtins/errorcode/mod.rs:register]]

The source-level `FAIL`/`TRAP`/`PROPAGATE` model is specified by `./mfb spec language
error-model`; the runtime `Error` representation is specified by `./mfb spec memory
fallible-call-abi`.

## Reading order

- `rule-codes` — the compiler diagnostic registry: the `G-SSS-EEEE` code scheme,
  symbolic names, severities, the full rule table, and the source-context
  rendering format used by `mfb build`.
- `error-codes` — the runtime `errorCode::` registry: the Name→Integer table, the
  code→integer encoding rule, and the subsystem partitioning. The constants are registered in Rust; a drift-guard test compares their names,
  integer values, and order with the topic's Constant Registry table.
  [[src/codegen/builtins/errorcode/mod.rs:table_matches_registry]]

## See Also

* ./mfb spec language error-model — the source-level failure/TRAP/PROPAGATE model
* ./mfb spec memory fallible-call-abi — how an `Error` (code, message, source) is carried at runtime
* ./mfb spec architecture commands — the build/audit commands that emit diagnostics
