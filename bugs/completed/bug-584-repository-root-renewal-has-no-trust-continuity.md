# bug-584: root renewal replaces the pinned root without an authenticated transition

Last updated: 2026-09-12
Effort: large (3h–1d)
Severity: MEDIUM
Class: Security / root-of-trust lifecycle

Status: Fixed
Regression Test: `store::tests::init_registry_root_refuses_to_replace_a_configured_root`,
`store::tests::renew_registry_root_keeps_the_anchor_and_rotates_the_delegated_keys`,
`store::tests::renew_registry_root_refuses_anything_but_the_configured_root_key`,
`store::tests::reanchor_registry_root_replaces_the_anchor_only_when_explicitly_selected`,
`server::tests::a_pinned_client_verifies_the_chain_across_an_authenticated_root_renewal`,
`client::tests::verify_pinned_metadata_follows_a_renewal_and_refuses_the_retired_root`,
`local::tests::root_version_round_trips_and_fails_closed_when_malformed`,
`cli_repo_governance::repo_root_renewal_keeps_an_already_pinned_client_verifying`.

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

Measured on pre-fix code (`cargo test -p mfb_repository --lib
measure_bug_584_second_init -- --nocapture`, scratch test): a SECOND
`init_registry_root` on a configured store

- minted a brand-new offline root keypair (`private_changed=true`),
- replaced the anchor (`c3c8a3fd…` -> `fc5597ad…`),
- rotated the online snapshot/timestamp keys, and
- bumped `root.json` to `"version":2`,

all with `Ok(...)` and no way to authenticate the transition — the old root key
was neither supplied nor retained. The doc's root cause held exactly.

The RED regression test is
`store::tests::init_registry_root_refuses_to_replace_a_configured_root`; on
pre-fix code it failed with `called \`Result::unwrap_err()\` on an \`Ok\` value`
(the second ceremony succeeded).

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

Implemented: the one conflated ceremony is split into three explicitly selected
modes, all written through a single `Store::write_registry_root`, each appending
a transparency-log entry (`root-init` / `root-renew` / `root-reanchor`).

- `init_registry_root` is **one-time**: it refuses against a configured
  registry and touches nothing while refusing.
- `renew_registry_root(registry_id, expires_at, root_private)` is the
  authenticated transition. The operator supplies the offline root key;
  possession of it IS the authentication. The version bumps, the expiry is
  fresh, the online snapshot/timestamp keys are regenerated, and the ANCHOR is
  unchanged — so an already-pinned client verifies the renewed chain with no
  out-of-band step and no client-side protocol change. A stranger key, a
  malformed key, a mismatched registry id, and an uninitialized registry are
  all refused without a write. The private key is still never persisted.
- `reanchor_registry_root` is the lost-key recovery ceremony, mirroring the
  existing ident `reanchor` precedent (plan-23 §3.6): it mints a new anchor,
  and every pinned client fails hard until it re-pins out of band.

CLI: `mfb-repo renew-root … --root-key-file <path>` (a PATH, so the offline
private key never enters the process table) and `mfb-repo reanchor-root …`.

Second half (9530d72a8): renewal rotates the delegated online keys, which makes
"renew to retire a compromised online key" a real remedy — but the client never
read `root.json`'s `version`, so the retired root could simply be replayed back.
`verify_registry_metadata` gained a `min_root_version` floor beside the existing
snapshot floor, pinned in a new `root-version` file that fails closed when
corrupt. Equal versions still verify, and a client with no such file starts at
0, so nothing already pinned is refused.

Deliberately NOT implemented: rotating to a new root key while the old one is
still available (an old-root-signed successor document). That needs a new
client-visible document format and a compatibility ruling for already-pinned
clients, and is recorded in the spec as unsupported in this protocol version.
Renewal covers expiry/delegated-key lifecycle without it.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add red renewal and overwrite-refusal tests.
- [x] Decide whether delegated-key renewal needs root rotation at all — it does
  NOT: renewal re-delegates fresh online keys under the same root key, so the
  anchor never has to move for routine lifecycle.

Acceptance: tests establish that a second invocation replaces the anchor.
Commit: a869dd247

### Phase 2 — the fix

- [x] Implement one-time initialization AND an authenticated root transition.
- [x] Add client verification and operator recovery documentation (`mfb spec
  package-manager repository-protocol` "Root lifecycle"; `mfb-repo` usage).

Acceptance: pinned clients either advance only through a valid transition or
remain protected from accidental/takeover-like replacement.
Commit: a869dd247, 9530d72a8

### Phase 3 — full validation

- [x] Run `cargo test -p mfb_repository --no-fail-fast` — exit 0, 361 lib + 24
  bin tests passed.
- [x] Verify first trust, delegated-key rotation, root renewal, and invalid
  transition cases (the regression tests above; the CLI acceptance test drives
  the real binaries end to end — `cargo test --test cli_repo_governance`, from a
  detached worktree, 2 passed, exit 0).

Acceptance: full suite green and root continuity is explicit and verifiable.
Commit: a869dd247, 9530d72a8
