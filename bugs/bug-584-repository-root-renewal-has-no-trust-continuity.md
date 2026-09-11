# bug-584: root renewal replaces the pinned root without an authenticated transition

Last updated: 2026-09-11
Effort: large (3h–1d)
Severity: MEDIUM
Class: Security / root-of-trust lifecycle

Status: Open
Regression Test: store and client metadata-chain tests to be added for a
client pinned before root renewal.

Re-running `mfb-repo init-root` generates a wholly new root key and overwrites
the sole `registry_config` record.  The old offline root key neither signs the
new root nor remains servable.  Clients that pinned the original root correctly
refuse the replacement; recovery requires accepting a new fingerprint out of
band, indistinguishable to the client from registry takeover.

Correct behavior: root renewal has an authenticated old-to-new transition with
threshold/expiry policy, or the command is explicitly one-time initialization
and refuses to overwrite a live root.

References:

- Security review, 2026-09-11 (root-key lifecycle trace)
- `repository/src/main.rs:init-root` prints the new private root key;
  `store.rs:init_registry_root` overwrites the configuration.

## Failing Reproduction

Create a root, trust it from a client, invoke `init_registry_root` again, and
attempt `verify_pinned_metadata` with the old root pin:

```
cargo test -p mfb_repository client::tests::a_pinned_client_can_verify_an_authenticated_root_renewal
```

- Observed today: the second ceremony replaces `root_public`; the client rejects
  it as not matching its pinned root fingerprint.
- Expected: either a valid transition authenticated by the old root advances
  trust, or the second invocation is refused before it changes state.

## Root Cause

`repository/src/store.rs:init_registry_root` always generates `root_public` and
`root_private`, then its `ON CONFLICT(id) DO UPDATE` overwrites root public key,
signed root JSON, and online delegated keypairs.  The old root private key is
not supplied to the command and is not retained, so no transition can be
created.  `client.rs:verify_registry_metadata` correctly anchors trust solely
in the stored old fingerprint and consequently has no renewal path.

## Goal

- Prevent silent replacement of a configured root.
- Define and implement an authenticated, auditable root-rotation ceremony if
  renewal is required.

### Non-goals (must NOT change)

- Do not persist the offline root private key on the serving host.
- Do not auto-accept a new root fingerprint merely because its version rises.
- Do not weaken root-fingerprint pinning or metadata-expiry checks.

## Blast Radius

- `store.rs:init_registry_root` — fixed in this bug.
- `main.rs:init-root` — must require the selected initialization/rotation mode.
- `client.rs:{trust_registry,verify_registry_metadata,verify_pinned_metadata}`
  — require transition verification or clear refusal diagnostics.
- `server.rs:root_metadata` — may need to serve a bounded prior-root/transition
  chain.

## Fix Design

The safe short-term fix is refusing a second initialization.  If rotation is a
product requirement, design a TUF-style root transition: old trusted root signs
the new root, version strictly increases, both documents are retained long
enough for clients to advance, and the CLI receives old-key signing material
only from an offline operator-controlled path.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add red renewal and overwrite-refusal tests.
- [ ] Decide whether delegated-key renewal needs root rotation at all.

Acceptance: tests establish that a second invocation replaces the anchor.
Commit: —

### Phase 2 — the fix

- [ ] Implement one-time initialization or an authenticated root transition.
- [ ] Add client verification and operator recovery documentation.

Acceptance: pinned clients either advance only through a valid transition or
remain protected from accidental/takeover-like replacement.
Commit: —

### Phase 3 — full validation

- [ ] Run `cargo test -p mfb_repository`.
- [ ] Verify first trust, delegated-key rotation, root renewal, and invalid
  transition cases.

Acceptance: full suite green and root continuity is explicit and verifiable.
Commit: —
