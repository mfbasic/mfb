# Auditability

Because ordinary calls auto-unwrap and auto-propagate, fallible control flow is
otherwise invisible in source — there is no `try`/`catch` keyword marking where a
call may fail. Surfacing that hidden control flow is a first-class goal of the
toolchain. This topic is the rationale and the catalogue of what must be surfaced;
the concrete commands that implement it are owned by their own topics — `mfb audit`
by `./mfb spec tooling audit-format`, `mfb fmt` by `./mfb spec tooling fmt`, and the
full CLI surface by `./mfb spec tooling cli-reference`.

## What must be surfaced

The build-time audit (and a future language server) surface the following:

- Every fallible call site, including calls hidden inside expressions and argument
  lists.
- Each auto-propagation edge from a fallible call to the enclosing `TRAP` or
  function return.
- Each `TRAP` recovery path, including whether it `RETURN`s, `PROPAGATE`s, or
  replaces the error with `FAIL`.
- Scopes that hold live resource bindings across fallible calls, and the lexical
  drop-close edges that release those resources on each exit path, together with the
  drop-close failure metadata rule (see `./mfb spec language resource-management`).
- All native binding packages, linked native libraries, declared symbols, ABI
  mappings, and native resource close functions used by a build.
- Package permissions and host capabilities when a standard or native package
  requires filesystem, network, terminal, threads, process, environment, clock,
  randomness, or native-library access. (`./mfb spec tooling audit-format`
  catalogues the finding for each.)

**Not implemented**, and stated here as a design target rather than a shipped
analysis: confusing identifier similarity in dense or security-sensitive code. In
the current ASCII-only identifier set that would be case-only near-collisions; if
non-ASCII identifiers are ever enabled it would also cover Unicode normalization,
case-fold, script-mixing, and confusable-character collisions. `mfb audit` emits
no such finding today.

Fallible-call, propagation, `TRAP`, permission, native-link, and resource-cleanup
metadata are carried in `.mfp` packages when exported APIs contain or expose those
behaviors, so an importer's audit sees through the package boundary.

## Language-server design target

A language-server entry point (`mfb lsp`) is intended but **not yet implemented**:
it is absent from the CLI. The editor-side diagnostics — marking every fallible call
site, propagation edge, `TRAP` recovery, resource move/use-after-move, native-link,
permission, version-conflict, lockfile, and identifier-similarity finding — are the
design target for that server and editor tooling, not a feature the present
toolchain ships. The build-time half of this vision is implemented today by
`mfb audit`.

## See Also

* ./mfb spec tooling audit-format — `mfb audit` output, flags, and exit codes
* ./mfb spec tooling fmt — the `mfb fmt` formatter contract
* ./mfb spec tooling cli-reference — the complete command-line surface
* ./mfb spec language error-model — the implicit-failure model this surfaces
* ./mfb spec language resource-management — resource cleanup and drop-close edges
