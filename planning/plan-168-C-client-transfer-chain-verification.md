# plan-168-C: Clients verify the signed transfer chain before trusting a new owner

Last updated: 2026-10-04
Effort: large (3h–1d)
Depends on: plan-168-B

The trust half of plan-168. One pure verifier decides whether a release's header
`identKey` is trusted for the release's publisher, starting from the project's pin
(or the registry name binding on first add). It walks the owners' rotation chains
and the signed transfer links. Every client consumer uses it.

The `project.json` pin and the `mfb.lock` `identKey` become "the verified identKey
of the installed release". Pin-follow extends from rotations to transfers.

Outcome:

- A consumer pinned to alice installs bob's `alice#toolbox@2.0.0` with the
  notice `ownership of alice#toolbox moved to bob through a signed transfer`.
- `@1.0.0` still installs.
- A forged offer, a replayed link, a withheld release or an unchained key is
  refused with `PACKAGE_IDENT_KEY_UNTRUSTED`.

References: plan-168-A §3–§4; `01_repository-protocol.md` §Ident Rotation
(pin-follow) and §Install Path; `03_signing.md` (`mfb info`);
`src/docs/spec/tooling/03_lockfile.md`, `01_project-manifest.md`.

## Prerequisites

See plan-168-A, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-168-B complete | `grep -c '^Commit: —' planning/plan-168-B-*.md` → `0` (or archived) | MET (archived in `0fa3d9968`) |

## 1. Goal

- `repository/src/trust.rs:verify_release(...)` accepts a release exactly when
  the rules in §3 hold, and every consumer below calls it.
- Pre-transfer releases verify offline after install exactly as today.

### Non-goals

- No new diagnostic rule codes. Reuse `PACKAGE_IDENT_KEY_UNTRUSTED` /
  `PACKAGE_IDENT_REANCHORED`, so `src/docs/spec/diagnostics/01_rule-codes.md`
  is unchanged.
- No binding of `/index` to signed snapshot metadata (existing spec deviation).
- Offline `mfb build` stays network-free: it keeps comparing header vs pin
  (`build/packages.rs:classify_installed_package`).

## 2. Current State

- Identity consumers of `IndexResponse`. There are 7 `fetch_index*` call sites
  (`grep -rc 'client::fetch_index\(_with_key\)\?(' src/cli/pkg.rs src/cli/resolve.rs src/cli/info.rs`
  → 4 / 2 / 1). Of these, 4 read identity fields
  (`grep -rn 'index\.ident_key\|index\.ident_fingerprint' src`):
  - `src/cli/pkg.rs:add_package_from_registry` (pin := `index.ident_key`);
  - `src/cli/resolve.rs:registry_dependency_for` (indirect pin);
  - `src/cli/resolve.rs:resolve` (lock `identKey`/`identFingerprint` := index);
  - `src/cli/info.rs:OnlineRegistry::current_ident`.
  The other 3 (`check_abi`, `update_declared_dependency`, `print_package_info`)
  read versions/state only.
- `src/cli/pkg.rs:follow_rotated_pin` fetches `/idents/<prefix owner>` and
  follows rotations only.
- `src/cli/mod.rs:install_verified_package` → `classify_installed_package`
  requires header == the lock key.
- `src/cli/info.rs:verify_chain`:
  - rejects `proof ident does not belong to the signing owner` when the prefix ≠
    the signer;
  - queries `/index/<signer>#<package>`, which is the wrong ident after a transfer.
- **UNVERIFIED (suspected pre-existing).** After an ident rotation, a fresh
  registry `pkg add` of a pre-rotation release may fail, because `resolve` locks
  `index.ident_key` (current) while the header holds the old key. No test covers
  a registry add across rotation:
  `grep -n 'pkg", "add"' tests/cli/cli_repo_governance.rs` shows only `file://`
  adds in the rotation test. Phase 1 measures it.

### Verified properties

- Rotation is linear per owner (`store.rs:rotate_ident` replaces the one current
  ident key; `ident_chain` links old→new). Every key alice used before offering
  precedes the offer key on her chain, so forward walks from any pre-transfer pin
  reach the offer key.
