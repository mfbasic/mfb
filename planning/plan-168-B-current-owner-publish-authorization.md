# plan-168-B: Publish, validate and token scope follow the stored package owner

Last updated: 2026-10-03
Effort: medium (1h–2h)
Depends on: plan-168-A

Server half of the bug-792 fix. `/validate` and `/publish` authorize against the
package's stored owner (as `/signing` and `/release-state` already do since
bug-494). They check the publisher's own current ident key and require
`package.author` to be the publisher. `package::verify_proof` / `verify_attestation`
bind the signed `owner` to the release's `author` rather than the ident prefix.
Publish tokens follow stored ownership too.

Outcome: the bug's probe `spec_probe_transfer_recipient_can_publish` passes
(bob's `alice#toolbox@2.0.0` validates and publishes with `publisher.owner == "bob"`,
`epoch == 1`). Alice can no longer publish `alice#toolbox`.

References: plan-168-A §3 (overall design), `bugs/bug-792-*.md`,
`src/docs/spec/package-manager/01_repository-protocol.md` §`/validate`,
`src/docs/spec/package-manager/04_owner-names.md`.

## Prerequisites

See plan-168-A §Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-168-A complete | `grep -c '^Commit: —' planning/plan-168-A-*.md` → `0` (or A archived under `planning/completed/`) | NOT MET |

## 1. Goal

- A session owner who is the stored owner of an existing package can validate
  and publish a new version under the unchanged ident, with their own ident key.
- A session owner who is not the stored owner is refused, even when the ident
  prefix names them.
- First publish (no `packages` row) is unchanged: the prefix decides.
- Proof and attestation `owner` fold-equal `package.author`, and the ident keeps
  its `<owner>#<package>` shape check.

### Non-goals

- No client verification changes (C), and no `signing_ident` change (D).
- Diagnostics for the first-publish path keep their exact strings.
  `session owner does not match package ident owner` is referenced 3 times
  (`grep -rn 'session owner does not match package ident owner' repository/src src tests | wc -l` → 3).
- No change to the attestation JSON or proof JSON shape.

## 2. Current State

- `repository/src/server.rs:validate_package_request` step 1 does four
  prefix-based things:
  - compares `fold(owner_part)` with `claims.sub`;
  - looks up `owner_auth_key_by_fingerprint(owner_part, ...)`;
  - compares `package.author` with that owner's display;
  - compares the header ident fingerprint with `owner_with_ident_key(owner_part)`.
- `repository/src/package.rs:verify_proof` / `verify_attestation` both
  `expect("owner", ident prefix)`.
- `server.rs:require_package_owner` is the precedent: the stored owner decides
  when a row exists, the prefix only for first publish.
- `server.rs:scope_permits` / `scope_owner_matches` compare token scopes against
  the ident prefix, so bob cannot scope a CI token to `alice#toolbox`.
- `verify_proof` / `verify_attestation` callers:
  - production: `server.rs:validate_package_request`,
    `src/cli/pkg.rs:validate_package_file`,
    `src/cli/build/packages.rs:classify_installed_package`;
  - tests: `repository/src/package.rs`, `src/target/package_mfp/mod.rs`,
    `tests/cli/cli_repo_publish.rs`.
  - Command: `grep -rn 'verify_proof\b\|verify_attestation\b' --include=*.rs src repository/src tests`.

### Measured populations

| What | Count | Command |
|---|---|---|
| verify_proof/verify_attestation call lines (production 6, tests 9) | 15 | command above, `\| grep -v 'pub fn' \| wc -l` |
| Diagnostics strings asserted on | "session key does not match current owner key" 3, "package ident owner is not registered" 2, "package author does not match owner name" 2, "package identKey does not match the owner's current ident key" 1 | `grep -rn '<string>' repository/src src tests \| wc -l` |
| `scope_permits` / `scope_owner_matches` references | 15 | `grep -c 'scope_permits\|scope_owner_matches' repository/src/server.rs` |

### Verified properties

- **Every published pre-fix artifact satisfies `fold(proof.owner) == fold(author)`
  and `attestation.owner == author`.**
  - `verify_proof` required `proof.owner == prefix` exactly.
  - Validation required `fold(prefix) == fold(session)` and `author == owner_display`.
  - `/signing` signs `attestation.owner = owner_display`.
  - So every published pre-fix artifact meets both conditions. Exact `proof.owner == author` is NOT
    guaranteed for a hand-crafted artifact whose prefix differs in case from the
    display name; fold-equality is (B-D1).
- `package::verify_proof_rejects_field_mismatch_and_bad_json` asserts the
  `<owner>#<package>` shape error. The shape check is kept, so the test stays valid.

## 3. Design

- `validate_package_request`, step 1 rewritten:
  - `owner = owner_auth_key_by_fingerprint(claims.sub, auth_fp)`, the session
    owner;
  - `match store.package_owner(ident)`: `Some(p)` with `p.id != claims.owner_id`
    → new diagnostic `session owner is not the package's current owner`;
    `None` → the existing prefix comparison and its existing string;
  - `package.author == owner.owner_display`;
  - step 5 compares the header fingerprint with
    `owner_with_ident_key(claims.sub)`.
