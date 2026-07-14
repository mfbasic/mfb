# plan-10-D: Accounts + hardening

Last updated: 2026-07-04
Effort: medium

Part **D** of plan-10 (Package Registry Completion). Closes the §2 accounts
surface (orgs, publish tokens, transfers) and the operational hardening not in
the spec but needed for correctness under load. Lowest urgency; interleaves
with the other sub-plans as capacity allows. Overview and gap analysis:
[plan-10-repo.md](plan-10-repo.md).

- **Depends on:** [plan-23](plan-23-key-trust-model.md) (**assumed complete** —
  account/key model and the transparency log all account actions log to) and
  plan-10-C Phase C1 (release states share the ident-signed-request pattern).
  Hardening depends on nothing.
- **Spec:** `mfb spec package-manager` (`src/docs/spec/package-manager/`,
  esp. `owner-names` and `repository-protocol` — accounts); operational items
  are gap §2.6 (no spec).

> Superseded by plan-23 and removed from this document: key-rotation recovery
> (ident chain rotation + re-anchor, plan-23-B) and per-machine
> credential add/revoke (machine linking, plan-23-A/B).

## Context

Only single-user accounts exist — no orgs, publish tokens, or ownership
transfer — and the service has operational gaps (global DB mutex, no reaping,
no rate limiting, no request-size cap). Closes gap rows §2.2 (orgs, publish
tokens, ownership transfer) and §2.6 (operational).

Under the plan-23 model these features must respect two invariants:

1. **Publishing always requires the ident key** (the proof) — no account
   feature may create a credential that can publish without it.
2. **Every account mutation is ident-authorized and logged** — auth sessions
   alone may read and request attestations, never change account state.

## Phases

### Phase D1 — Accounts: orgs, publish tokens, transfers

Depends on plan-23 (keys, log) and plan-10-C C1 (ident-signed request pattern).

- [ ] **Orgs**: an org is an account with its own ident keypair. Member/role
  tables (owner/admin/publisher) bind *member idents* to the org; role grants
  and removals are requests signed by an owner/admin member's ident, logged.
  Publishing an org package: the builder is a machine linked to the **org**
  ident (plan-23 link flow) — the proof is org-ident-signed; the member's own
  ident never signs org packages. Role checks gate who may link a machine to
  the org ident and who may request attestations for org packages.
- [ ] **Publish tokens** (CI creds): a token is a **scoped auth key** — an
  auth keypair registered with a scope (owner/package/route restrictions),
  a TTL, and ident-signed issuance; revocable individually; logged at issue,
  use, and revoke. A token can request attestations only within scope and can
  never bypass invariant 1 (the CI box still needs the ident to sign proofs —
  i.e. a CI publisher is a *linked machine* whose auth key happens to be
  scoped and short-lived).
- [ ] **Ownership transfer**: two-sided — a transfer offer signed by the
  current owner's ident and an acceptance signed by the receiving owner's
  ident; server re-binds the package to the new owner and logs both halves;
  the package's already-published versions keep verifying against the old
  ident's proofs/attestations (issued-only facts), while new versions publish
  under the new ident.
- [ ] Tests: org publish honors member roles (publisher can, plain member
  cannot); a scoped/expired token cannot exceed its scope, cannot mutate
  account state, and cannot publish without the org/owner ident; a transfer
  requires both ident signatures and both log entries; old versions of a
  transferred package still verify, new ones verify under the new ident.

Acceptance: org membership, scoped/revocable tokens, and two-sided transfers
all work, are ident-authorized, logged, and never violate the two invariants.
Commit: —

### Phase D2 — Hardening

Operational; independent of the other phases.

- [ ] SQLite WAL + a small connection pool (or a writer task) to remove the global mutex bottleneck.
- [ ] Background reaping of expired challenges, sessions, and pending link/pairing blobs (plan-23 adds the latter).
- [ ] Rate limiting on register/challenge/login and `POST /signing` (attestation issuance is cheap but logged — rate-limit to keep the log spam-free).
- [ ] Request-size cap on inline artifacts; optional upload-reference flow for large `.mfp` blobs.
- [ ] Typosquat warn-only check at publish.

Acceptance: concurrent publishes no longer serialize on the global mutex; expired challenges/sessions/pairing blobs are reaped; rate limits and the request-size cap are enforced; the typosquat check warns at publish.
Commit: —

## Open Decision

- **Org ident custody** (Phase D1): the org ident private key is held by
  owner/admin members' machines via the plan-23 link flow. Recommend: linking
  a machine to an org ident requires an ident-signed approval from an existing
  owner/admin member (the plan-23 pairing flow, with the approver's role
  checked server-side at relay time), so org key spread is role-gated and
  logged. Alternative (defer): server-side org signing — rejected for
  violating invariant 1.
