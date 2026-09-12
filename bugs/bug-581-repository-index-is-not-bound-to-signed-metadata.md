# bug-581: a verified package index is not bound to its request or signed snapshot

Last updated: 2026-09-12
Effort: large (3h–1d)
Severity: HIGH
Class: Security / supply-chain integrity

Status: **PARTIAL.** Phase 1 (route binding) landed in `ed87c111a`.
Phase 2 (binding the index to signed snapshot state) is root-caused below and is
an OPEN DESIGN DECISION — it is a signed-metadata format change, not a bug fix.
Regression Test: `client::tests::fetch_index_rejects_a_validly_signed_response_for_another_ident`
and `client::tests::fetch_index_accepts_an_honest_response_whose_owner_case_differs`
in `repository/src/client.rs`.

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

- [x] Add red wrong-ident and wrong-owner tests.
- [ ] Stale-index and truncated-index tests — deferred with Phase 2; there is
      nothing to assert against until a per-package commitment exists.
- [x] Identify every install/resolution consumer of `IndexResponse`.

**Measured, and the report was right about the mechanism.** `fetch_index`
builds `ident` for the URL and never compares it with `response.ident`, nor
`response.owner` with the requested owner. Its only signature check is
`name_binding_message(response.owner, response.ident_fingerprint)` — over values
the response itself supplies, so it binds the response to *itself*.

RED: `fetch_index_rejects_a_validly_signed_response_for_another_ident` fails on
pre-fix code at its first assertion — a `mallory#other` index returns `Ok`.

**One thing the report did not mention, and it decides the fix.** Owners are
resolved case-folded server-side (`validation::fold_owner` → `owners.owner_folded`)
but the response carries `owner_display`, so `Alice#pkg` is a legitimate request
whose honest answer says `alice`. An exact owner comparison — the obvious
reading of "compare `response.owner` with the requested owner" — would have
refused valid input. The ident, by contrast, is echoed back verbatim by
`server.rs:package_index` (it returns the raw path parameter), so that one is an
exact comparison. Two fields of the same response, two different comparison
rules, and only measuring the server tells you which is which.

**Consumers of `IndexResponse`** (`grep -rn --include='*.rs' fetch_index`):
`src/cli/resolve.rs` (dependency resolution / lock) and `src/cli/pkg.rs`
(`pkg add`, which pins the returned `identKey` into `project.json`). `pkg add`
is why this is HIGH: the substituted key becomes the trust anchor for a
dependency the user never named.
Commit: — (tests landed with the fix, `ed87c111a`)

### Phase 2 — the fix

- [x] Enforce route binding in `fetch_index`.
- [ ] Add a signed package-index commitment/proof and verify it client-side —
      **open design decision, see below.**

Phase 1 rejects the response unless `response.ident` equals the requested ident
exactly and `fold_owner(response.owner)` equals `fold_owner(owner)`. Three
cryptographically VALID substitutions are refused, each defeating a different
half-fix: a different owner+package; the same owner with a different package
(an owner-only check would accept it); and the correct ident echoed back but
signed for and keyed to another owner (an ident-only check would accept it).

POSITIVE: `fetch_index_accepts_an_honest_response_whose_owner_case_differs`
passes **both before and after** the fix. That is what makes it a containment
check rather than a restatement of the new behaviour.

Acceptance: every attack test rejects while an honest metadata/index flow works.
**Met for Phase 1.**
Commit: `ed87c111a`

### Phase 3 — full validation

- [x] Run `cargo test -p mfb_repository`.
- [ ] Verify add/install against rotated metadata and an advancing index —
      belongs to Phase 2.

`cargo test -p mfb_repository --no-fail-fast` -> **343 passed, 0 failed,
exit 0**, run in an isolated `git worktree add --detach` so concurrent peer
edits in the shared checkout could not contaminate it. The pre-existing
`fetch_index_verifies_the_name_binding_before_returning_the_ident_key` passes
untouched: the route binding is purely additive and relaxes no existing check.

**The instrument.** This is repository client transport logic. It emits no IR
and moves no golden, so the artifact gate is structurally blind to it and its
silence would prove nothing. The `mfb_repository` unit suite, with its
loopback-HTTP stub registry, is the only instrument that exercises the protocol.

Acceptance: full suite green. **Met.** "Index selection is bound to signed
state" is **not** met — that is Phase 2.
Commit: `ed87c111a`

## Phase 2 is a design decision, not a bug fix — and here is why

Phase 1 closes *substitution*: the index you get is for the package you asked
for. It does **not** close *staleness or truncation*: a registry can still serve
a correctly-identified version list that omits a newer version, and nothing in
the response contradicts it.

### What was measured

- `verify_pinned_metadata` (`client.rs:1039`) verifies root -> timestamp ->
  snapshot, and returns `()`. `DelegatedMetadata.index_hash` is computed and
  **discarded**; no caller has ever read it. The report is correct.
- `store.rs:index_canonical_hash` hashes `(ident, version, hash, state)` for
  **every package in the registry**, sorted, as one SHA-256. So `indexHash`
  commits to global state.
- There is no route that serves the full index. `grep -n '\.route(' server.rs`
  lists `/index/:ident` and nothing else index-shaped.

Those three facts together mean the obvious fix is not available: a client
holding one package's response **cannot** recompute the global `indexHash`, and
there is no way to fetch the input that would let it. The bug doc's Fix Design
already says this, and measuring confirms it.

### Why the remaining options are the owner's call

To defeat staleness you need a commitment to **per-package** state signed by the
**offline** snapshot key. (A commitment signed by the online server key is worth
little here: the threat model for this bug is precisely an adversary holding
that key — that is what made the name binding useless.) Two shapes:

1. **Per-package targets in `snapshot.json`.** Snapshot carries a map from ident
   to a digest of that package's canonical version list. Simple to verify;
   snapshot size grows linearly with the package count, and every publish
   re-signs a document proportional to the whole registry.
2. **Merkle root + inclusion proof.** `indexHash` becomes a Merkle root over
   per-package leaves and `/index/:ident` serves an inclusion proof. Snapshot
   stays O(1) and the log machinery already in `log.rs` (RFC 6962, inclusion and
   consistency proofs, both already tested) can be reused directly. It changes
   the meaning of the existing `indexHash` field.

Either is a **wire and metadata format change**, and both force a compatibility
ruling that is not a technical detail: **what should a client do when
`snapshot.json` carries no per-package commitment?** Fail closed breaks every
already-deployed registry and every pinned client. Fail open means the fix does
nothing against an adversary who simply omits the field — the same fail-open
mistake bug-517 explicitly avoided. There is no safe default to assume here, so
guessing one would be the wrong kind of progress.

Recorded rather than implemented, per the standing rule that "root-caused but
the fix is a product decision" is a legitimate outcome.

### What is NOT blocked, if Phase 2 is wanted

Option 2 reuses `log.rs` wholesale, and since bug-582 every checkpoint advance
is already consistency-proof-gated — so the append-only anchor a per-package
Merkle commitment would hang off is now sound. That was not true before this
same pass.
