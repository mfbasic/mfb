# plan-23-B: Machines & lifecycle + transparency log

Last updated: 2026-07-04
Effort: medium

Part **B** of plan-23 (key & trust model). Delivers the multi-machine account
story (link, revoke), the ident lifecycle (chain rotation, re-anchor), and the
real transparency log. Design in the index, not restated here:
[plan-23-key-trust-model.md](plan-23-key-trust-model.md) (§3.2, §3.6, §7).

- **Depends on:** plan-23-A (keys, register, `/signing`, publish/verify chains).
- **Blocks:** plan-10-C (release states log here; signed metadata references
  the checkpoint), plan-10-D (account actions log here).
- **Spec:** `mfb spec package-manager {key-store, repository-protocol}`,
  `mfb spec tooling cli-reference`, `mfb spec diagnostics error-codes`.

Decision honored (index §10.1): link transport is a **pairing code +
argon2id-encrypted relay blob** (single-use, short TTL, server relays
ciphertext it cannot read); QR/local-network transfer is a later nicety.

## Non-goals

- Release states, signed metadata root — plan-10-C (they consume this log).
- Orgs, publish tokens, ownership transfer — plan-10-D.
- Reaping/rate-limiting of pairing blobs — plan-10-D Phase D2 (hardening).

## Context

After plan-23-A an account works from exactly one machine: keys exist only
where `register` ran, there is no way to add or revoke a machine, no ident
lifecycle, and `logEntry` is still a stub string. This sub-plan closes all of
that; linked machines become full equals (index §2).

## Phases

### Phase B1 — Machine link + revoke

- [ ] `repository/src/server.rs` + `store.rs`: pairing endpoints — old machine posts the argon2id-encrypted ident blob (single-use, short-TTL row); new machine registers its **own auth keypair** to the account and fetches the blob; blob deleted on first fetch and on TTL expiry. Auth-key revocation endpoint (ident-signed request): key `status='revoked'`, sessions on it killed.
- [ ] `repository/src/client.rs` + `local.rs`: `mfb repo link <owner>` (new machine: generate auth keypair, enter pairing code, decrypt and store `<owner>.ident.{prv,pub}`) and the old-machine side (`mfb repo link --start`: display code, encrypt + upload). `mfb machine revoke <fingerprint>` for a lost machine's auth key.
- [ ] After link, the new machine passes the full plan-23-A build/publish path with no involvement from the old machine (machines are equals).
- [ ] Specs: `package-manager/key-store` (pairing transfer), `package-manager/repository-protocol` (pairing + revoke endpoints), `tooling/cli-reference` (`mfb repo link`, `mfb machine revoke`).
- [ ] Tests: linked machine builds + publishes with its own auth key and the copied ident; the relay blob is unreadable without the code, single-use, and expires; a revoked auth key cannot open a session or request attestations; revocation requires an ident signature (auth session alone refused).

Acceptance: a second machine links via pairing code, is a full equal end-to-end, and a lost machine's auth key is cleanly revocable.
Commit: —

### Phase B2 — Ident lifecycle: chain rotation + re-anchor

- [ ] Rotation (`mfb key rotate`): generate a new ident keypair; sign the chain link (`new identKey` signed by the **old** ident, domain-tagged); server verifies the link, re-binds the name, marks the old ident `past`; subsequent attestations name the new ident. Local store updated on every linked machine at next contact (server serves the chain; client follows it).
- [ ] Consumer pin-follow: on install/verify, a package or index naming a *newer chained* ident updates the `project.json` pin automatically with a notice; an ident change with **no chain link** is a hard error.
- [ ] Re-anchor ceremony (total ident loss): registry-operator action (manual, out-of-band verification) binds the name to a fresh ident with no chain link; clients that hold the old pin fail hard with an explicit re-anchor warning telling the user to re-verify out-of-band (index §3.6).
- [ ] Publish-time staleness: an attestation naming a `past` ident is refused (plan-23-A §3.4 step 5 becomes reachable — test it here).
- [ ] Specs: `package-manager/repository-protocol` (rotate + re-anchor), `package-manager/signing` (chain), `tooling/cli-reference` (`mfb key rotate`), `diagnostics error-codes` (re-anchor warning, stale-attestation refusal).
- [ ] Tests: rotation chain verifies and pins follow silently; packages published under the old ident still verify (issued-only facts); post-rotation publish with a stale cached attestation is refused, refetch + rebuild succeeds; a no-chain rebind hard-errors on clients with the old pin.

Acceptance: chain rotation is seamless for consumers, old packages stay valid, stale attestations are refused, and a re-anchor is loud and unmistakable.
Commit: —

### Phase B3 — Transparency log

Subsumes the former plan-10-C C1. RFC 6962 Merkle hashing (index/plan-10
decision) to match CT/Rekor tooling.

- [ ] `repository/src/store.rs`: `log_entries` table — monotonic index, entry kind (registration, name binding/re-bind, attestation request, publish, auth revoke, ident rotation, re-anchor), payload, leaf hash, timestamp; in-DB Merkle tree.
- [ ] Append an entry from every state-changing endpoint (plan-23-A's register/`/signing`/publish and B1/B2's link/revoke/rotate/re-anchor); replace the stub `logEntry` with the real `{index, leafHash}` everywhere it is returned.
- [ ] `GET /log/checkpoint` — signed tree head (size + root hash, server-key-signed); `GET /log/proof/<entry>` — inclusion proof; consistency proofs between two tree sizes.
- [ ] Client pins the last-seen checkpoint (`~/.mfb/<repo-hash>/`) and rejects rollback; `mfb pkg verify` can optionally demand an inclusion proof for the package's publish entry.
- [ ] Specs: `package-manager/repository-protocol` (log endpoints, real `logEntry`), `diagnostics error-codes` (rollback rejection).
- [ ] Tests: inclusion proof verifies against the checkpoint root; consistency proof verifies across appends; a tampered leaf breaks the root; every state-changing endpoint appends exactly one entry; client rejects a shrunken tree.

Acceptance: inclusion + consistency proofs verify, tampering breaks the root, every state change is logged, and checkpoint rollback is rejected client-side.
Commit: —
