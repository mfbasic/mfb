# Specifications (`mfb spec`)

The compiler's specification lives in `src/docs/spec/**` and is embedded in the binary
(`mfb spec`), version-locked to the code: the spec you read always matches the
binary you have. It is the **single source of truth** for every externally
observable compiler/language/format/ABI contract, and it must stay accurate to the
compiler **as-is** at all times.

**The rule: any compiler change that adds, removes, or changes an observable
contract updates the owning `src/docs/spec` topic in the same change.** This is part of
the Hard Completion Gate, not optional cleanup — a change that leaves the spec
stale is not done. Prefer an accurate stub over a missing or wrong topic. Contracts
that require a spec update include: language surface and type rules; IR/NIR op or
value forms and lowering behavior; the `.mfp` byte format; memory layouts, the
native calling convention, runtime-helper ABI, and program startup; AArch64
encoding; diagnostics and error codes; CLI/manifest/lockfile/audit/fmt/doc output;
the registry/signing protocol; threading; Unicode; and standard-package semantics.

Find the owning topic with `mfb spec` (or `mfb spec <package> --all`). Current
packages: `architecture` (the compiler pipeline/passes/CLI), `language` (source
semantics), `memory` (runtime value layouts + native ABI), `linker`, `package`
(`.mfp` byte format), `threading`, `diagnostics` (rule + error-code registries),
`tooling` (manifest/source-selection/lockfile/audit/fmt/doc/CLI), `package-manager`
(registry/keys/signing), `unicode`, `app` (`-app` GUI runtime), `stdlib` (regex/
datetime/csv/json/http/url/PCG64 models).

Conventions when editing the spec:

- **Single source of truth.** Each fact has one canonical topic. Other topics give
  a short summary and a `./mfb spec <package> <topic>` (or `./mfb man <package>`)
  link — never a second full copy. Small inlined facts are fine; a rats-nest of
  references and duplicated bodies is not.
- **Provenance.** Back a non-obvious implementation claim (magic number, offset,
  ABI register, enum variant, capability list, pass ordering) with an invisible
  `[[src/file.rs:Symbol]]` citation at claim-cluster granularity — symbol-preferred,
  `[[src/file.rs:line]]` only where no symbol fits. Grep-confirm the symbol exists
  before citing. The renderer strips `[[ ]]` everywhere (including headings), so
  they never display in `mfb spec`/`man` output but keep claims traceable for
  reviewers. Do not add non-verifiable claims.
- **Adding a topic / package.** A new topic is `NN_slug.md` beside the package's
  `spec.md` (auto-discovered, ordered by the `NN` prefix). A new package is a
  directory with a `spec.md` overview plus its `## See Also`; add its name to
  `PACKAGE_ORDER` in `src/docs/spec/mod.rs`. Update the package overview's reading-order
  prose when adding a topic.
- **The error-code registry is build input.** `build.rs` generates the
  `errorCode::` constants directly from the **Constant Registry** table in
  `src/docs/spec/diagnostics/02_error-codes.md` (`mfb spec diagnostics error-codes`),
  asserting that hyphen-stripping each code equals its integer column; a
  `#[cfg(test)]` drift guard (`table_matches_registry`) enforces the match. Edit
  that table for any runtime error-code change. The legacy external specs
  (`mfbasic.md`, `error_codes.md`, `standard_package.md`, `project.md`, …) are
  archived under `planning/old-moved-to-src-spec/` and superseded by the
  embedded topics — update the `mfb spec` topic, not those.

Verify spec changes: `cargo build` (regenerates the embedded table; if a brand-new
file is not picked up, `touch build.rs` and rebuild), `cargo test --bin mfb spec`,
and confirm `mfb spec <package> --all` renders with no leaked `[[` markers and that
every `./mfb spec`/`./mfb man` link target and `[[…:Symbol]]` citation resolves.
