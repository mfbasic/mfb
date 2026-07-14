# Audit 2 — Surface 8: Supply chain — install / resolve / registry client (compiler side)

Last updated: 2026-07-14
Untrusted party: a malicious or MITM'd registry, or a spoofed dependency source.
Must not: get an unverified or substituted package accepted, execute code at
install/resolve time, or downgrade/pin-bypass a dependency.

Scope read: `src/cli/{pkg,resolve,repo,init,mod}.rs`, `repository/src/{client,local,
lib}.rs`, cross-ref `src/cli/build.rs` (Surface 1 build-import verification) and
`repository/src/{crypto,package}.rs` (Surface 5).

## Executive result

The install/resolve path is **substantially hardened**; the "install is blind"
hypothesis is **refuted** (SUP-04). Both registry install paths run the full
plan-23 §3.5 signature chain before the blob is committed
(`install_verified_package` → `classify_installed_package`, `src/cli/mod.rs:63`),
and `mfb build` re-verifies every dependency against the pinned identKey
(`verify_and_report_packages`, `build.rs:1049`). Content blobs are
content-addressed and SHA-256-verified on download (`client.rs:942`). No package
code executes at install/resolve time. reqwest is built with `rustls-tls`; there
is **no** cert-verification bypass (no `danger_accept_invalid_certs`). Residual
findings are about trust bootstrap and version-list authenticity.

## Findings

### SUP-01 — LOW — Registry transport defaults to plaintext HTTP; no scheme enforcement
- Location: `repository/src/lib.rs:12` (`DEFAULT_REPO_URL = "http://127.0.0.1:7777"`);
  `client.rs:19-21` (`repo_url_from_env`); `client.rs:920-925,1005-1018` (URL scheme
  used verbatim; no `https` check).
- Threat/impact: with an `http://` registry (the default, or any `MFB_REPO_URL`),
  all registry traffic is cleartext — an on-path observer learns which
  packages/versions a build pulls, and an active MITM can tamper at will. Tampering
  alone does not yield a substituted package (the signature chain still gates
  acceptance), but plaintext is what makes the SUP-02 first-contact window
  exploitable by a mere on-path attacker rather than requiring a CA compromise.
- Best fix: require `https` for non-loopback registry URLs (allow `http` only for
  loopback), or warn loudly. Keep signatures as the authenticity anchor. Folded
  into **bug-189**.
- Non-goals: making TLS the trust anchor / weakening the signature chain.

### SUP-02 — MEDIUM — First-contact TOFU on the registry server key (MITM window at bootstrap)
- Location: `client.rs:26-34` (`ensure_server_key` fetches `/ident`, pins on first
  contact); `local.rs:240-254` (`pin_server_key` — `if path.is_file()`
  compare-else-write); consumed as the trust root by `fetch_index` verify
  (`client.rs:889-914`), `classify_installed_package` (`build.rs:1195-1214`), and
  resolve/add anchors (`pkg.rs:559`, `resolve.rs:109-117`).
- Threat/impact: the server key is the root of the §3.5 chain (it signs the
  owner→identKey name binding and every package attestation). On the *first*
  interaction with a registry URL, whatever `/ident` returns is pinned with no
  external check. An active MITM (trivial over the SUP-01 plaintext default)
  substitutes its own server key; it then serves a self-consistent name binding,
  attestation, and a package signed with its ident key — every downstream §3.5
  check passes because they all chain back to the attacker-pinned server key. A
  fully substituted package is accepted, cached, compiled, linked.
- Mechanism: `pin_server_key` (`local.rs:242`) trusts-on-first-use; the only closer,
  `mfb repo trust <registry-id> <root-fingerprint>` (`repo.rs:57-74` →
  `client.rs:625-642`), cross-checks against an OOB fingerprint but is opt-in, and
  `verify_pinned_metadata` no-ops when no root is pinned (`client.rs:648-651`). The
  default registry ships no baked-in root fingerprint.
- Reproduction: clean machine (no `~/.mfb/<hash>/server.pub`), point `MFB_REPO_URL`
  at an attacker answering `/ident` with key K, `/index/<owner>#<pkg>` with a name
  binding signed by K, and `/blob/<h>` with a package signed by attacker ident +
  attested by K; `mfb pkg add alice#toolbox`. Observed: package verifies (Verified),
  installs, attacker ident key pinned into `project.json`. Expected: bootstrap
  requires an OOB-verified anchor before the first package is trusted.
- Best fix: ship a pinned root fingerprint for the default registry and require
  `mfb repo trust` (or a baked-in pin) before the first install on an unpinned
  store; at minimum print the fetched fingerprint and require confirmation on first
  pin. → **bug-189**.
- Non-goals: post-bootstrap protection is already correct — a later server-key
  change is refused (`local.rs:244-249`); ident re-anchoring without a signed chain
  is a hard error (`pkg.rs:947-960`). This is strictly the first-contact window.

