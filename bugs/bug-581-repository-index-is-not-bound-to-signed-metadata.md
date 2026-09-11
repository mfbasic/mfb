# bug-581: a verified package index is not bound to its request or signed snapshot

Last updated: 2026-09-11
Effort: large (3h–1d)
Severity: HIGH
Class: Security / supply-chain integrity

Status: Open
Regression Test: `repository/src/client.rs` index/metadata tests to be added.

`fetch_index(owner, package)` validates a server signature over the response's
`owner` and ident-key fingerprint, but does not verify that the returned `ident`
is the requested `<owner>#<package>`.  It also verifies root → timestamp →
snapshot metadata, then discards the snapshot's committed `indexHash` without
binding the served index response to it.  A registry holding its online server
key can return a validly signed binding and version list for a different package
or a targeted stale/truncated index; the client returns it as the requested
package index.

Correct behavior: an index response is cryptographically and structurally bound
to the exact requested package and to the signed metadata state the client has
accepted.

References:

- Security review, 2026-09-11 (key and metadata protocol pass)
- `repository/src/store.rs:Store::index_canonical_hash`

## Failing Reproduction

Extend `client::tests::fetch_index_verifies_the_name_binding_before_returning_the_ident_key`
to serve a response with a valid server signature for `mallory#other` while the
client requests `alice#pkg`:

```
cargo test -p mfb_repository client::tests::fetch_index_rejects_a_validly_signed_response_for_another_ident
```

- Observed today: `fetch_index` accepts it; its only signature message is
  `name_binding_message(response.owner, response.ident_fingerprint)`.
- Expected: it rejects before returning any versions because the response ident
  and canonical owner do not match the requested route.

Add a pinned-root test whose valid snapshot commits one index hash while the
served package response represents another; it must also reject.

## Root Cause

`repository/src/client.rs:fetch_index` constructs `ident` for the URL but never
compares it with `response.ident`, nor compares `response.owner` with the
requested owner.  `verify_pinned_metadata` verifies snapshot freshness and
returns no value; `DelegatedMetadata.index_hash` is therefore never compared to
the index received afterwards.  The server-side binding in
`repository/src/server.rs:package_index` signs only owner and ident fingerprint,
not the requested package or versions.

## Goal

- Reject a response whose owner/ident differs from the requested index.
- Provide and verify a signed, package-specific index commitment (or an
  inclusion proof in a signed full-index commitment) before using versions.

### Non-goals (must NOT change)

- Do not weaken the existing server-key pin, root chain, or package signature
  checks.
- Do not treat a matching name-binding signature alone as package-index
  authenticity.
- Do not silently re-pin a different ident for an existing dependency.

## Blast Radius

- `client.rs:fetch_index` — fixed in this bug.
- `client.rs:verify_pinned_metadata` / `DelegatedMetadata` — fixed in this bug
  because the currently verified index hash is unused by every caller.
- `server.rs:package_index` and `store.rs:index_canonical_hash` — require a
  compatible package commitment/proof design.
- `server.rs:package_detail` — anonymous display API; audit separately if it is
  ever used as an install authority.

## Fix Design

First add direct route-binding checks.  Then extend signed metadata with a
verifiable per-package target/index commitment, or serve a Merkle inclusion
proof against the signed full index.  Hashing a client-selected subset cannot
verify the present global `indexHash`, so merely comparing a package response to
that value is not a viable fix.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add red wrong-ident, wrong-owner, stale-index, and truncated-index tests.
- [ ] Identify every install/resolution consumer of `IndexResponse`.

Acceptance: all attacks are accepted on HEAD and the test evidence records why.
Commit: —

### Phase 2 — the fix

- [ ] Enforce exact route binding in `fetch_index`.
- [ ] Add a signed package-index commitment/proof and verify it client-side.

Acceptance: every attack test rejects while an honest metadata/index flow works.
Commit: —

### Phase 3 — full validation

- [ ] Run `cargo test -p mfb_repository`.
- [ ] Verify add/install against rotated metadata and an advancing index.

Acceptance: full suite green; index selection is bound to signed state.
Commit: —
