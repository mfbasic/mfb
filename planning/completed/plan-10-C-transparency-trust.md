# plan-10-C: Release states & signed metadata

Last updated: 2026-07-04
Effort: medium

Part **C** of plan-10 (Package Registry Completion). Adds maintainer release
states and the signed root/snapshot/timestamp metadata chain on top of the
plan-23 trust base. Overview and gap analysis: [plan-10-repo.md](plan-10-repo.md).

- **Depends on:** [plan-23](plan-23-key-trust-model.md) — **assumed complete**:
  the transparency log (plan-23-B) that every action here logs to, the server
  (attestation) key that the root delegates, and the key lifecycle (ident
  rotation/re-anchor, auth revocation) that this sub-plan no longer contains.
  Also plan-10-A (`/index` exists to carry release states).
- **Spec:** `mfb spec package-manager` (`src/docs/spec/package-manager/`, esp.
  `signing`).

> **Superseded by plan-23 and removed from this document:**
> - the former Phase C1 (transparency log) — delivered by **plan-23-B**
>   (RFC 6962 Merkle log, checkpoints, inclusion/consistency proofs).
> - the former Phase C2 key machinery — signing-key rotation (`POST
>   /keys/rotate`, `past` keys) and past-key install verification
>   (`publishedAt < rotatedAt`) are **obsolete**: plan-23 uses a one-off
>   signing key per package (no windows, nothing to rotate) and `issued`-only
>   proofs/attestations that verify forever. Ident rotation, re-anchor, and
>   auth-key revocation live in plan-23-B.

## Context

Only the `available` release state exists, and there is no signed metadata:
no offline root, no snapshot/timestamp chain, so a mirror or MITM can serve a
stale or partial index undetected. Closes gap rows §2.3 (offline root,
snapshot/timestamp) and §2.5 (`/release-state`, `/root.json`,
`/snapshot.json`, `/timestamp.json`, release states).

## Phases

### Phase C1 — Release states

Maintainer lifecycle for published versions. Blob and signatures untouched —
a state is registry metadata about a version, never a modification of it.

- [ ] `POST /release-state` (§5): maintainer sets `available|deprecated|yanked` (never `blocked`/`legal-tombstoned` — registry-operator states); authenticated session + ident-signed request body (domain-tagged, mirroring plan-23 proof signing); logged to the plan-23-B transparency log.
- [ ] `Store` persists state transitions with timestamps; `/index` serves the current state per version (plan-10-A field).
- [ ] Client: `mfb pkg` surfaces states on `info`; resolution eligibility (`available`/`deprecated` eligible, `yanked` pin-only, `blocked`/`legal-tombstoned` excluded) is consumed by plan-10-B's resolver.
- [ ] Tests: state change requires the owner's ident signature (auth session alone refused); transition is logged with an inclusion proof; `/index` reflects the new state; yanked is excluded from floating resolution but selectable by exact pin (test lands with plan-10-B's resolver; here assert the served state).

Acceptance: release states are ident-authorized, logged, and served; yanked semantics are enforced at the index/eligibility level.
Commit: —

### Phase C2 — Signed metadata root-of-trust

Offline root + online snapshot/timestamp + client trust checks. Depends on C1
(states are part of the snapshotted index) and plan-23-B (checkpoint).

- [ ] Offline registry root key + `root.json` binding: registry ID, the **delegated plan-23 server (attestation) key**, delegated snapshot/timestamp keys, thresholds, expiration, root version. The plan-23 `repoFingerprint` pin becomes "fingerprint of a key delegated by the pinned root".
- [ ] Online snapshot/timestamp keys; `GET /snapshot.json`, `GET /timestamp.json` carrying index hashes, versions, expiry, and the plan-23-B log checkpoint reference.
- [ ] Client trust: configured registry ID + pinned root fingerprint; reject expired metadata, undelegated keys, registry-ID mismatch, version rollback, and any index entry whose attestation key is not root-delegated or whose publish inclusion proof fails.
- [ ] Tests: tampered snapshot rejected; expired timestamp rejected; rollback to an older snapshot version rejected; an attestation signed by a non-delegated key rejected; first-install verifies the full chain (root → snapshot/timestamp → index → plan-23 §3.5 package chain).

Acceptance: tampered/expired/rolled-back metadata is rejected and a first install verifies the full trust chain end to end.
Commit: —

## Decisions

- *To confirm (§5.4):* where the root/snapshot/timestamp private keys live —
  the offline root must not sit on the serving host; document the signing
  workflow (Phase C2). plan-23's server key moves under this root as a
  delegated key; its distribution answer (plan-23 §10.3) is then superseded by
  `root.json` fetch + pinned root fingerprint.
