# plan-10-B: Resolution

Last updated: 2026-07-04
Effort: medium

Part **B** of plan-10 (Package Registry Completion). Makes resolution real: the per-symbol ABI index
and the client resolver + `mfb.lock` that consumes it. Overview and gap analysis:
[plan-10-repo.md](plan-10-repo.md).

- **Depends on:** [plan-23](plan-23-key-trust-model.md) (**assumed complete** — key/trust model,
  v2 header) and plan-10-A (install path: `/index`, `/blob`).
- **Spec:** `mfb spec package-manager` (`src/docs/spec/package-manager/`, esp. `repository-protocol`);
  `mfb spec package container-format` (v2 header per plan-23); `mfb spec tooling lockfile`
  (`src/docs/spec/tooling/03_lockfile.md`).

## Context

`publish` returns `abiIndex: {}` today and there is no resolver or `mfb.lock`, so superset
substitution, diamond-conflict diagnostics, and `check-abi` cannot exist. Closes gap rows §2.4
(`ABI_INDEX` computed/embedded/stored/served, resolver, diamond diagnostics, `mfb.lock`,
`check-abi`) and the `mfb pkg install` / release-state eligibility items in §2.1.

## Phases

### Phase B1 — ABI index

Compute and serve one hash per exported symbol; unlocks resolution and `check-abi`.

- [ ] Compiler: compute `ABI_INDEX` — one hash per exported symbol over its full public shape (functions, records, unions, enums, constants, globals, native wrappers, resource behavior, effect flags) per §8.2; embed as a new `.mfp` metadata section (amend `mfb spec package container-format`). The section lives in `packageBinaryRepr`, so it is covered by plan-23's `packageBinaryHash` and the package signature automatically.
- [ ] `repository/src/package.rs` parses the section; `/validate` and `/publish` return the real `abiIndex` and `Store` persists it on the version row.
- [ ] `/index` serves the stored `abiIndex`.
- [ ] `mfb pkg check-abi` diffs the working tree's `ABI_INDEX` against the latest published version and names changed/dropped symbols.
- [ ] Tests: golden ABI hashes stable across rebuilds; adding an export is a superset; changing a signature changes exactly that symbol's hash; `check-abi` names the changed symbol.

Acceptance: ABI hashes are stable/superset-correct, a signature change is isolated to its symbol, and `check-abi` names it.
Commit: —

### Phase B2 — Resolver + lockfile

The §8.3 client resolver and `mfb.lock`. Depends on B1.

- [ ] Implement the §8.3 algorithm in the client: single-dep latest-compatible (`ABI_INDEX(V) ⊇ ABI_INDEX(anchor)`), diamond union with precise conflict diagnostics naming requirers and disagreeing symbols, exact `--pin` bypass.
- [ ] Honor release-state eligibility (`available`/`deprecated` eligible, `yanked` pin-only, `blocked`/`legal-tombstoned` excluded).
- [ ] Write `mfb.lock` per `lockfile.md` (selected/requested versions, hashes, ABI metadata, checkpoint, root/snapshot/timestamp versions). Key metadata per plan-23: the pinned owner `identKey` fingerprint per dependency and the registry `repoFingerprint` — no signing-key status/rotation fields (one-off keys have none).
- [ ] `mfb pkg install`: with a current lock, fetch by hash only — never resolve. `mfb pkg update`: explicit re-resolution producing a reviewable lock diff.
- [ ] Tests: re-resolve is byte-identical; diamond conflict names both requirers; a patch release is selected as a compatible substitute; locked install does no index lookups.

Acceptance: deterministic re-resolve, diamond-conflict diagnostics, compatible-substitute selection, and lookup-free locked install.
Commit: —

## Decision

*To confirm (§5.2):* ABI section placement in the `.mfp` container needs a container-format spec
amendment before the compiler emits it (Phase B1); it must sit inside `packageBinaryRepr` so
plan-23's `packageBinaryHash`/signature coverage holds without header changes.
