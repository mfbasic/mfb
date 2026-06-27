# Diagnostics and Error Codes

The two code registries the toolchain exposes as stable, externally-observable
contracts: the **compiler diagnostics** raised during a build, and the **runtime
error codes** carried in an `Error` value at run time. Both are partitioned,
numbered namespaces a consumer (an IDE, a CI gate, a reimplementation) can depend
on, so they belong in the specification rather than only in code.

These are *contracts*, not source syntax — the language-level error *model*
(`FAIL`/`TRAP`/`PROPAGATE`, auto-unwrap/auto-propagate) is specified by
`./mfb spec language error-model`, and the runtime `Error` value's representation
by `./mfb spec memory fallible-call-abi`. This package owns only the code
registries and how diagnostics are rendered.

## Reading order

- `rule-codes` — the compiler diagnostic registry: the `G-SSS-EEEE` code scheme,
  symbolic names, severities, the full rule table, and the source-context
  rendering format used by `mfb build`.
- `error-codes` — the runtime `errorCode::` registry: the Name→Integer table, the
  code→integer encoding rule, and the subsystem partitioning. The build generates
  the constants from `specifications/error_codes.md`; this topic is the embedded,
  human-facing mirror of that registry, kept in sync by a drift-guard test.

## See Also

* ./mfb spec language error-model — the source-level failure/TRAP/PROPAGATE model
* ./mfb spec memory fallible-call-abi — how an `Error` (code, message, source) is carried at runtime
* ./mfb spec architecture commands — the build/audit commands that emit diagnostics
