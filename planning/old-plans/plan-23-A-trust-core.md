# plan-23-A: Trust core — keys, signing, publish, verify

Last updated: 2026-07-04
Overall Effort: x-large (the whole plan-23 feature)
Effort: medium

Part **A** of plan-23 (key & trust model). Delivers the end-to-end trust core:
register with client-held keys, attestation issuance, one-off package signing,
the container v1.0 header, and the publish + verify chains. The design itself
(keys, flows, header, blobs, checks) lives in the index and is not restated
here: [plan-23-key-trust-model.md](plan-23-key-trust-model.md) (§2–§6).

- **Depends on:** nothing — land this sub-plan first.
- **Blocks:** plan-23-B (machines & log), all of plan-10.
- **Spec:** `mfb spec package container-format`, `mfb spec package-manager
  {signing, key-store, repository-protocol}`, `mfb spec package
  verifier-rules`, `mfb spec diagnostics error-codes`.

Decisions honored (index §10): container is **hard v1.0** — `containerMajor=1`,
`containerMinor=0`, readers verify exactly 1.0, **no backwards compatibility**;
`GET /ident` serves the server public key; `mfb pkg validate <pkg>` checks an
*existing* package ("is this package correct?"), it is not a pre-signing step.

## Non-goals

- Machine linking, ident rotation/re-anchor, revocation — plan-23-B.
- Transparency log — plan-23-B (`logEntry` may remain a stub string here).
- `/blob`, `/index`, install-by-name — plan-10-A.
- Release states, signed metadata root — plan-10-C. Orgs/tokens — plan-10-D.

## Context

The tree today: single client-held `auth` keypair aliased as all roles;
`/keys/signing` returns it three ways; `build --sign` signs with it; verify
checks the package signature directly against the `project.json`-pinned key.
Additionally the working tree holds **uncommitted rejected code** (server-held
ident private key in SQLite, `/keys/signing` returning private keys) — index
§8. Everything here replaces that.

## Phases

### Phase A1 — Reset + register + server key

- [ ] Revert the §8 uncommitted working-tree changes in `repository/src/{store,server,client,local}.rs` (the server-held-ident model; verify with `git diff` that only this session's hunks go).
- [ ] `repository/src/crypto.rs`: role-discriminated registration message (`"mfb-repo-register-v1\0" || role || "\0" || owner || "\0" || publicKey`) so an auth proof cannot be replayed as an ident proof; domain-tag helpers for `"MFP-PROOF-v1\0"`, `"MFP-ATTEST-v1\0"`, `"MFP-PACKAGE-v2\0"`.
- [ ] `repository/src/store.rs`: server keypair bootstrap on first run (private key in `server_secrets`-style table, never returned by any route); `register_owner` stores **two** public keys (`role ∈ auth|ident`, both `status='current'`) after verifying both role-separated proofs.
- [ ] `repository/src/server.rs`: `POST /accounts/register` accepts `{owner, authKey, identKey, proofs:{auth,ident}}`; response returns both fingerprints. New `GET /ident` returns the server public key (`server.pub` distribution, index §10.3).
- [ ] `repository/src/local.rs` + `client.rs`: generate both keypairs locally; store as `<owner>.auth.{prv,pub}` and `<owner>.ident.{prv,pub}` (0600); fetch + pin `~/.mfb/<repo-hash>/server.pub` on first contact.
- [ ] Specs: `package-manager/key-store` (new layout, server.pub pin), `package-manager/repository-protocol` (register, `/ident`), `package-manager/signing` (§2 key model).
- [ ] Tests: register persists two keys; a proof for one role replayed for the other is rejected; server private key never appears in any response; `server.pub` pinned on first contact and mismatches refused after.

Acceptance: two-key register with role-separated proofs works end-to-end, the server keypair exists and only its public half is retrievable (via `/ident`), and the rejected uncommitted code is gone.
Commit: —

### Phase A2 — `POST /signing` + `build --sign` + v1.0 header

- [ ] `repository/src/server.rs`: `POST /signing` (authenticated session): request `{owner, ident, version, signingFingerprint}` → verify session owner and `ident` starts with `<owner>#`, record the request, return attestation JSON (index §5) + server signature (`MFP-ATTEST-v1` domain).
- [ ] `src/cli/build.rs`: `--sign <owner>` generates the **one-off signing keypair**, fetches the attestation, mints the proof (index §5) signed by the local ident key (`MFP-PROOF-v1` domain), threads all of it to the package writer, and **discards the one-off private key** after signing. Delete the old `/keys/signing` match logic.
- [ ] Package writer (`src/target` / `src/manifest/package.rs`): emit the index §4 header — `containerMajor=1, containerMinor=0`; full `identKey` + `signingKey`; proof + proofSig; attestation + attestationSig; `packageBinaryHash` (SHA-256, 32 bytes) + `binaryReprLength` inside the signed prefix; prefix signature (`"MFP-PACKAGE-v2\0" || SHA-256(prefix)`). Drop the fingerprint fields and the zeroed-signature whole-file hash. Embedded manifest identity fields updated to match (`identKey`/`signingKey` fingerprints).
- [ ] Executable signing metadata (`mfb-signing-v1` JSON blob) updated to the new key set.
- [ ] Specs: `package/container-format` (full v1.0 rewrite), `package/binary-representation` + `metadata-encoding` (manifest identity fields), `package-manager/signing` (proof/attestation, domains), `package-manager/repository-protocol` (`/signing`), `tooling/cli-reference` (`build --sign`).
- [ ] Tests: attestation refused without a session or for a mismatched owner; built header round-trips with all new fields; proof verifies under the local ident public key; one-off private key absent from disk after build; header states v1.0.

Acceptance: `build --sign` produces a v1.0 package carrying identKey, signingKey, ident-signed proof, and server-signed attestation, with the prefix signature and `packageBinaryHash` correct.
Commit: —

### Phase A3 — Publish checks + verify chain

- [ ] `repository/src/package.rs`: v1.0 reader — verify `containerMajor/Minor == 1/0` (hard, no back-compat), parse the new fields, recompute `packageBinaryHash`, expose the signed prefix for signature checks.
- [ ] `repository/src/server.rs` publish path: the full index §3.4 check chain (session/owner, attestation genuine + ours, ident/version/signingFingerprint/identFingerprint pinning, current name↔ident binding, proof verification, hash + package signature). Each refusal gets a distinct diagnostic.
- [ ] Client verify (`src/cli/build.rs` `classify_installed_package`, `mfb pkg verify`, and `mfb pkg validate <pkg>` per index §10.4): the index §3.5 chain anchored on pinned `server.pub` + `project.json`-pinned `identKey`; `signatureType == 0` stays allowed for local `file://` sources only.
- [ ] Specs: `package/verifier-rules` (§3.5 chain), `package-manager/repository-protocol` (publish checks), `diagnostics error-codes` (one code per §3.4/§3.5 refusal).
- [ ] Tests: end-to-end register → build → publish → verify happy path; the **two-credential negatives** (ident-only forgery fails at publish — no valid attestation; auth-only forgery fails verification — no valid proof); a tampered-field sweep across **every** header field flips the result to Tampered; attestation reused for a different version/package refused; wrong-container-version package refused.

Acceptance: publish enforces the full §3.4 chain, client verification enforces the full §3.5 chain, both negative-credential forgeries fail, and the tampered-field sweep passes.
Commit: —
