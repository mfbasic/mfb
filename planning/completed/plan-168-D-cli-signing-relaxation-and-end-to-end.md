# plan-168-D: `mfb build --sign` accepts a transferred ident; end-to-end proof and close-out

Last updated: 2026-10-04
Effort: medium (1h–2h)
Depends on: plan-168-C

The last piece of bug-792. `src/cli/build/signing.rs:signing_ident` stops
rejecting a manifest ident whose prefix differs from the signing owner. The
registry's `/signing` already refuses an attestation to anyone but the stored
owner (`server.rs:require_package_owner`), so the local check only blocked the
legitimate recipient.

D then proves the whole feature through the shipped binaries, removes the spec's
deviation paragraph, and closes bug-792.

Outcome: `mfb repo publish bob <dir>` with `"ident": "alice#xfer_pkg"` publishes
0.2.0 after the transfer. A fresh consumer's `mfb pkg add alice#xfer_pkg`
installs 0.2.0 verified, and `@0.1.0` installs alice's release verified.

References: plan-168-A §3; `bugs/bug-792-*.md`; `03_signing.md` §build step 1;
`01_repository-protocol.md` deviation paragraph.

## Prerequisites

See plan-168-A, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-168-C complete | `grep -c '^Commit: —' planning/plan-168-C-*.md` → `0` (or archived) | MET (archived in `e48c24b69`) |

## 1. Goal

- `signing_ident(owner, name, "alice#pkg")` returns `Ok("alice#pkg")` for any
  well-formed ident. Shape errors are unchanged.
- The full transfer scenario passes through the CLI.

### Non-goals

- No change to `/signing`. It remains the authority.
- No change to the executable signing metadata format (`author = owner`).

## 2. Current State

- `src/cli/build/signing.rs:signing_ident` rejects with "does not belong to
  owner" when the prefix ≠ the owner (case-insensitive). Its single caller is
  `src/cli/build/mod.rs` (`build_project`); see
  `grep -rn 'signing_ident' src/cli` → 1 call site + 2 tests.
- Tests asserting the rejection (`src/cli/build/mod.rs`):
  - `signing_ident_rejects_bad_idents` (the `bob#shape` assert);
  - `build_project_sign_rejects_a_foreign_manifest_ident` (manifest ident
    `ada#app`, `--sign zed`). After relaxation it still fails, but for an
    unrelated reason: `load_build_signing_info` reads zed's ident key from the
    real `local_paths_for_repo(repo_url_from_env())`. The test would then depend
    on the machine's key store and registry environment, and would no longer
    reach the signing error arm "before any registry/key access", which is what
    its doc comment says it covers.
- CLI harness: `tests/cli/cli_repo_governance.rs:repo_ownership_transfer_is_two_sided_and_rebinds_the_package`
  (transfer) and `tests/cli/cli_repo_publish.rs:repo_registry_add_installs_and_verifies_from_index`
  (add/install).

## 3. Design

- `signing_ident` keeps the empty-ident default and the `<owner>#<package>`
  shape check, and drops the owner comparison.
- Edits to the tests that asserted the old rule, each with its AGENTS.md proof:
  - Remove the `bob#shape` assert. (1) Added by the plan-12 coverage wave
    (`git log -S 'bob#shape' --oneline | tail -1` → `b20ab6b56`). (2) It
    protects "you can only sign your own prefix". (3) Only the `03_signing.md`
    step-1 text and the `01_repository-protocol.md` deviation paragraph depend
    on it (`grep -rn 'signing_ident' src/docs`), both updated here. (4) bug-792
    plus `require_package_owner` show the rule wrongly blocks a stored owner.
  - Add `signing_ident_accepts_a_transferred_ident`.
  - Repurpose `build_project_sign_rejects_a_foreign_manifest_ident` as
    `build_project_sign_rejects_a_malformed_manifest_ident`. Change its manifest
    ident from `ada#app` to `ada-app` (no `#`), so it again fails inside
    `signing_ident` before any key or registry access. Update its doc comment;
    `assert!(build_project(&options).is_err())` is unchanged. (1) Added by the
    plan-68 residual coverage backfill
    (`git log -S 'build_project_sign_rejects_a_foreign_manifest_ident' --oneline | tail -1`
    → `e081c97cd`). (2) It protects "a bad `--sign` ident fails in
    `build_project`'s signing error arm without touching keys or the registry".
    (3) Nothing else references it (`grep -rn 'build_project_sign_rejects' src tests`
    → only its definition). (4) bug-792: a foreign prefix is no longer an error,
    so the old input no longer reaches the arm. The shape error still does.

## Phases

> **NOTE: keep the checkboxes current as you go** (same rules as plan-168-A).

### Phase 1: Relax `signing_ident`

- [x] `src/cli/build/signing.rs:signing_ident` per §3 (also update its doc
      comment, which says the ident "must belong to the signing owner");
      `src/cli/build/mod.rs` tests per §3.

Acceptance: `cargo test -p mfb --bin mfb -- signing_ident build_project_sign_rejects_a_malformed_manifest_ident` → `test result: ok`, 0 failed (est. 3–6 min; guess).
  Measured: `test result: ok. 4 passed; 0 failed`.
Commit: bb166a318

### Phase 2: End-to-end CLI scenario