- `/idents/<owner>` is not server-signed. A's `toBinding` supplies the signed
  current key; the chain links are self-authenticating (signed by the old key).

## 3. Design: the verifier (`repository/src/trust.rs`, new module in `lib.rs`)

Inputs:

- the server key;
- the verified `IndexResponse`;
- `chain_of(owner) -> IdentChainResponse` (injected, for testing);
- the anchor (`Pinned(key)` or `FirstUse`);
- the release (`IndexVersion` + header `identKey` + content hash).

Output: `Trusted { key, publisher, crossed_transfer: bool }` or `Err(detail)`.

**Owners and links.** `O_0 = index.owner`, and link `k` moves `O_k → O_{k+1}`.

**Link checks.** For each link `k`, in this order:

1. `sequence == k`.
2. A legacy link (null signatures) ends the walk: `n_trusted = k`. Neither its
   owner fields nor any later link is checked. This comes before the owner check
   so that legacy data the client cannot authenticate never fails an index whose
   pre-transfer releases verify today. With no legacy link, `n_trusted` = the
   number of links.
3. `fold(fromOwner) == fold(O_k)`, and the `toBinding` name binding verifies
   under the server key.
4. `fp(toIdentKey)` lies on `toOwner`'s chain at or before the bound key
   (follow from `toIdentKey` reaches the bound key).
5. The offer verifies under `fromIdentKey`, and the accept under `toIdentKey`,
   over v2 with `releaseDigest`.
6. `release_digest(versions with epoch <= k) == releaseDigest`. A withheld or
   re-dated release breaks this.

**Key trust per owner.**

- `Pinned(P)`: P must lie in some owner's **tenure**, and `j` is the first such
  tenure.
  - Tenure `k` is owner `O_k`'s run of keys while holding the package:
    forward-reachable from the accept key `A_{k-1}` of link `k-1` (no lower
    bound for `k = 0`), and forward-reaching the offer key `F_k` of link `k`
    (for the last trusted owner, the bound current key: the last link's
    `toBinding`, or the index binding when there are no links).
  - The tenure rule matters when an owner appears twice (alice→bob→alice): P
    can be on alice's chain in tenure 0 and tenure 2, or in both when she never
    rotated. The first match gives the most verification through signed links.
    Any match is sound, but a fixed rule keeps the result deterministic.
  - If P is in no tenure, verification fails.
  - Owner `O_j` trusts the keys forward-reachable from P (`chain_path`, a new
    helper next to `client.rs:follow_ident_chain` returning every key on the path).
  - Owners after `O_j` trust the keys forward from their accept key `A_k`.
  - Owners before `O_j` are trusted only through digests: a release with epoch
    `e < j` must be in `D_{j-1}`, and P must be forward-reachable from `A_j`.
- `FirstUse`: `O_0` trusts its bound current key and that key's chain ancestors
  (TOFU, the same trust `pkg add` grants today). Later owners trust as above.
- A link's `fromIdentKey` must be trusted for `O_k`.

**Release check (epoch e).**

- `publisher.owner` fold-equals the header `author`.
- When `e <= n_trusted`: `publisher.owner` fold-equals `O_e`, and either the
  header key is trusted for `O_e`, or `e < j` and the release is in `D_{j-1}`.
- When `e > n_trusted` (past a legacy terminator), the **origin-owner fallback**
  applies: accept only if `publisher.owner` fold-equals `O_0`, and the header key
  is trusted for `O_0` by the anchor alone. That means forward from P when P is
  in `O_0`'s tenure 0, or the bound key and its ancestors for `FirstUse`. This is
  exactly today's prefix-owner rule, so legacy alice→bob→alice packages keep
  shipping. Any other publisher past the terminator is refused
  (plan-168-B refuses to publish one).
- An epoch past the number of served links is refused.

**New pin** := the header key.

**Consumers.**

- `follow_rotated_pin` → `follow_identity_pin`. Same call site in
  `verify_packages`, plus a notice for a crossed transfer.
