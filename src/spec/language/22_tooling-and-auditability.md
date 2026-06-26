# 22. Tooling And Auditability

The compiler and language server must make fallible control flow visible even though ordinary calls auto-unwrap and auto-propagate.

Required diagnostics and tooling metadata:

- Mark every fallible call site in editor diagnostics or semantic tokens, including calls hidden inside expressions and argument lists.
- Show each auto-propagation edge from a fallible call to the enclosing `TRAP` or function return.
- Show each `TRAP` recovery path, including whether it `RETURN`s, `PROPAGATE`s, or replaces the error with `FAIL`.
- Report scopes that hold live resource bindings across fallible calls, and surface the lexical drop-close edges that release those resources on each exit path, together with the drop-close failure metadata rule from §15.
- Surface all native binding packages, linked native libraries, declared symbols, ABI mappings, and native resource close functions used by a build.
- Surface package permissions and host capabilities when a standard or native package requires filesystem, network, process, environment, clock, randomness, or native-library access.
- Lint dense or security-sensitive code for confusing identifier similarity. In the current ASCII-only identifier set this includes case-only near-collisions; if non-ASCII identifiers are ever enabled, it also includes Unicode normalization, case-fold, script-mixing, and confusable-character collisions.
- Include fallible-call, propagation, `TRAP`, permission, native-link, and resource-cleanup metadata in `.mfp` packages when exported APIs contain or expose those behaviors.

The toolchain must provide an audit command:

```text
mfb audit [--format text|json] [--locked] [path]
```

`mfb audit` reports fallible call sites, auto-propagation paths, `TRAP` recovery paths, resource cleanup behavior, native links, package permissions, dependency versions, lockfile mismatches, and verifier status. `--locked` requires the resolved dependency graph to match `mfb.lock`.

Additional required tooling commands:

```text
mfb fmt [--check] [--indent N] [path]
mfb test [--filter pattern] [--locked] [path]
mfb lsp
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

`mfb test` discovers exported or private zero-argument `SUB` declarations whose names start with `test` in files included by the `project.json` test source entries. A test succeeds when it completes without failing and fails when it produces an error. Test builds use the same package resolver, verifier, resource rules, and audit metadata as executable builds.

`mfb lsp` starts the language-server protocol implementation. It must expose diagnostics for fallible calls, auto-propagation paths, `TRAP` recovery, resource moves/use-after-move, unsafe or invalid native links, permissions, package-version conflicts, lockfile mismatches, dense security-sensitive lines, and identifier near-collisions.