- [x] `tests/cli/cli_repo_governance.rs`: new test
      `repo_transfer_recipient_publishes_and_consumers_verify_both_owners`:
  1. alice publishes 0.1.0;
  2. a consumer adds it (pin = alice);
  3. alice transfers to bob, and bob accepts;
  4. `mfb repo publish bob <dir>` (manifest ident `alice#xfer_pkg`, version
     0.2.0) succeeds;
  5. the consumer runs `pkg update` → 0.2.0 `[Verified]` with the transfer
     notice; `mfb build` passes offline;
  6. a second fresh consumer adds `alice#xfer_pkg@0.1.0` → `[Verified]`;
  7. alice's attempt to publish 0.3.0 fails;
  8. bob rotates his ident, publishes 0.3.0, and the first consumer updates
     `[Verified]`;
  9. bob transfers to carol, carol publishes 0.4.0, and the consumer updates
     `[Verified]`.
- [x] Extend `repo_ownership_transfer_is_two_sided_and_rebinds_the_package` with
      one assertion: the `/index` transfer link exists. Additive, no assertion
      weakened.

Acceptance: the scenario passes through shipped binaries.
  Check: `cargo test --test cli_repo_governance repo_transfer_recipient_publishes_and_consumers_verify_both_owners` → `1 passed` (est. 10–15 min: release build plus three accounts. This is the bug's own Validation Plan requirement and nothing smaller exercises build→publish→install).
  Measured: `1 passed` (and the whole binary `9 passed`). With `signing.rs`
  restored to its pre-Phase-1 text the same test fails at step 4 with
  `project ident `alice#xfer_pkg` does not belong to owner `bob`` — the bug's
  documented build-side mechanism.
Commit: ab3cc833c

### Phase 3: Spec close-out and bug closure

- [x] `01_repository-protocol.md`: delete the "Current implementation deviation"
      paragraph under §Accounts — Transfers. The transfer contract paragraph now
      describes shipped behavior; check that every cited symbol resolves.
- [x] `03_signing.md` build step 1: "a declared ident is used as-is; the registry
      issues the attestation only to the package's stored owner (or, for a new
      ident, its prefix owner)".
- [x] Measure the remaining stale prose:
      `grep -rn 'prefix' src/docs/spec/package-manager | grep -i 'transfer\|signing_ident'`
      → fix every hit that describes pre-fix behavior. Record the count in Corrections.
- [x] Update `bugs/bug-792-*.md`: Status Fixed, Regression Test =
      `repository/src/server.rs:tests::spec_probe_transfer_recipient_can_publish`,
      the Phase commits. Move it to `bugs/completed/`.

Acceptance: the spec has no deviation for transfers, and citations resolve.
  Check: `grep -c 'Current implementation deviation' src/docs/spec/package-manager/01_repository-protocol.md` → one fewer than before this phase (record both numbers), and `cargo test -p mfb --bins citations_resolve` → `test result: ok` (est. 4–6 min; guess).
  Measured: `2` before, `1` after (the remaining one is the signed-metadata
  `/index` binding deviation, unrelated); `cargo test -p mfb --bins
  citations_resolve` → `test result: ok. 1 passed`; `cargo test --bin mfb
  spec` → `46 passed`.
Commit: 1ba768a08

## Validation Plan

- Tests: D Phase 1–2, plus everything A–C added.
- Coverage: the CLI test runs the release `mfb` and `mfb-repo` (`tests/common/mod.rs:mfb_exe`, `repo_exe`).
- Runtime proof: Phase 2.
- Doc sync: Phase 3, plus A4, B3 and C4.
- Final gate for all of plan-168, run ONCE here:
  `cargo test --no-fail-fast -- --skip artifact_gate_all > /tmp/168-full.log 2>&1; echo EXIT=$?` → `EXIT=0`
  (whole workspace per root `Cargo.toml` `default-members`; `.ai/testing-gates.md`
  form). Then `cargo fmt --all -- --check` → no output. Estimated 60–120 min
  (guess). The artifact gate is skipped because no plan-168 letter touches codegen
  or runtime; state that in the landing commit.

## Open Decisions

- **D-D1.** Recommended: `signing_ident` checks shape only, and the registry's
  ownership decides. Alternative: query `/index` locally and require a verified
  chain to the signer before building. Rejected, because it duplicates the
  server's authoritative check and makes `build --sign` fail for a first publish
  before any package row exists.

## Corrections

- **Stale prefix prose (Phase 3 measurement).** `grep -rn 'prefix'
  src/docs/spec/package-manager | grep -i 'transfer\|signing_ident'` → 3 hits;
  1 described pre-fix behavior (the deviation paragraph, deleted) and 2 are
  accurate (the stored-owner offer refusal; the origin-owner rule past a legacy
  link). The grep misses wording without "prefix": three more stale statements
  were fixed — `tooling/01_project-manifest.md`'s `ident` row and
  `tooling/07_cli-reference.md`'s `--sign` paragraph ("must belong to the
  signing owner"), and the `mfb info` chain description there, which now covers
  a transferred ident.
- **Phase 2 step 5's "`[Verified]`" is asserted on `mfb build`** (`uses xfer_pkg
  - [Verified]`) and on `pkg verify`; `pkg update` itself prints the lock diff,
  the notice and the installs, not a verification verdict. "`mfb build` passes
  offline" is asserted as a build that verifies against the pin alone (the build
  never contacts the registry); the registry is not stopped mid-test.

## Summary

A one-function relaxation, gated by the server check that already exists, plus
the end-to-end proof that closes bug-792.