- `add_package_from_registry`: pin := the verified header key of the chosen version.
- `resolve`:
  - verify each selected release; its blob is already fetched by
    `load_import_edges`;
  - `LockedPackage.ident_key` / `ident_fingerprint` := the verified key;
  - `apply_manifest_change` rewrites differing pins via
    `manifest::package::project_json_with_updated_ident_key`.
- `registry_dependency_for`: `FirstUse` anchor.
- `info.rs`:
  - query `/index/<proof-ident>`;
  - drop the prefix==signer check;
  - run the verifier with `FirstUse`;
  - `IdentState` is computed against the publisher's chain (`toBinding` or
    `index` binding).

**Risk.** Ancestor trust in `FirstUse` mode accepts a rotated-away key of `O_0`.
That is equal to today's TOFU, since the registry chooses the bound key. In
`Pinned` mode trust is forward-only. A compromised old key of bob could forge an
accept only for what alice actually offered to bob.

The origin-owner fallback is no weaker than today. A registry that injects a fake
legacy link can only make later owners' releases fail (a denial, not a forgery).
The releases it still accepts are the origin owner's, under the trust every
client applies today.

**Rejected alternatives.**
- Pin the origin owner forever and verify every release by a chain walk at build
  time. That makes `mfb build` network-bound.
- Store the whole chain in the lock. That is a new lockfile format for no gain,
  because the pin is the end of a verified walk.

## Phases

> **NOTE: keep the checkboxes current as you go** (same rules as plan-168-A).

### Phase 1: Measure the suspected rotation defect

- [x] Write a `/tmp` probe (or a new `cli_repo_governance` test
      `repo_registry_add_of_a_pre_rotation_release_installs`):
      register alice, publish 0.1.0, `key rotate alice`, then a fresh consumer
      runs `pkg add alice#x@0.1.0`. Record the observed result in Corrections.
      If it fails, land it as a RED test (Phase 3 must turn it green). If it
      passes, strike this UNVERIFIED claim with the evidence.

Acceptance: the claim is resolved with output.
  Check: `cargo test --test cli_repo_governance repo_registry_add_of_a_pre_rotation_release_installs` → recorded pass/fail (est. 8–15 min cold release build; this is the only path that runs the registry add across a rotation).
  Measured: FAILED — `error: refusing to install `rot_pkg`@0.1.0: the package identKey does not match the identKey pinned in project.json`. Landed RED; Phase 3 must turn it green.
Commit: (this commit — recorded in the next)

### Phase 2: The pure verifier

- [ ] `repository/src/trust.rs` + `pub mod trust;` in `repository/src/lib.rs`;
      `chain_path` in `repository/src/client.rs`.