- `verify_proof` / `verify_attestation`: keep the ident shape check; replace
  `expect("owner", prefix)` with `fold_owner(signed owner) == fold_owner(&package.author)`,
  error `proof owner does not match the package author`.
- Token scope:
  - `scope_permits(store, scope, ident)`: `X#*` permits when the package's stored
    owner (or, with no row, the prefix) fold-equals `X`; an exact scope still
    requires `scope == ident`.
  - `issue_token` accepts an exact scope `a#p` from issuer `o` when the stored
    owner of `a#p` is `o`.
- `publish_package` passes `publisher_ident_fingerprint` (A §4.3).
- Rejected: dropping the owner check from the proof entirely. That would let an
  alice-signed proof claim any author.

## Phases

> **NOTE: keep the checkboxes current as you go** (same rules as plan-168-A).

### Phase 1: RED regression and contrast tests

- [ ] `repository/src/server.rs` tests: land the bug's probe as
      `spec_probe_transfer_recipient_can_publish`. Replace its two
      `h.store.create_transfer_offer` / `accept_transfer` lines with the A
      helper that drives the real v2 handlers. All assertions stay as written in
      the bug doc.
- [ ] Add the contrast tests:
  - `the_former_owner_cannot_publish_after_transfer`: alice validates
    `alice#toolbox@2.0.0` and is refused with
    `session owner is not the package's current owner`;
  - `first_publish_is_still_prefix_bound`.
- [ ] Confirm RED: the probe fails with the bug's documented diagnostics.

Acceptance: the regression is RED for the documented mechanism.
  Check: `cargo test -p mfb_repository --lib -- spec_probe_transfer_recipient_can_publish` → FAILED with `recipient validation failed: ["session owner does not match package ident owner", "session key does not match current owner key"]` (est. 3 min; guess).
Commit: —

### Phase 2: Authorization and proof binding

- [ ] `repository/src/package.rs:verify_proof` / `verify_attestation`: implement
      §3. Add `package.rs` tests:
  - `proof_owner_binds_to_the_author_not_the_prefix` (bob-authored `alice#x`
    passes; alice-owner proof on a bob-authored package fails);
  - `a_case_differing_legacy_prefix_still_verifies`.
- [ ] `repository/src/server.rs:validate_package_request`, `scope_permits`,
      `issue_token`, `publish_package`: implement §3.
- [ ] Tests:
  - `a_recipient_token_scoped_to_the_transferred_package_can_attest`;
  - `the_publish_records_bob_as_publisher_at_epoch_1`;
  - the existing `publish_chain_enforces_two_credentials_and_pinning` and
    `a_transferred_package_is_governed_by_its_current_owner` run unchanged.

Acceptance: the probe is GREEN, the contrast cases hold, and no pre-existing
publish test changed.
  Check: `cargo test -p mfb_repository --lib -- spec_probe_transfer the_former_owner first_publish proof_owner a_case_differing a_recipient_token the_publish_records publish_chain a_transferred_package` → `test result: ok`, 0 failed (est. 3–5 min; guess).
Commit: —

### Phase 3: Spec sync

- [ ] `01_repository-protocol.md` §`/validate`:
  - step 1 becomes stored-owner authorization (first publish uses the prefix);
  - step 5 checks the session owner's current ident;
  - step 6 binds the proof owner to `author`;
  - remove the `validate_package_request` clause from the deviation paragraph.
- [ ] `04_owner-names.md`: the paragraph citing `require_package_owner` states
      that `/validate` / `/publish` also use the stored owner.
- [ ] Token-scope prose in §Accounts.

Acceptance: `cargo test --bin mfb spec` → `test result: ok` (est. 3–6 min; guess).
Commit: —

## Validation Plan

- Tests: Phases 1–2, with negative cases (the former owner, a foreign proof
  owner, a foreign token scope).
- Runtime proof: the probe drives the real `/signing` and `/validate` handlers
  end to end in-process. The CLI path waits on D (`signing_ident`).
- Final gate (once): `cargo test -p mfb_repository --no-fail-fast > /tmp/168B.log 2>&1; echo EXIT=$?` → `EXIT=0`, then `cargo test --test cli_repo_publish > /tmp/168B-pub.log 2>&1; echo EXIT=$?` → `EXIT=0` (est. 15–20 min; guess). The second command covers the CLI publish/install path that calls `verify_proof`.

## Open Decisions

- **B-D1.** Recommended: fold-equal `owner` vs `author`, because exact equality
  breaks hand-crafted case-differing legacy releases (§Verified properties).
  Alternative: exact equality. Rejected.
- **B-D2.** Recommended: token wildcard scopes follow stored ownership, so bob's
  `bob#*` covers `alice#toolbox` after the transfer. Alternative: require an
  exact scope per transferred package. More friction, and no security gain,
  because the stored owner already controls the package.

## Corrections

## Summary

A small, well-precedented change (`require_package_owner`). The risk is in
`verify_proof`, which clients share, and the case-folding rule plus the legacy
test cover it.