### SUP-03 — MEDIUM — `/index` version list is unauthenticated in the default path (downgrade / rollback)
- Location: version selection — `pkg.rs:593-631` (`select_index_version`),
  `resolve.rs:312-392` (`select_node`), `resolve.rs:179-216` (seeds from
  `fetch_index`); the only signed part of the index is the owner→identFingerprint
  name binding (`client.rs:905-914`) — `versions[]`, per-version `hash`, `state`,
  `published_at`, `abi_index` are **not** covered by any signature the default path
  verifies.
- Threat/impact: a malicious or MITM'd registry (post-SUP-02, or an
  honest-but-compromised registry with a legitimately-pinned key) can omit the
  newest version or mark it `yanked`/`blocked`, steering resolve/update/floating-add
  to an older, validly-signed, vulnerable version — a signature-preserving
  downgrade — or lie about `state` to make a good version look non-installable.
- Mechanism: `fetch_index` authenticates only the identKey binding
  (`client.rs:897-914`); it does not verify list completeness/freshness/per-version
  hashes against any signed manifest. Anti-rollback exists but is opt-in: the signed
  snapshot chain (`verify_registry_metadata`, `client.rs:489-663`) runs only when a
  root was pinned via `mfb repo trust`; transparency-log inclusion
  (`verify_publish_inclusion`, `client.rs:382-421`) only under `mfb pkg verify
  --proof`. The default install/update/add checks only `lock.repo_fingerprint`
  equality (`resolve.rs:112-117`) and per-blob content hash (`client.rs:942`) —
  neither detects a stale/truncated version list.
- Reproduction: with `alice#toolbox` at `1.0.0` (vulnerable) and `1.1.0` (fixed)
  both validly signed, have the registry return an index listing only `1.0.0` (or
  `1.1.0` as `yanked`); `mfb pkg update`. Observed: resolver selects `1.0.0`, it
  verifies and locks. Expected: a signed, monotonic snapshot/`indexHash` (or
  mandatory log-inclusion) prevents hiding `1.1.0`.
- Best fix: make the signed-metadata snapshot chain (or log-inclusion of the
  selected version against the monotonic checkpoint) **mandatory** on
  resolve/update/add; bind the index/version list under a server-signed `indexHash`
  and enforce `snapshotVersion` monotonicity by default. → **bug-189**.
- Non-goals: the registry cannot substitute a *different publisher's* code (the
  blob must still be signed by the pinned ident key) — this is version-selection
  integrity, not cross-publisher substitution.

### SUP-04 — Not demonstrated (documented positive) — "Install blindly trusts, relies solely on build-time verification"
- Refuted. Install is **not** blind: the full §3.5 chain runs at install time
  against the pinned/registry-vouched identKey (`build.rs:1159-1227`) before the
  blob is committed; anything less than `Verified` is fatal and the staged file is
  removed (`mod.rs:71-77`). Unsigned registry blobs are rejected at install, and
  again at build unless local or `--unsigned` (`build.rs:1091-1099`). Content bytes
  are SHA-256-checked on download (`client.rs:942`); the lock pins the resolved
  hash, ident key, and `repoFingerprint` (`resolve.rs:104-138`). Two independent
  verification points (install + build), both anchored on the pinned key.
- Additional checked-and-clean: **no install/resolve-time code execution** —
  resolution reads dependency import edges by pure binary decode
  (`resolve.rs:406-441`, `binary_repr::package_info_from_mfp`); no build scripts /
  post-install hooks / DOC-macro evaluation run against a fetched dependency;
  `build_project` compiles only the *local* project (`pkg.rs:129,340`).
  Path-traversal/symlink on cache write handled (`O_EXCL create_new` staging +
  `rename`, name validation, `mod.rs:21-58`; in-memory blob read, `resolve.rs:414-420`).
  `mfb init` fetches/trusts nothing remote.

## Trust-flow trace
`mfb pkg add <owner>#<pkg>` → `fetch_index` (name binding verified under
TOFU-pinned server key, SUP-02) → `select_index_version` (registry-controlled list,
SUP-03) → `fetch_blob` (SHA-256 verified) → `install_verified_package` →
`classify_installed_package` §3.5 (Verified required) → commit + pin identKey.
`mfb pkg update` → resolve (SUP-03) → `mfb.lock` (hash + identKey + repoFingerprint
+ checkpoint) → install. `mfb build` → `verify_and_report_packages` re-verifies
every dep. Net: byte-for-byte and cross-publisher-substitution defenses are strong;
the exploitable gaps are **bootstrap trust (SUP-02)** and **version-list
authenticity (SUP-03)**, with **plaintext transport (SUP-01)** as the enabler.

## Verdict

No CRITICAL/HIGH. Two MEDIUM (SUP-02 bootstrap TOFU, SUP-03 version-list downgrade)
+ one LOW (SUP-01 plaintext default), combined into **bug-189**. SUP-04 documents
the strong install/build verification that is already in place.
