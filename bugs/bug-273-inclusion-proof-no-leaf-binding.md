# bug-273: `verify_publish_inclusion` never binds the transparency-log leaf to the queried package

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Security (trust-boundary gap)

Status: Open
Regression Test: repository/tests (new) — inclusion proof for ident@version rejects a leaf whose payload is a different entry

The client's publish-inclusion check fetches `(index, leafHash)` from
`/log/publish?ident=&version=` and verifies the RFC-6962 inclusion proof against
the signed checkpoint — but it never recomputes the leaf hash from the entry's
canonical contents `{"ident":…,"version":…,"hash":…}`. Because the client knows
all three values yet verifies none of them, a malicious or compromised registry
can answer with the `(index, leafHash)` of *any* real log entry (its own
`register` entry, an unrelated package's publish) and the proof verifies. Both
consumers — the post-publish self-check (`src/cli/pkg.rs`) and `mfb pkg verify
--proof` (which otherwise raises `PACKAGE_ATTESTATION_INVALID`) — then report
inclusion "verified" for a publish that was never logged, defeating the spec's
stated transparency-log guarantee.

The single correct behavior a fix produces: inclusion verification for
`ident@version` with expected content hash `H` must fail unless the proof's leaf
equals `leaf_hash(canonical{ident, version, H})` — i.e. the leaf is bound to the
exact package being verified.

References:

- `src/docs/spec/package-manager/01_repository-protocol.md` (Transparency Log:
  "every forgery path is forced to leave a signed entry in this log")
- Canonical publish-leaf payload: `repository/src/store.rs` (`publish_log_entry`,
  ~line 1956); `LogEntry` shape at `repository/src/server.rs` (~line 495).
- Found during goal-06 review of `repository/src/client.rs`.

## Failing Reproduction

```
# Hostile/compromised registry (or holder of the pinned server key):
# answer GET /log/publish?ident=alice#pkg&version=1.0.0 with the index+leafHash
# of some *other* real entry E, and serve a valid inclusion proof for E.
mfb pkg verify --proof alice#pkg@1.0.0
```

- Observed: prints inclusion verified ("log index N ⊂ checkpoint size M") for a
  package/version that was never published to the log.
- Expected: fails — the served leaf does not match `leaf_hash(canonical payload
  for alice#pkg@1.0.0 with the expected content hash)`.

## Root Cause

`repository/src/client.rs:470-509` (`verify_publish_inclusion`): checks
`proof.leaf == entry.leaf_hash` and that the proof verifies against the
checkpoint, but never reconstructs or compares
`leaf_hash({"ident":…,"version":…,"hash":…})`. The leaf is treated as an opaque
server assertion, so any genuine leaf in the tree satisfies the check.

## Goal

- `verify_publish_inclusion` recomputes the canonical leaf from
  `(ident, version, expected_content_hash)` and requires it to equal the entry's
  leaf hash before/alongside proof verification.
- `mfb pkg verify --proof` and the post-publish self-check reject a leaf-mismatch.

### Non-goals (must NOT change)

- The RFC-6962 proof math (already correct and tested).
- The wire shape of `/log/publish` beyond, if needed, returning the leaf payload.
- Do not weaken the check to a warning; it must be a hard failure.

## Blast Radius

- `client.rs:verify_publish_inclusion` — fixed by this bug.
- Caller `src/cli/pkg.rs` post-publish self-check — benefits, no change needed
  beyond passing the expected hash.
- Caller `src/cli/pkg.rs` `mfb pkg verify --proof` (`PACKAGE_ATTESTATION_INVALID`)
  — benefits.
- `verify_log_consistency` — separate latent issue, see bug-276.

## Fix Design

Pass the expected content hash into `verify_publish_inclusion` (the caller already
knows it from the resolved/downloaded package), reconstruct the canonical leaf
string exactly as `store.rs` writes it, hash it with the same `log::leaf_hash`,
and require equality with `entry.leaf_hash`. Rejected alternative: trusting the
server to return the payload and hashing that — still lets the server pick which
package the leaf describes; the client must build the payload from values it
independently knows.

## Phases

### Phase 1 — failing test + audit
- [ ] Test: serve a valid proof for entry E while querying a different
      ident@version; assert verification fails.
### Phase 2 — the fix
- [ ] Reconstruct + compare the canonical leaf in `verify_publish_inclusion`;
      thread the expected hash through both callers.
### Phase 3 — validation
- [ ] Full `repository/` suite green; `mfb pkg verify --proof` still passes for a
      genuinely-logged package.

## Validation Plan

- Regression test: mismatched-leaf inclusion rejection.
- Runtime proof: honest publish still verifies; substituted leaf is rejected.
- Doc sync: none (this restores the documented guarantee).

## Summary

The proof verifies the wrong thing: it confirms the leaf is in the tree but not
that the leaf *is* this package. Binding the leaf to `(ident, version, hash)`
closes it; risk is limited to the reconstruction matching `store.rs`'s canonical
encoding byte-for-byte.
