# 22. Tooling And Auditability

Because ordinary calls auto-unwrap and auto-propagate, fallible control flow is otherwise invisible in source. The toolchain and a future language server are designed to surface it. The `mfb audit` command (below) implements the build-time half of this; the editor/LSP half is a design target (see the "Not yet implemented" note).

Designed diagnostics and tooling metadata:

- Mark every fallible call site in editor diagnostics or semantic tokens, including calls hidden inside expressions and argument lists.
- Show each auto-propagation edge from a fallible call to the enclosing `TRAP` or function return.
- Show each `TRAP` recovery path, including whether it `RETURN`s, `PROPAGATE`s, or replaces the error with `FAIL`.
- Report scopes that hold live resource bindings across fallible calls, and surface the lexical drop-close edges that release those resources on each exit path, together with the drop-close failure metadata rule from §15.
- Surface all native binding packages, linked native libraries, declared symbols, ABI mappings, and native resource close functions used by a build.
- Surface package permissions and host capabilities when a standard or native package requires filesystem, network, process, environment, clock, randomness, or native-library access.
- Lint dense or security-sensitive code for confusing identifier similarity. In the current ASCII-only identifier set this includes case-only near-collisions; if non-ASCII identifiers are ever enabled, it also includes Unicode normalization, case-fold, script-mixing, and confusable-character collisions.
- Include fallible-call, propagation, `TRAP`, permission, native-link, and resource-cleanup metadata in `.mfp` packages when exported APIs contain or expose those behaviors.

The toolchain provides an audit command:

```text
mfb audit [--format text|json] [--locked] [path]
```

`mfb audit` reports fallible call sites, auto-propagation paths (`trap`/`return`),
`TRAP` recovery classifications, resource cleanup behavior (including native
resource types and may-fail close edges), package permissions (host
capabilities), dependency versions, lockfile mismatches, and per-package verifier
status. `--format` accepts `text` (default) or `json` (both `--format json` and
`--format=json` forms); `[path]` defaults to `.`. `--locked` elevates a stale or
missing lockfile to an error: the lockfile is `mfb.lock`, and the check compares
the lockfile's recorded `projectHash` against a hash over the current
`project.json` package requests (it does not yet compare the full resolved
dependency graph — resolved versions, content hashes, or transitive deps). The
exit code is `0` when clean, `1` on error-severity findings, `2` on a usage
error, and `3` on unreadable or malformed input. (Native-link reporting is a
declared section but the current collector leaves it empty; native `LINK`
metadata surfaces instead through the native-resource entries.) [[src/audit/mod.rs:run]]

The formatter command:

```text
mfb fmt [--check] [--indent N] [path]
```

`mfb fmt` applies the standard formatter to every `.mfb` file selected by the
project (or to a single `.mfb` file). Like `mfb build` and `mfb doc`, `path`
defaults to the current directory. The formatter normalizes block indentation
and keyword capitalization, leaving comments, blank lines, and string contents
untouched. `DOC` and `LINK` blocks are re-indented from their own nesting (a
`DOC` body one level under `DOC`, with `EXAMPLE` source one level deeper; a
`LINK` body following its `FUNC`/`FREE` nesting), but their text and casing are
preserved — `DOC` bodies are free-form prose and `LINK` `ABI` lines use a
contextual lowercase `return`. `--indent N` sets the indent width in spaces
(default `2`). `--check` writes nothing and exits with a toolchain diagnostic
when any file is not already formatted.

The complete current command set is discoverable via `mfb help` and documented by
`./mfb spec architecture commands`. The `audit`, `fmt`, and `doc` commands above
are described here because they surface auditability as a language property; the
remaining build, package-management, and repository commands are owned by that
architecture topic.

> **Not yet implemented.** A dedicated test runner (`mfb test`) and a
> language-server entry point (`mfb lsp`) are intended but absent from the
> current CLI. The language-server diagnostics described above — marking every
> fallible call site, propagation edge, `TRAP` recovery, resource
> move/use-after-move, native-link, permission, version-conflict, lockfile, and
> identifier-similarity finding — are the design target for an LSP and editor
> tooling, not a feature the present toolchain ships.

## See Also

* ./mfb spec architecture commands — full CLI command set and build modes
* ./mfb spec language documentation — `DOC` blocks and `mfb doc`
