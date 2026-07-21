# Toolchain Contracts

The `mfb` toolchain's externally-observable contracts that are **not** language
semantics or on-disk package bytes: the project manifest, how source files are
selected, the lock file, the audit output, the formatter's normalization, the
HTML documentation renderer, and the CLI surface itself. These are stable formats
and behaviors a consumer (an IDE, a CI gate, a reimplementation) depends on, so
they are specified here as reimplementable contracts rather than left as prose in
the command help.

This package owns the *formats and algorithms*. The build *pipeline* those
commands drive is `./mfb spec architecture`; the language they compile is
`./mfb spec language`; the `.mfp` byte format is `./mfb spec package`; the
registry/signing workflow behind `repo publish`/`repo`/`build --sign` is
`./mfb spec package-manager`.

## Reading order

- `project-manifest` — the `project.json` schema and validation rules.
- `source-selection` — the glob algorithm that turns `sources[]` into the `.mfb`
  input set.
- `lockfile` — the `mfb.lock` format, the `projectHash` algorithm, and `--locked`.
- `audit-format` — the `mfb.audit.v1` JSON schema, the `AUDIT-*` finding
  catalogue, and the analysis model behind `mfb audit`.
- `fmt` — the `mfb fmt` normalization rules.
- `doc-html` — the `mfb doc` / `pkg doc` HTML rendering model.
- `cli-reference` — every command, flag, and exit code, the `pkg info` output,
  and the embedded `spec`/`man` terminal rendering.
- `auditability` — the rationale and catalogue for surfacing the language's
  implicit fallible control flow, plus the language-server design target.

## See Also

* ./mfb spec architecture commands — build modes and the build-flag semantics
* ./mfb spec architecture flows — the end-to-end build pipeline
* ./mfb spec diagnostics rule-codes — the diagnostics these commands emit
* ./mfb spec package-manager — registry protocol, signing, and key storage
