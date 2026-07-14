# plan-10-A: Install path (`/blob`, `/index`)

Last updated: 2026-07-04
Overall Effort: x-large (the whole plan-10 feature)
Effort: small

Part **A** of plan-10 (Package Registry Completion). Delivers the install path
that makes published packages retrievable. Overview, gap analysis, sequencing,
and cross-cutting decisions live in the index: [plan-10-repo.md](plan-10-repo.md).

- **Depends on:** [plan-23](plan-23-key-trust-model.md) — **assumed complete**:
  the key/trust model (ident/auth/one-off signing, proof + attestation, v2
  header, publish + install verification chains) all lands there.
- **Blocks:** plan-10-B (resolution), plan-10-C (release states & signed
  metadata), plan-10-D (accounts).
- **Spec:** `mfb spec package-manager` (`src/docs/spec/package-manager/`,
  esp. `repository-protocol` and `key-store`); `mfb spec package
  container-format` (v2 header, per plan-23).

> The former Phase A1 (three-role key model) is **superseded by plan-23** and
> has been removed from this document.

## Context

Nothing can be installed: there is no `/blob`/`/index`, so a package can be
published but never downloaded through the registry. Closes gap rows §2.5
(`/blob`, `/index`), the `mfb pkg add <owner>#pkg` item in §2.1, and the
blob-ordering bug in §2.6.

## Phases

### Phase A2 — Install path: `/blob` and `/index`

Make published packages retrievable.

- [ ] `GET /blob/<hash>` — stream `packages/<hash>.mfp`; 404 if absent; immutable, long-cache headers; verify on read that the recomputed hash matches the path (blob-store corruption defense).
- [ ] `GET /index/<owner>#<package>` (`#`→`%23`) — return the version list: `version, hash, publishedAt, state, abiIndex, logEntry` per version, plus the owner's current `identKey` and its server-signed name binding so a first `add` can pin the ident (plan-23 §3.5 anchor). No key-rotation fields — one-off signing keys have no status/window (plan-23). Add `Store::list_package_versions`.
- [ ] Fix the publish ordering bug: write the blob inside/after the committed transaction, or write to a temp file and rename only on commit.
- [ ] Client `mfb pkg add <owner>#pkg[@ver]`: hit `/index`, pick latest/requested, pin the owner's `identKey` into `project.json` on first add, `GET /blob/<hash>`, then run the full plan-23 §3.5 verification chain (pinned server key → attestation → pinned ident → proof → package signature → `packageBinaryHash`), install into the project package dir, append to `project.json`; keep the existing `file://` path as a `source: "file:"` special case.
- [ ] Tests: publish then `GET /blob/<hash>` returns identical bytes; `GET /index` lists the version with `publishedAt`; `add <owner>#pkg` pins the ident, installs, and verifies the full chain; a tampered blob is rejected (hash and/or signature); a package whose attestation names a different ident than the pinned one is rejected.

Acceptance: a published package is retrievable by blob, listed by index, installs + verifies via `add` through the plan-23 chain, and a tampered blob is rejected.
Commit: —