- [ ] Unit tests (`trust.rs`, fake chains, no network):
  - `pre_transfer_release_still_verifies`;
  - `recipient_release_verifies_through_the_chain`;
  - `chain_of_two_transfers_verifies` (alice→bob→carol, carol's release);
  - `forged_offer_is_rejected` (offer signed by a key not on alice's chain);
  - `forged_accept_is_rejected`;
  - `replayed_link_at_wrong_sequence_is_rejected`;
  - `withheld_release_breaks_the_digest`;
  - `rotation_before_and_after_transfer_verifies` (alice rotates before
    offering, bob rotates after accepting; releases signed by each key);
  - `legacy_link_terminates_the_chain` (a later owner's release past it is
    refused);
  - `a_legacy_link_with_a_foreign_from_owner_does_not_fail_pre_transfer_releases`;
  - `legacy_round_trip_origin_owner_release_verifies` (unsigned alice→bob→alice
    links, alice's epoch-2 release under her pin);
  - `round_trip_owner_pin_selects_its_first_tenure` (signed alice→bob→alice;
    alice never rotates, so her pin is in tenures 0 and 2; tenure 0 is chosen,
    and releases of all three epochs verify);
  - `a_pin_on_bob_verifies_alice_release_by_digest`;
  - `first_use_trusts_bound_key_ancestors_only_for_the_origin`;
  - `index_from_an_older_server_is_a_single_owner_chain`.

Acceptance: `cargo test -p mfb_repository --lib -- trust::` → `test result: ok`, 0 failed, with all fifteen tests listed (est. 3 min; guess).
Commit: —

### Phase 3: Wire every consumer

- [ ] `src/cli/pkg.rs`: `follow_identity_pin` (rename of `follow_rotated_pin`,
      rotation behavior preserved; the existing rotation integration test runs
      unchanged), and `add_package_from_registry`.
- [ ] `src/cli/resolve.rs`: `resolve` (verify the selected release and lock its
      key), `registry_dependency_for`, `apply_manifest_change` pin rewrite.
      Update the lock-key unit tests' expected key only where the test asserts
      `index.ident_key` semantics. Each such edit carries the AGENTS.md 4-answer
      proof: the old behavior is the bug-792 / Phase 1 defect.
- [ ] `src/cli/info.rs:verify_chain` + `RegistryLookup` (proof ident,
      `FirstUse` verifier); fake-registry tests in `src/cli/info/tests.rs`:
      `a_transferred_executable_reports_the_new_owner`,
      `a_foreign_ident_without_a_chain_is_refused`.

Acceptance: consumers compile against the verifier, and their unit tests pass.
  Check: `cargo test -p mfb --bin mfb -- cli::info cli::resolve cli::pkg` → `test result: ok`, 0 failed (est. 4–8 min; guess).
Commit: —

### Phase 4: Spec sync

- [ ] `01_repository-protocol.md`:
  - pin-follow paragraph (§Ident Rotation) covers transfers;
  - §Install Path: the client verifies `transfers` before pinning;
  - deviation paragraph drops "clients do not yet verify it".
- [ ] `03_signing.md`: `mfb info` reads the proof ident's index and the
      publisher's chain.
- [ ] `tooling/03_lockfile.md`: `identKey` = the verified key of the selected
      release.
- [ ] `tooling/01_project-manifest.md`: `identKey` pin semantics.

Acceptance: `cargo test --bin mfb spec` → `test result: ok` (est. 3–6 min; guess).
Commit: —

## Validation Plan

- Tests: Phase 2 (adversarial, pure), Phase 3 (consumers), Phase 1 (registry add
  across rotation).
- Runtime proof: D's end-to-end CLI test is the binding proof that real
  consumers accept bob's release and keep accepting alice's.
- Final gate (once): `cargo test -p mfb_repository --no-fail-fast > /tmp/168C-repo.log 2>&1; echo EXIT=$?` → `EXIT=0`; `cargo test --test cli_repo_install --test cli_repo_publish --test cli_repo_governance > /tmp/168C-cli.log 2>&1; echo EXIT=$?` → `EXIT=0` (est. 25–40 min; guess). These three integration binaries are every consumer path C changes.

## Open Decisions

- **C-D1, anchor trust.** Recommended: `Pinned` = forward-only, and `FirstUse` =
  the bound key plus its ancestors for the origin owner only. Alternative:
  ancestors always. Rejected, because a pinned consumer should not start trusting
  keys the owner rotated away from.
- **C-D2, pin semantics.** Recommended: pin := the verified identKey of the
  installed release, which may move backward across a transfer through the
  digest. Alternative: a monotone "newest owner" pin. Rejected, because it breaks
  `@<old version>` installs and offline builds of them.
- **C-D3, releases past a legacy link.** Recommended: the origin-owner
  fallback (§3). It keeps today's behavior for pre-fix round trips and refuses
  everything else. Alternative: refuse every release past the terminator.
  Rejected: it would break alice→bob→alice packages that publish and install
  today.

## Corrections

- **The suspected rotation defect is real** (§2 UNVERIFIED claim, Phase 1).
  `repo_registry_add_of_a_pre_rotation_release_installs` (register alice,
  publish 0.1.0, `key rotate alice`, fresh consumer `pkg add alice#rot_pkg@0.1.0`)
  fails at HEAD of this branch with `refusing to install `rot_pkg`@0.1.0: the
  package identKey does not match the identKey pinned in project.json`: the add
  pins `index.ident_key` (the rotated, current key) while the release header
  carries the pre-rotation key. Landed as a RED test.

## Summary

All of plan-168's trust risk lives here, isolated in one pure, adversarially
tested function. The consumers only swap their anchor source.
