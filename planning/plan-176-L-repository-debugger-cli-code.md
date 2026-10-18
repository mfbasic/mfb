# plan-176-L: Repository, debugger, CLI and manifest code

Last updated: 2026-10-18
Effort: large (3h–1d)
Depends on: plan-176-K

L splits the eleven over-budget CODE files in the package-repository crate (`repository/src`),
the debugger (`src/debugger`), the package CLI (`src/cli/pkg.rs`, `src/cli/resolve.rs`) and the
manifest validator (`src/manifest/mod.rs`). Each `foo.rs` becomes `foo/mod.rs` plus part files
grouped by responsibility; `src/manifest/mod.rs` gains two sibling parts. The work is pure code
motion: items move between files, `impl` blocks are cut into one block per file, and private
items that now cross a file boundary become `pub(super)`. The test code of these files was
already moved into `tests/` directories by letters I (repository) and K (the rest) — every one
of them carried more than the 250 inline test LOC the rule allows; L leaves those declarations
in the new `mod.rs` files.

The single outcome: after L lands, `cargo test --test file_size_budget` shows no row for any of
L's files (old or new paths) and `tests/guards/file_size_ceilings.txt` has no `ceiling` row for
them. There are no exemptions (rule revised 2026-10-18).

References:

- `planning/plan-176-A-file-size-guard.md` — the rule, §Decisions, "Splitting an over-budget
  file" (`.ai/build-tooling.md`: `foo.rs` → `foo/mod.rs` + parts, ≤ 900 per part, `mod.rs`
  re-exports the surface, sweep `[[foo.rs:` citations, inherit coverage exceptions).
- `planning/plan-176-I-repository-tests.md` and `planning/plan-176-K-frontend-tooling-tests.md`
  — where the test modules of these files went.
- `.ai/net-tls.md` (names `ensure_transport_security` in `repository/src/client.rs`),
  `.ai/testing-gates.md` (names `repository/src/client.rs`), `.ai/build-tooling.md` (names
  `check_libraries` in `src/manifest/mod.rs`), `.ai/resources-packages.md` (package subsystem),
  `.ai/remote_systems.md` (box 2223), `.ai/sub-agents.md`.
- Spec topics whose citations move: `package-manager/01_repository-protocol.md`, `02_key-store.md`, `03_signing.md`,
  `04_owner-names.md` (repository); `tooling/03_lockfile.md`, `tooling/06_doc-html.md`,
  `tooling/07_cli-reference.md`, `architecture/03_packages.md`,
  `package/03_metadata-encoding.md` (cli); `tooling/10_debugger.md` (debugger);
  `tooling/01_project-manifest.md`, `tooling/02_source-selection.md`,
  `architecture/02_frontend.md`, `package/15_package-meta-section.md` (manifest).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-K is complete and archived | `ls planning/plan-176-K-* planning/completed/plan-176-K-*` → only `planning/completed/…` | NOT MET (2026-10-14) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | NOT MET (2026-10-14) |
| L's files have no uncommitted changes | `git status --porcelain -- repository/src src/cli src/debugger src/manifest scripts/coverage-exceptions.txt` → empty | MET (2026-10-14) |
| Box 2223 (Kali aarch64 glibc) has cargo | `ssh -p 2223 test@127.0.0.1 'command -v cargo'` → `/usr/bin/cargo` | MET per `.ai/remote_systems.md` (verified 2026-09-10); re-run before Phase 4 |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify — a
> prerequisite recorded NOT MET may have landed since, and one recorded MET may
> have regressed.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you. The reader needs to know how far off the gate is,
> not just which row you hit first.

## 1. Goal

- Every file in §2's table and every part L creates is ≤ 1000 code LOC (largest planned part:
  `repository/src/store/accounts.rs` 797) and ≤ 250 inline test LOC
  (`python3 -I tools/loc-census/census.py --file <path>`).
- Every external path keeps resolving: `mfb_repository::server::IndexResponse`,
  `crate::store::Store`, `crate::cli::pkg::install_vendor_blobs`,
  `crate::manifest::check_libraries`, `prompt::{address, number, table, …}`, etc.
- Every spec citation to a moved symbol names the file that now defines it.

### Non-goals (explicit constraints)

- No logic edit. The only non-motion edits are: `pub(super)` on items that cross a part
  boundary, `mod`/`use` wiring lines, the store key-writer test's file read (§4.3), the
  `command_tests` declaration (§4.4), coverage-exception lines, and doc/comment repoints.
- No public API change of the `mfb_repository` crate; no wire format, database schema, CLI
  flag, message text or exit status changes.
- No test added, removed or renamed. No `[[test]]` target renamed.
- No visibility widened beyond `pub(super)` (L moves nothing one level deeper whose caller is
  above the parent).

## 2. Current State

Counts: `python3 -I /tmp/loc/span.py <file>` on the working tree of `17a9b5799` (prints
`(code, test)`); after plan-176-A, `python3 -I tools/loc-census/census.py --file <path>`. Part
LOC: the split map's `xref.py` sum of code lines over the listed ranges (each file's part sums
equal its total). Citation counts: `grep -rnoE "\[\[<path>(:[^]]+)?\]\]" src/docs/spec | wc -l`.

| File | Code | Test before I/K | Parts after L | Cites |
|---|---|---|---|---|
| `repository/src/client.rs` | 1445 | 2838 (I) | mod + 5 | 51 |
| `repository/src/server.rs` | 3056 | 7487 (I) | mod + 10 | 74 |
| `repository/src/store.rs` | 3423 | 3408 (I) | mod + 7 | 39 |
| `src/cli/pkg.rs` | 2019 | 899 (K) | mod + 6 | 38 |
| `src/cli/resolve.rs` | 1016 | 600 (K) | mod + 3 | 20 |
| `src/debugger/breakpoints.rs` | 1271 | 945 (K) | mod + 2 (+ existing `report.rs`) | 11 |
| `src/debugger/linux.rs` | 1708 | 0 | mod + 4 | 13 |
| `src/debugger/macos.rs` | 1980 | 0 | mod + 4 | 12 |
| `src/debugger/prompt.rs` | 2041 | 1922 (K) | mod + 4 | 28 |
| `src/debugger/values.rs` | 1068 | 501 (K) | mod + 2 | 4 |
| `src/manifest/mod.rs` | 1092 | 1002 (K) | mod + 2 | 38 |

After I and K, each host's test module is a `#[cfg(test)] mod tests;` declaration pointing at
`<host dir>/tests/mod.rs`. That declaration resolves identically from `foo.rs` and from
`foo/mod.rs`, so L moves it into `mod.rs` unchanged. No L part file is named `tests`.

### Verified properties

- **Glob re-exports keep paths.** `mod.rs` declares `mod <part>;` and re-exports with
  `pub use <part>::*;` (`pub(crate) use` in `src/` where the items are `pub(crate)`); a glob
  forwards each item at its own visibility, so `crate::server::RegisterRequest` and
  `super::pkg::*` users are unchanged. Each part starts with `use super::*;`. Glob cycles of
  this shape compile without warnings: probe `/tmp/loc/probe2` printed `6`, no warning.
- **Privacy is descendant-visible.** Items left in `mod.rs` need no edit to be used from a part
  or from `tests/`. Only a part's private item used by a sibling, by `mod.rs` or by the tests
  needs `pub(super)`. The counts in §4 come from the map's whole-word cross-reference
  (`/tmp/loc_lm/xref.py`), minus trait methods and local-name collisions it reports.
- **A split `impl X { … }` is legal as one `impl X` block per file.** Inherent impls may be
  spread over any number of blocks in the same crate.
- **cfg gating.** `src/debugger/mod.rs:28-29`: `#[cfg(all(target_os = "linux", target_arch = "aarch64"))] mod linux;`
  — children inherit the gate, so `linux/` compiles only on aarch64 Linux: box 2223 (Kali
  aarch64 glibc, `/usr/bin/cargo`, `.ai/remote_systems.md`). `src/debugger/mod.rs:32-33`:
  `#[cfg(target_os = "macos")] mod macos;` — compiles on the macOS host. The only other gated
  code in L's files is `harden_private_state` in `repository/src/store.rs`
  (`#[cfg(unix)]` 173/204, `#[cfg(not(unix))]` 225), which stays in `store/mod.rs`
  untouched, so no Windows check is needed
  (`grep -c 'cfg(windows)\|target_os = "windows"' <L files>` → 0 in every other file).
- **No source-location dependence.** `grep -rnE 'module_path!|\bfile!\(|\bline!\(' src repository/src`
  → no hits (plan-176-A). The one `include_str!` in L's scope is the store test's (§4.3);
  `repository/src/server.rs:1289` only mentions `include_str!` in a doc comment.
- **The store key-writer test reads one file.** `key_insertion_has_exactly_one_writer`
  (introduced by `e18cbb6fa`, 2026-09-12, bug-580) counts
  `INSERT INTO keys (owner_id, role, public_key` in `include_str!`'d store source and asserts 1
  (`grep -c "INSERT INTO keys (owner_id, role, public_key" repository/src/store.rs` → 1, in
  `insert_key_tx`). After plan-176-I it lives in one of the `repository/src/store/tests/*.rs`
  parts (the moved store tests are 3468 LOC, over the 1000 test-only limit, so I splits them;
  `grep -rln key_insertion_has_exactly_one_writer repository/src/store/tests` names the part) and
  includes `../../store.rs`. After L that writer is in `store/accounts.rs`, and `store/mod.rs`
  holds 0.
- **Coverage exceptions match by exact path.** `scripts/coverage-check.sh` compares each
  file's repo-relative path against the set read from `scripts/coverage-exceptions.txt`;
  lines 17-18 name `src/cli/pkg.rs` and `src/cli/resolve.rs`. A split file whose parts are not
  listed loses its exception.
- **Live-registry calls by part** (`grep -n 'client::' src/cli/pkg.rs src/cli/resolve.rs`
  mapped onto §4's ranges): pkg — `mod.rs` (old 60, 63), `publish.rs`, `update.rs`, `add.rs`,
  `verify.rs`, `info.rs` (old 2165, 2168); none in `remove.rs`. resolve — `mod.rs` (old
  615-1063) and `closure.rs` (old 278-292); none in `lockfile.rs` or `drift.rs`.
- **Stale references found by the map** (pre-existing; fixed in L):
  - `.ai/testing-gates.md:703` says `pin_server_key` is in `repository/src/client.rs`; it is
    `repository/src/local.rs:377` (`grep -rn "fn pin_server_key" repository/src`).
  - `tests/cli/cli_repo_install.rs:637` cites `src/cli/pkg.rs:566` and `:639` cites
    `src/cli/resolve.rs:253`; `src/cli/resolve.rs:556` cites `src/cli/pkg.rs:1140`. None of
    those lines is the code described (`sed -n '566p;1140p' src/cli/pkg.rs`,
    `sed -n 253p src/cli/resolve.rs`). The described code is `add_package_from_file` (copies
    the ident from the `.mfp` header) and, today, `is_registry_dependency`
    (`src/cli/resolve.rs:573`, which replaced the `.filter(|dep| dep.ident.contains('#'))` the
    spike comment quotes). After plan-176-E (the file is 1241 LOC, over the 1000 test-only
    limit, so E splits it into a directory) the comment sits in one of
    `tests/cli/cli_repo_install/*.rs` (`grep -rn 'src/cli/pkg.rs:566' tests/cli` finds it).
  - `src/docs/spec/tooling/10_debugger.md:69` names `src/debugger/breakpoints.rs` as a path.
  - `.ai/build-tooling.md:84` cites `src/manifest/mod.rs` "~1049" for `check_libraries`.

## 3. Design Overview

One convention per file (plan-176-A): `foo.rs` → `foo/mod.rs` (the module's `use` header, its
core types, and the declarations `mod <part>; pub use <part>::*;` plus the existing
`#[cfg(test)] mod tests;`) + part files by responsibility. Line ranges below are source lines
as of `17a9b5799`; item NAMES are authoritative if lines drift (I and K removed only the
trailing test modules, which sit below every listed range, except `src/manifest/mod.rs`'s
inline `canvas_auto_paint_tests` at 171-235, which K left in place).

Gate class: code motion outside codegen. None of these files emits code (repository server
and client, debugger, package CLI, manifest validation), so byte-identity of compiler output
is not the gate; the crate and module tests that exercise the moved code are, plus
`cargo check --all-targets` / `cargo clippy --all-targets` with no new warning,
`spec_citations_resolve` and the three guards. The aarch64-Linux debugger is checked on box
2223.

Risk concentrates in:
- **`repository/src/store.rs`** — one `impl Store` of 2829 LOC cut into seven blocks, and the
  one test whose logic depends on the file layout (§4.3).
- **`src/debugger/linux.rs`** — compiled on no developer host; only box 2223 proves it.
- **Coverage exceptions** — lines 17-18 of `scripts/coverage-exceptions.txt` must follow the
  code that has the live-registry boundary; the pure parts get no line.

Rejected alternatives:
- *Keeping `foo.rs` and adding `#[path]` siblings* — the citation forcing function (old path
  disappears, `spec_citations_resolve` fails until swept) only works when the file moves.
- *Explicit per-name re-exports* — server alone would need 58 cited names plus the handlers;
  globs keep one line per part.
- *One exception line for the whole `src/cli/pkg/` directory* — `coverage-check.sh` matches
  exact paths; a prefix rule would be a new gate semantics, outside plan-176.
- *Copying the store writer test's needle into every part's test* — the test's job is "one
  writer across the store"; reading every part keeps it one assertion.

## 4. Detailed Design

Shared wiring for every entry: parts begin `use super::*;`; part-specific `use` lines are added
only where `cargo check` reports an import missing or unused. **Visibility rule:** the items
named under "pub(super)" are the measured set; any further private item or private field the
compiler reports as inaccessible across a part boundary gets `pub(super)`, nothing wider.

**Citation sweep rule:** every `[[<old path>:Sym]]` becomes `[[<new path>:Sym]]` where
`<new path>` is the file listed for `Sym` below (symbols that stay in `mod.rs` also change
path, `foo.rs` → `foo/mod.rs`). Check per file:
`grep -rhoE '\[\[<new dir>/[^]]+\]\]' src/docs/spec | sort -u | sed 's/^\[\[//;s/\]\]$//' | while IFS=: read -r p s; do [ -n "$s" ] && ! grep -qw -- "${s##*::}" "$p" && echo "MISSING $p:$s"; done`
→ no output, and `grep -rn '\[\[<old path>' src/docs/spec` → 0.

### 4.1 `repository/src/client.rs` → `repository/src/client/` (1445)

- `mod.rs` (20): `use` header (1-20), the `mod`/`pub use {transport, identity, registry_trust, governance, packages}::*;`
  lines, `#[cfg(test)] mod tests;` (I).
- `transport.rs` (257; 24-352, 1059-1073, 1862-1979): `CONNECT_TIMEOUT`, `CONTROL_TIMEOUT`,
  `BLOB_TIMEOUT`, `MAX_JSON_BYTES`, `MAX_BLOB_BYTES`, `http_client`, `no_redirect_client`,
  `repo_url_from_env`, `ensure_transport_security`, `MAX_REDIRECTS`, `redirect_policy`,
  `ensure_redirect_target`, `resolve_redirect_host`, `ensure_redirect_host_resolves_public`,
  `is_blocked_redirect_ip`, `percent_encode`, `post_json`, `get_json`, `read_body_capped`,
  `read_json_response`, `sanitize_server_text`.
- `identity.rs` (389; 353-868): `ensure_server_key`, `fetch_server_key`, `register`, `auth`,
  `request_attestation`, `link_start`, `link_fetch`, `rotate_ident`, `PendingIdent`,
  `recover_pending_ident`, `fetch_ident_chain`, `registry_chains`, `follow_ident_chain`,
  `chain_path`, `revoke_machine`.
- `registry_trust.rs` (327; 869-1058, 1074-1309): `fetch_checkpoint_unpinned`,
  `fetch_checkpoint`, `publish_leaf_payload`, `verify_publish_inclusion`,
  `verify_log_consistency`, `decode_hex32`, `DelegatedMetadata`, `verify_registry_metadata`,
  `parse_metadata`, `check_field`, `check_not_expired`, `metadata_i64`, `metadata_str`,
  `decode_delegated_key`, `trust_registry`, `verify_pinned_metadata`,
  `fetch_and_verify_metadata`. (Named so it does not read as the crate-level `crate::trust`;
  `client.rs` never writes a bare `trust::` path.)
- `governance.rs` (275; 1310-1625): `set_org_member`, `issue_publish_token`,
  `revoke_publish_token`, `verify_owner_binding`, `current_owner_binding`,
  `ClientTransferTerms`, `client_transfer_terms`, `transfer_offer`, `transfer_accept`,
  `set_release_state`.
- `packages.rs` (177; 1626-1861): `fetch_index`, `fetch_index_with_key`, `fetch_blob`,
  `blob_exists`, `put_blob`, `PackageArtifact`, `validate_package`, `publish_package`,
  `package_request`.

pub(super) (25): `BLOB_TIMEOUT`, `CONTROL_TIMEOUT`, `MAX_BLOB_BYTES`, `MAX_JSON_BYTES`,
`check_field`, `check_not_expired`, `decode_delegated_key`, `decode_hex32`,
`ensure_redirect_host_resolves_public`, `ensure_redirect_target`, `ensure_transport_security`,
`get_json`, `http_client`, `is_blocked_redirect_ip`, `metadata_i64`, `metadata_str`,
`no_redirect_client`, `package_request`, `parse_metadata`, `percent_encode`, `post_json`,
`publish_leaf_payload`, `read_body_capped`, `resolve_redirect_host`, `sanitize_server_text`.

Citations (51 cites, 32 symbols, `package-manager/0{1,2,3,4}_*.md`):
transport — `CONNECT_TIMEOUT`, `CONTROL_TIMEOUT`, `BLOB_TIMEOUT`,
`ensure_redirect_host_resolves_public`, `ensure_transport_security`, `no_redirect_client`,
`post_json`, `read_body_capped`, `read_json_response`, `redirect_policy`, `repo_url_from_env`,
`sanitize_server_text`; identity — `auth`, `chain_path`, `ensure_server_key`,
`fetch_server_key`, `link_fetch`, `recover_pending_ident`, `register`, `request_attestation`,
`rotate_ident`; registry_trust — `fetch_checkpoint`, `verify_pinned_metadata`,
`verify_publish_inclusion`, `verify_registry_metadata`; governance — `transfer_offer`;
packages — `fetch_blob`, `fetch_index`, `fetch_index_with_key`, `package_request`,
`publish_package`, `validate_package`.

Other refs: `.ai/net-tls.md:266` `repository/src/client.rs` → `repository/src/client/transport.rs`.
`.ai/testing-gates.md:703`: the bullet's path becomes `repository/src/local.rs`
(`pin_server_key`'s real home, §2) and its `client::tests` mention stays (that module path is
unchanged by I and L). No guard, include or coverage entry.

### 4.2 `repository/src/server.rs` → `repository/src/server/` (3056)

- `mod.rs` (138; 1-26 `use` header + `AppState`, 927-1036 `serve`, `build_router`, `health`,
  3850-3894 `bad_request`, `conflict_or_bad_request`, `not_found`, `internal`,
  `unauthorized`), `pub use <each part>::*;`, `#[cfg(test)] mod tests;` (I).
- `limits.rs` (59; 27-86, 833-891, 909-926): `RateLimiter` + impl, `too_many_requests`,
  `MAX_BODY_BYTES`, every `*_PER_IP_MAX`, `*_PER_OWNER_MAX`, `AUTH_GLOBAL_CEILING`,
  `SEARCH_LIMIT_*`, `SEARCH_DESCRIPTION_PREVIEW_CHARS`, `MAX_VERSIONS_PER_OWNER`.
- `wire.rs` (644; 87-808 every pub request/response DTO from `RegisterRequest` to
  `HealthResponse` plus `impl IndexVersion`, 1192-1226 `SearchResponse`, `SearchResult`,
  1485-1609 `PackageDocsResponse`, `DocPageResponse`, `DocProseResponse`, `DocGroupResponse`,
  `DocDeclResponse`, `DocEntryResponse`, `impl PackageDocsResponse::from_view`).
- `auth.rs` (148; 809-827 `SESSION_TOKEN_ISSUER`, `SESSION_TOKEN_AUDIENCE`, `SessionClaims`;
  2140-2239 `publish_token_scope`, `require_package_owner`, `session_and_ident`; 2481-2524
  `scope_owner_matches`, `scope_permits`, `governing_owner`; 3834-3849 `verify_session_token`;
  3895-3910 `bearer_token`).
- `accounts.rs` (483; 1037-1128 `server_ident`, `register`, `challenge`; 2901-3367
  `rotate_ident`, `ident_chain`, `link_start`, `link_fetch`, `revoke_challenge`,
  `revoke_machine`, `login`, `attestation_json`, `json_str`, `signing`).
- `log_proofs.rs` (107; 1129-1184 `log_checkpoint`, `ProofQuery`, `log_inclusion_proof`;
  1949-2010 `ConsistencyQuery`, `log_consistency_proof`, `PublishEntryQuery`,
  `log_publish_entry`). (Named so it does not read as `crate::log`.)
- `browse.rs` (590; 892-908 `description_preview`; 1185-1191 `SearchQuery`; 1227-1484
  `search`, `registry_identity`, `stylesheet`, `landing_page`, `search_html`,
  `package_docs_html`, `DocsLookupError`, `lookup_package_docs`, `package_docs`; 1610-1948
  `package_page_html`, `package_audit_html`, `split_ident`, `package_detail`,
  `package_audit`; 2011-2114 `package_index`).
- `metadata.rs` (87; 2115-2139 `metadata_key`, `SNAPSHOT_TTL_SECS`, `TIMESTAMP_TTL_SECS`;
  2525-2618 `root_metadata`, `snapshot_metadata`, `timestamp_metadata`,
  `metadata_uninitialized`).
- `governance.rs` (283; 2240-2480 `org_members`, `issue_token`, `revoke_token`,
  `transfer_offer`, `transfer_accept`; 2619-2712 `release_state`).
- `blobs.rs` (154; 2713-2900): `validate_blob_hash`, `resolve_blob_kind`, `package_blob`,
  `head_blob`, `put_blob`.
- `publish.rs` (363; 828-832 `LEGACY_TRANSFER_PUBLISH_REFUSED`; 3368-3833 `validate_package`,
  `publish_package`, `validate_package_request`, `invalid_report`).

Callers that must keep working: `repository/src/client.rs` parts (`crate::server::{…DTOs}`),
`src/cli` (`mfb_repository::server::{IndexResponse, ValidatePackageResponse, …}`),
`repository/src/main.rs` (`server::serve`, `verify_session_token`, `attestation_json`).

pub(super) (77; handler fns and limits named by `build_router` or by the tests): limits —
`AUTH_GLOBAL_CEILING`, `BLOB_UPLOAD_PER_OWNER_MAX`, `CHALLENGE_PER_IP_MAX`,
`LOGIN_PER_IP_MAX`, `LOG_PER_IP_MAX`, `MAX_BODY_BYTES`, `MAX_VERSIONS_PER_OWNER`,
`PUBLISH_PER_OWNER_MAX`, `REGISTER_PER_IP_MAX`, the 4 `SEARCH_*`, `SIGNING_PER_IP_MAX`,
`VALIDATE_PER_OWNER_MAX`, `RateLimiter` + `new`/`allow`/`prune`, `too_many_requests`;
log_proofs — `ProofQuery`, `ConsistencyQuery`, `PublishEntryQuery` and their fields (tests
construct them); auth — `SESSION_TOKEN_*`, `bearer_token`, `governing_owner`,
`publish_token_scope`, `require_package_owner`, `scope_owner_matches`, `scope_permits`,
`session_and_ident`; wire — `PackageDocsResponse::from_view`; handlers — 11 in accounts, 4 in
log_proofs, 9 in browse, 4 in metadata, 6 in governance, 4 in blobs, 3 in publish, plus
`json_str` and `metadata_key`.

Citations (74 cites, 58 symbols, `package-manager/01`, `03`, `04`): wire — `RegisterRequest`,
`RegisterResponse`, `ChallengeRequest`, `ChallengeResponse`, `LoginRequest`, `LoginResponse`,
`SigningRequest`, `SigningResponse`, `TransferLink`, `PackageArtifactRequest`,
`ValidatePackageResponse`, `PublishPackageResponse`, `PackageDocsResponse`, `DocPageResponse`,
`DocDeclResponse`, `from_view`; limits — `RateLimiter`, `MAX_BODY_BYTES`; auth —
`SessionClaims`, `verify_session_token`, `require_package_owner`, `scope_permits`,
`governing_owner`; accounts — `server_ident`, `register`, `challenge`, `login`, `signing`,
`attestation_json`, `rotate_ident`, `ident_chain`, `link_start`, `link_fetch`,
`revoke_machine`; mod — `build_router`, `serve`, `conflict_or_bad_request`; browse —
`description_preview`, `search`, `lookup_package_docs`, `package_detail`, `package_audit`,
`package_index`; log_proofs — `log_checkpoint`, `log_inclusion_proof`,
`log_consistency_proof`, `log_publish_entry`; metadata — `root_metadata`,
`snapshot_metadata`, `timestamp_metadata`; governance — `org_members`, `issue_token`,
`release_state`; blobs — `package_blob`, `head_blob`, `put_blob`; publish —
`publish_package`, `validate_package_request`. Path refs: none
(`grep -rn repository/src/server.rs tests/guards scripts .ai AGENTS.md src/docs .github Cargo.toml`).

### 4.3 `repository/src/store.rs` → `repository/src/store/` (3423)

`impl Store` (235-3712, 2829 LOC) becomes one `impl Store { … }` block per file.
- `mod.rs` (282): 1-234 (`use` header, `Store`, `CachedLogHead`, `CHALLENGE_PURPOSE_*`,
  `KEY_ROLE_*`, `PublishMetadata`, `TargetRowForTest`, `OwnerRecord`, `KeyRecord`,
  `PublishedVersion`, `ChallengeRecord`, `NewSession`, `PairingFetch`, `make_private`, both
  cfg variants of `harden_private_state`, `OpenedRepository`); an `impl Store` block with
  `open_repository`, `conn` (236-330), `reap_expired` (3452-3478), and
  `force_expire_challenge` (cfg(test)), `ensure_server_keypair`, `server_keypair`,
  `server_public_key`, `ensure_server_secret` (3641-3712); `now_unix` (4255-4280),
  `is_unique_violation` (4388-4398); `mod`/`pub use` lines; `#[cfg(test)] mod tests;` (I).
- `schema.rs` (484): `impl Store { migrate }` (460-818), `backfill_legacy_transfers`,
  `LegacyLogEvent`, `json_value` (3998-4187), `add_column_if_missing` (4310-4331).
- `accounts.rs` (797): `impl Store` methods `register_owner` through `count_owners`
  (819-1659: owners, keys, challenges, pairing, rotation, revocation, sessions),
  `current_ident_key_on` (3905-3923), `insert_key_tx` (4188-4236).
- `versions.rs` (378 code + 56 cfg(test)): an `impl Store` block with
  `all_package_versions`, `replace_version_metadata` and the three `*_for_test` methods
  (331-459), `list_package_versions`, `latest_active_version`, `put_version_docs`,
  `latest_active_version_docs` (1660-1824), `package_version_exists`, `owner_version_count`,
  `publish_package_version` (2145-2330), `set_release_state` (3395-3451);
  `PackageVersionRow` (3842-3859), `insert_version_targets` (4281-4309).
- `browse.rs` (378): `impl Store { package_detail, package_audit, search_packages }`
  (1825-2144), `typosquat_candidates` (3479-3497), the row structs `SearchResultRow` through
  `ReleaseStateChangeRow` plus `escape_like` (3758-3841), `fuzzy_search_candidates`,
  `within_edit_distance_one` (4332-4387).
- `blobs.rs` (115): `BlobRow` + `from_row` (108-126), `impl Store` methods `blob_kind` through
  `forget_blob` (2331-2501).
- `governance.rs` (624): `impl Store` methods `owner_record` through `transfer_history`
  (2502-3132: orgs, publish tokens, transfers); `TransferTerms`, `TransferRecord`,
  `PendingOfferRow`, `STALE_TRANSFER_TERMS`, `NO_PENDING_OFFER` (3860-3904);
  `transfer_terms_on` (3924-3997).
- `transparency.rs` (365): an `impl Store` block with `init_registry_root`,
  `renew_registry_root`, `reanchor_registry_root`, `write_registry_root`, `registry_config`,
  `index_canonical_hash` (3133-3394) and `log_size`, `signed_checkpoint`, `log_leaf_hashes`,
  `publish_log_entry` (3498-3640); `LogEntryRef`, `RootCeremony`, `RegistryConfig`
  (3713-3757); `append_log_tx` (4237-4254).

`mod.rs` re-exports every part with `pub use <part>::*;`, keeping
`crate::store::{now_unix, NewSession, PairingFetch, Store, RegistryConfig, …}`. pub(super)
(8 free items): `add_column_if_missing`, `json_value`, `append_log_tx`,
`current_ident_key_on`, `insert_key_tx`, `owner_record`, `fuzzy_search_candidates`,
`within_edit_distance_one`; plus fields of `PendingOfferRow`, `RootCeremony` and
`LegacyLogEvent` used across files (visibility rule).

**The writer test (`key_insertion_has_exactly_one_writer`, in the `repository/src/store/tests/*.rs`
part plan-176-I put it in).** Replace the single-file `include_str!` with a
runtime read of every top-level `.rs` file in the store directory:

```rust
let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/store");
let mut writers = 0;
for entry in std::fs::read_dir(dir).expect("read repository/src/store") {
    let path = entry.expect("store dir entry").path();
    if path.extension().is_some_and(|e| e == "rs") {
        writers += std::fs::read_to_string(&path).expect("read store part").matches(needle).count();
    }
}
```

`read_dir` is not recursive, so `store/tests/` is not read (and the test's own needle is
built with `concat!`, so it never matches itself). The assertion and its message are
unchanged. AGENTS.md justification: (1) `git log -S key_insertion_has_exactly_one_writer` →
`e18cbb6fa` (2026-09-12, bug-580: auth/ident role separation). (2) Protects: every `keys` row
is created by `insert_key_tx`. (3) Dependents: none (`grep -rn key_insertion_has_exactly_one_writer`
→ the test and its plan-176-I entry). (4) Proof: after the split `store/mod.rs` holds 0
needles and `store/accounts.rs` holds 1 (`grep -c` per file), and the I-era
`include_str!("../../store.rs")` no longer names a file; the test's intent is "across the
store source", which reading every part preserves and which survives adding a part.

Citations (39 cites, 29 symbols, `package-manager/01`, `03`, `04`): mod — `Store`,
`open_repository`, `reap_expired`; accounts — `register_owner`, `owner_with_auth_key`,
`create_challenge`, `create_auth_challenge`, `create_ident_challenge`, `complete_challenge`,
`record_signing_request`, `store_pairing_blob`, `take_pairing_blob`, `rotate_ident`,
`reanchor_ident`, `revoke_auth_key`; schema — `migrate`, `backfill_legacy_transfers`;
versions — `list_package_versions`, `replace_version_metadata`, `publish_package_version`;
browse — `typosquat_candidates`; blobs — `unreachable_blobs`; governance —
`publish_token_for_key`, `create_transfer_offer`, `accept_transfer`; transparency —
`append_log_tx`, `init_registry_root`, `renew_registry_root`, `reanchor_registry_root`. Path
refs: none.

### 4.4 `src/cli/pkg.rs` → `src/cli/pkg/` (2019; the directory exists with `command_tests.rs` and K's `tests/`)

- `mod.rs` (84; 1-115): module doc, `use` header, `PkgCommandError`, `run_pkg_command`, the
  `mod`/`pub(crate) use <part>::*;` lines, `#[cfg(test)] mod tests;` (K), and the test module
  declaration `#[cfg(test)] mod command_tests;` — the old `#[path = "pkg/command_tests.rs"]`
  (36-39) is dropped because the file already sits at `pkg/command_tests.rs`, which is where a
  plain `mod command_tests;` in `pkg/mod.rs` resolves. Module name and its `super::*` view
  are unchanged.
- `publish.rs` (345; 116-536): `publish_package_project`, `upload_vendor_blobs`,
  `transfer_offer`, `transfer_accept`, `set_release_state`, `check_abi`,
  `print_publish_verify_report`.
- `remove.rs` (143; 537-737): `parse_remove_options`, `run_remove`,
  `remove_installed_package_files`, `removal_closure`.
- `update.rs` (212; 738-1026): `UpdateOptions`, `parse_update_options`, `run_update`,
  `update_declared_dependency`, `UpdateChoice`, `select_update_version`.
- `add.rs` (440; 1027-1616): `AddOptions`, `parse_add_options`, `infer_pin`, `add_package`,
  `refuse_vendored_local_package`, `install_local_package`, `without_indirect_entry`,
  `add_package_from_file`, `add_package_from_registry`, `verify_registry_release`,
  `metadata_key`, `print_transfer_notice`, `install_vendor_blobs`,
  `state_is_floating_eligible`, `select_index_version`.
- `verify.rs` (534; 1617-2062, 2347-2514): `validate_package_file`,
  `project_pinned_ident_key`, `verify_packages`, `report_dependency_closure`,
  `follow_identity_pin`, `follow_listed_release`, `rewrite_pinned_ident_key`,
  `PackageVerifyStatus` + `message`, `PackageVerifyResult`, `package_verify_line`,
  `verify_package_dependency`, `verify_source_package_manifest`, `package_dependency_status`,
  `package_version_status`, `package_version_matches`.
- `info.rs` (261; 2063-2346): `run_pkg_doc`, `write_package_doc`, `print_package_info`,
  `signature_type_name`, `hex_bytes`, `package_export_kind_name`, `empty_marker`.

`pub(crate) use <part>::*;` keeps `super::pkg::*` working from `repo.rs`, `dispatch`,
`resolve.rs` and `cli/mod.rs`. pub(super) (24): `AddOptions`, `UpdateChoice`,
`UpdateOptions` (plus their fields, read by the tests and `command_tests`), `add_package`,
`add_package_from_registry`, `empty_marker`, `infer_pin`, `PackageVerifyStatus::message`,
`package_export_kind_name`, `parse_add_options`, `parse_remove_options`,
`parse_update_options`, `print_package_info`, `print_publish_verify_report`,
`project_pinned_ident_key`, `removal_closure`, `run_pkg_doc`, `run_remove`, `run_update`,
`select_index_version`, `select_update_version`, `validate_package_file`, `verify_packages`,
`write_package_doc`.

Citations (38 cites, 26 symbols; `tooling/03,06,07`, `package-manager/01`, `spec`,
`architecture/03_packages.md`, `package/03_metadata-encoding.md`): mod — `run_pkg_command`;
publish — `publish_package_project`, `upload_vendor_blobs`, `check_abi`; add —
`add_package`, `add_package_from_registry`, `infer_pin`, `install_vendor_blobs`,
`select_index_version`, `state_is_floating_eligible`, `verify_registry_release`,
`without_indirect_entry`; verify — `validate_package_file`, `verify_packages`,
`report_dependency_closure`, `follow_identity_pin`, `verify_package_dependency`,
`package_dependency_status`, `package_verify_line`, `package_version_matches`; info —
`run_pkg_doc`, `write_package_doc`, `print_package_info`, `signature_type_name`, `hex_bytes`,
`package_export_kind_name`.

Coverage: `scripts/coverage-exceptions.txt:17` (`src/cli/pkg.rs …`) is replaced by six lines,
one per part that holds live-registry calls, each with the existing reason text:
`src/cli/pkg/mod.rs`, `src/cli/pkg/publish.rs`, `src/cli/pkg/update.rs`,
`src/cli/pkg/add.rs`, `src/cli/pkg/verify.rs`, `src/cli/pkg/info.rs`. `src/cli/pkg/remove.rs`
has no live-registry call and gets no line.

Prose refs: `src/cli/pkg/command_tests.rs:3` "`cli/pkg.rs` is 2,274 lines…" → "`cli/pkg.rs`
(now `cli/pkg/`) was 2,274 lines…"; the `tests/cli` comment citing `src/cli/pkg.rs:566` →
"(`add_package_from_file`, `src/cli/pkg/add.rs`)"; `src/cli/resolve.rs:556`'s
`src/cli/pkg.rs:1140` → "`add_package_from_file` in `src/cli/pkg/add.rs`" (this comment moves
with `is_registry_dependency` into `resolve/closure.rs`).

### 4.5 `src/cli/resolve.rs` → `src/cli/resolve/` (1016)

- `mod.rs` (498; 1-28, 55-122, 592-1152): `use` header, `Requirer`, `Node`, `PinChange`,
  `Resolution`, `update`, `install`, `resolve`, `Selection`, `select_node`, `is_superset`,
  `load_import_edges`, `compare_version_components`, `compare_versions`, `read_manifest`;
  `pub(crate) use {lockfile, closure, drift}::*;`; `#[cfg(test)] mod tests;` (K).
- `lockfile.rs` (207; 29-54 `LOCKFILE_VERSION`, `LockedPackage`, `Lock`; 1153-1350
  `lock_path`, `write_lock`, `render_lock`, `read_lock`, `print_lock_diff`).
- `closure.rs` (195; 123-393 `MAX_CLOSURE_ROUNDS`, `ClosureChanges`, `apply_manifest_change`,
  `close_dependency_set`, `registry_dependency_for`, `apply_closed_manifest`; 572-591
  `is_registry_dependency`, `registry_dependency_count`).
- `drift.rs` (116; 394-571): `DriftClass`, `classify_drift`, `report_drift`.

pub(super) (6): `lock_path`, `render_lock`, `print_lock_diff`, `is_registry_dependency`,
`registry_dependency_count`, `report_drift`; plus `Lock` / `LockedPackage` fields that are not
already `pub(crate)` (visibility rule).

Citations (20 cites, 12 symbols; `tooling/03_lockfile.md`, `tooling/07_cli-reference.md`,
`package-manager/01`): mod — `update`, `install`, `resolve`, `select_node`; lockfile —
`LockedPackage`, `read_lock`, `write_lock`, `render_lock`; closure — `apply_manifest_change`,
`close_dependency_set`, `apply_closed_manifest`; drift — `classify_drift`.

Coverage: `scripts/coverage-exceptions.txt:18` becomes two lines with the same reason:
`src/cli/resolve/mod.rs` and `src/cli/resolve/closure.rs`. `lockfile.rs` and `drift.rs` are
pure and get no line. Prose ref: the `tests/cli` comment citing
`src/cli/resolve.rs:253` → "`resolve()`'s seeding filter at the time; today
`is_registry_dependency` in `src/cli/resolve/closure.rs`".

### 4.6 `src/debugger/breakpoints.rs` → `src/debugger/breakpoints/` (1271; the directory holds `report.rs` and K's `tests/`)

- `mod.rs` (589): 1-243 (header, `mod report;` unchanged, `REPORT_HINT`, `Breakpoint`,
  `Watch`, `WATCH_SPAN`, `watchable`, `watched_value`, `value_shown`, `commands_shown`,
  `listed`, `runs`, `Ended`, `Hear`, `Target`, `offsets`, `Breakpoints`); 324-491 (`written`,
  `tag`, `shown`, `find`, `describe`, `place`, `tested`, `if_shown`, `described`,
  `same_place`, `written_shown`); an `impl Breakpoints` block (492-836: `set`, `armed`,
  `user_at`, `is_park`, `remove`, `lines`, `breakpoint_lines`, `watch`, `unwatch`,
  `watch_lines`); `pub(crate) use {running, errors}::*;`; `#[cfg(test)] mod tests;` (K).
- `running.rs` (490): an `impl Breakpoints` block with 837-1172 (`count`, `stop_commands`,
  `take_notes`, `run`, `run_once`, `settle`, `sort_hits`, `watched_writes`,
  `watched_values`) and 1327-1542 (`restarted`, `finish`, `run_to_return`,
  `place_pending`); `enum Finish` (1543-1551).
- `errors.rs` (192): 244-323 (`PARK_ERROR`, `ErrorCodes`, `ErrorWatch`, `Raise`, `unseen`,
  `park_address`, `code_shown`, `impl ErrorCodes`) and an `impl Breakpoints` block 1173-1326
  (`watched_raise`, `run_to_raiser`, `watch_errors`, `clear_errors`, `watch_line`).

pub(super) (9): `ErrorWatch`, `ErrorCodes::{shown, matches}`, `park_address`, `unseen`,
`watched_raise`, `run_to_raiser`, `sort_hits`, `watched_writes`; plus `ErrorWatch` / `Raise`
fields read in `mod.rs` or `running.rs`.

Citations (11, `tooling/10_debugger.md`): mod — `Breakpoint`, `Breakpoints`, `Ended`,
`Watch`, `find`; running — `run` (`Breakpoints::run`), `Finish`; errors — `ErrorCodes`.
Prose: `src/docs/spec/tooling/10_debugger.md:69` "(`src/debugger/breakpoints.rs`)" →
"(`src/debugger/breakpoints/`)".

### 4.7 `src/debugger/linux.rs` → `src/debugger/linux/` (1708; aarch64 Linux only)

- `mod.rs` (210): 1-243 (`use` header, all `PTRACE_*`/`NT_*`/signal/`WCR_*`/`HW_DEBUG_*`/timing
  constants, `BREAKPOINT_INSTRUCTION`, `GLIBC_PTHREAD_SIZE`, `IoVec`, `SigAction`, the
  `extern "C"` block, `INTERRUPTED`, `on_interrupt`, `SESSION_THREAD`, `errno`,
  `LinuxAarch64` + `impl Backend`, `trace`, `os_error`); 489-562 (`Thread`, `Report`,
  `Tracee`, `Mapping`, `Library`); 1544-1552 (`impl Drop for Tracee`);
  `mod process; mod images; mod threads; mod inferior; pub(super) use process::*;`.
- `process.rs` (211; 244-488): `seize_refusal`, `task_ids`, `seize_all`, `spawn`,
  `seize_launched`, `stopped`, `exited`, `signaled`, `stop_signal`, `event`, `kill_and_reap`.
- `images.rs` (313): an `impl Tracee` block (563-902: `new`, `read`, `write`, `image`,
  `mappings`, `bias`, `refresh_libraries`, `library_file`, `loaded_functions`,
  `library_function_at`).
- `threads.rs` (567): an `impl Tracee` block (903-1543: `regset`, `set_regset`, `general`,
  `float`, `siginfo`, `watch_slots`, `set_thread_watchpoints`, `arm_watchpoints`, `lift`,
  `reinsert`, `index_of`, `thread_at`, `resume`, `wait_any`, `thread_ended`, `adopt`,
  `welcome`, `signal_stop`, `stop_all`, `classify`, `check_exit`, `blocked`, `step_rounds`,
  `pthread_gap`, `thread_name`, `facts`, `detach`).
- `inferior.rs` (407; 1553-1988): `impl Inferior for Tracee`.

`LinuxAarch64` stays `pub(super)` in `mod.rs`; `debugger/mod.rs` uses `linux::LinuxAarch64`
unchanged. `Tracee`/`Thread`/`Mapping`/`Library` fields stay private in `mod.rs` (visible to the
children). pub(super) (39): process — `event`, `exited`, `kill_and_reap`, `seize_all`,
`spawn`, `stop_signal`, `stopped`; images — `new`, `read`, `write`, `image`, `mappings`,
`bias`, `refresh_libraries`, `loaded_functions`, `library_function_at`; threads — `adopt`,
`arm_watchpoints`, `check_exit`, `classify`, `detach`, `facts`, `float`, `general`,
`index_of`, `lift`, `pthread_gap`, `reinsert`, `resume`, `set_regset`,
`set_thread_watchpoints`, `signal_stop`, `step_rounds`, `stop_all`, `thread_at`,
`thread_ended`, `wait_any`, `watch_slots`, `welcome`.

Citations (13, `tooling/10_debugger.md`): mod — `LinuxAarch64`, `Tracee`,
`GLIBC_PTHREAD_SIZE`, `BREAKPOINT_INSTRUCTION`, `on_interrupt`; process — `spawn`,
`seize_launched`, `seize_all`, `seize_refusal`, `kill_and_reap`; threads — `signal_stop`,
`step_rounds`, `set_thread_watchpoints`.

### 4.8 `src/debugger/macos.rs` → `src/debugger/macos/` (1980)

- `mod.rs` (437): 1-526 (module doc, `use` header, constants, the FFI block, `MacOs` +
  `impl Backend`, `spawn`, `Task`, `Library`, `INTERRUPTED`); an `impl Task` block with only
  `new` (527-551); 2296-2372 (`impl Drop for Task`, `task_port`, `kill_and_reap`, `alive`,
  `release`, `mach_error`); `mod libraries; mod threads; mod exceptions; mod inferior;
  use {libraries::*, threads::*, exceptions::*};` (a private glob: nothing leaves the module
  except `MacOs`, which stays in `mod.rs`).
- `libraries.rs` (451): an `impl Task` block with 552-953 (`refresh_libraries`,
  `executable_dependencies`, `read_c_string`, `read_library`, `library_starts`,
  `library_symbols`, `library_stub`, `library_function`, `loaded_functions`) and 1085-1163
  (`image`, `slide`, `executable_header`); `uleb128` (1463-1475).
- `threads.rs` (536): an `impl Task` block with 954-1084 (`patch`, `arm_watchpoints`,
  `pause_watchpoints`, `set_thread_watchpoints`, `lift`, `reinsert`, `check_exit`) and
  1164-1462 (`read`, `thread_ports`, `thread_at`, `region_containing`, `facts`,
  `thread_name`, `step_rounds`, `in_system_call`, `is_step`); free fns 1476-1614 (`place`,
  `ThreadPorts` + `Drop`, `general_state`, `neon_state`, `set_thread_state`, `debug_state`,
  `set_debug_state`, `set_single_step`, `write_watchpoints`, `watchpoint_slots`);
  `thread_state` (1779-1796); the second `impl Task { classify }` (2250-2295).
- `exceptions.rs` (142; 1615-1778): `ExceptionPort` + impl + `Drop`, `Exception` + impl
  (`answer`, `describe`).
- `inferior.rs` (414; 1797-2249): `impl Inferior for Task`.

pub(super) (30): exceptions — `Exception`, `ExceptionPort` (plus fields used by threads /
inferior), `answer`, `describe`, `install`, `receive`; libraries — `image`, `slide`,
`library_function`, `library_starts`, `library_stub`, `library_symbols`, `loaded_functions`,
`refresh_libraries`; threads — `arm_watchpoints`, `check_exit`, `classify`, `facts`,
`general_state`, `lift`, `neon_state`, `patch`, `pause_watchpoints`, `read`, `reinsert`,
`set_thread_state`, `step_rounds`, `thread_at`, `thread_ports`, `watchpoint_slots`.

Citations (12, `tooling/10_debugger.md`): mod — `MacOs`, `Task`, `Library`, `task_port`;
libraries — `loaded_functions`; threads — `set_thread_state`, `write_watchpoints`;
exceptions — `ExceptionPort`. `task_port` calls `std::env::current_exe()` at runtime (no path
dependence).

### 4.9 `src/debugger/prompt.rs` → `src/debugger/prompt/` (2041)

- `mod.rs` (308; 1-152 `use` header, `PROMPT`, `HELP`, `LIST_USAGE`, `LIST_REG_USAGE`;
  2146-2331 `run`); `pub(crate) use {command, render, stops, actions}::*;`;
  `#[cfg(test)] mod tests;` (K).
- `command.rs` (687; 153-955, the command-line grammar): `MAIN_THREAD`, `Command`,
  `Backtrace`, `Dump`, `Disassembly` + `from`, `is_count`, `instruction_count`,
  `instruction_window`, `instruction_span`, `address`, `Listing`, `FunctionFilter` + impl,
  `glob`, `RegisterPick` + impl, `parse`, `conditional`, `settable`, `assigned`,
  `COMMAND_NAMES`, `is_command`, `stop_commands`, `runs_on`, `step_count`, `arena_id`,
  `number`, `frame_count`, `error_code`, `thread_id`, and the constants in the range
  `MAX_MEMORY`, `MEMORY_SIZE`, `NZCV_FLAGS`, `STRING_LITERAL_PREFIX`, `Faults`, `Queue`.
- `render.rs` (255; 956-1135 `COLUMN_SEPARATOR`, `table`, `function_lines`,
  `register_lines`, `common_register_lines`, `register_value`, `thread_lines`,
  `arena_lines`; 1416-1536 `FIELD_WIDTH`, `state_lines`, `block_lines`, `dump_lines`,
  `memory_lines`, `hex_dump`).
- `stops.rs` (253; 1537-1834): `stop_thread`, `stop_place`, `error_shown`, `stop_lines`,
  `emit`, `hear_stop`, `inspect`, `finish_lines`, `restart`.
- `actions.rs` (538; 1136-1415 `list`, `no_thread`, `instructions`, `step`, `ended_lines`,
  `step_all`, `find_arena`, `show_arena`; 1835-2145 `show_memory`, `show_type`,
  `show_error`, `function_start`, `resolve`, `jump`, `backtrace`, `disassemble_at`,
  `places`).

The globs keep `prompt::{address, number, table, FunctionFilter, …}` working for the other
debugger modules. pub(super) (42): command — `Backtrace`, `Command`, `Disassembly`, `Dump`,
`Faults`, `Listing`, `Queue`, `RegisterPick` (enums; variants follow), `MAIN_THREAD`,
`MAX_MEMORY`, `MEMORY_SIZE`, `NZCV_FLAGS`, `STRING_LITERAL_PREFIX`, `parse` and the `parse`
methods of `FunctionFilter`/`RegisterPick`, `Disassembly::from`,
`FunctionFilter::{admits, words}`, `glob`, `runs_on`, `stop_commands`; render —
`register_value`; stops — `emit`, `finish_lines`, `hear_stop`, `inspect`, `restart`,
`stop_lines`; actions — `backtrace`, `disassemble_at`, `ended_lines`, `jump`, `list`,
`places`, `resolve`, `show_arena`, `show_error`, `show_memory`, `show_type`, `step`,
`step_all`; plus the fields of `Dump` and `FunctionFilter` read outside `command.rs`. The fn
`backtrace` (actions) and the module `super::backtrace` are in different namespaces, as today.

Citations (28, `tooling/10_debugger.md`): mod — `run`; command — `Faults`, `FunctionFilter`,
`RegisterPick`, `glob`, `parse`, `stop_commands`; render — `table`, `function_lines`,
`register_lines`, `common_register_lines`, `thread_lines`, `arena_lines`, `state_lines`,
`dump_lines`, `memory_lines`; stops — `hear_stop`, `stop_lines`, `stop_place`; actions —
`step`, `step_all`, `ended_lines`, `show_arena`, `show_memory`, `show_error`,
`function_start`, `resolve`, `jump`.

### 4.10 `src/debugger/values.rs` → `src/debugger/values/` (1068)

- `mod.rs` (641; 1-60 `Read`, `use` header, `Region`; 304-587 `show_literal`, `show`,
  `Memory` + impl; 832-1207 `payload_text`, `scalar_text`, `money_text`, `key_bytes`,
  `parse_integer`, `resource_summary`, `resource_kind`, `resource_tail`, `utf8`, `quoted`,
  `code_scalar`, `a`, `code_name`, `code_label`, `kind_name`, `payload_width`);
  `pub(crate) use {view, collection}::*;`; `#[cfg(test)] mod tests;` (K).
- `view.rs` (210; 61-303): `Scalar`, `Kind`, `Span`, `MapView`, `View`, `TYPE_USAGE`,
  `parse_view`, `type_names`, `element_kind`, `span`, `unquote`.
- `collection.rs` (217; 588-831): `Shape`, `Collection`, `Entry`, `Part` + impls, `elements`,
  `entry_lines`.

pub(super) (15): view — `Scalar::{width, name}`, `Kind::name`, `span`; collection —
`Collection`, `Entry`, `Part`, `Shape` (plus fields), `Shape::name`, `elements`, `entry`,
`entry_lines`, `key_at`, `live_entries`, `summary`.

Citations (4, `tooling/10_debugger.md`): view — `parse_view`; mod — `show`, `show_literal`,
`quoted`.

### 4.11 `src/manifest/mod.rs` + 2 parts (1092; already `mod.rs`)

Existing siblings: `closure`, `config`, `entry`, `json_edit`, `libraries`, `package`, `url`
(and K's `tests/`). No new part is named `libraries.rs`.
- `mod.rs` (358): 1-170 (module decls, `parse_project_json`, `max_buffer_bytes`,
  `validate_max_buffer`, `canvas_auto_paint_ms`, `validate_canvas_auto_paint_ms`), 236-433
  (`validate_project_manifest`, `validate_config`, `render_config_diagnostic`,
  `validate_packages`), 1337-1460 (`build_mode_is_app`, `icon_path`, `ResourceEntry`,
  `resource_entries`, `project_version`, `stdin_log_cap`, `project_kind`, `entry_point`,
  `validate_packages_array`, `field_position`, `fallback_field_position`); the inline
  `mod canvas_auto_paint_tests` (171-235) and `#[cfg(test)] mod tests;` stay;
  `mod fields; mod locator_check; pub(crate) use {fields::*, locator_check::*};`.
- `fields.rs` (451; 434-983, the per-field validators): `validate_name`,
  `validate_required_string`, `validate_optional_string`, `MAX_DESCRIPTION_BYTES`,
  `validate_description`, `validate_sources`, `validate_source_pattern_field`,
  `validate_kind`, `validate_mode`, `path_stays_in_project`, `validate_resources`.
- `locator_check.rs` (283; 984-1336, the `libraries` section validation): `LibraryFinding` +
  impl, `validate_libraries`, `check_libraries`, `check_locator`, `token_list`.

The globs keep `crate::manifest::{check_libraries, LibraryFinding, MAX_DESCRIPTION_BYTES,
validate_description, …}`. pub(super) (11): fields — `validate_name`,
`validate_required_string`, `validate_optional_string`, `validate_description`,
`validate_sources`, `validate_kind`, `validate_mode`, `validate_resources`,
`path_stays_in_project`; locator_check — `validate_libraries`, `LibraryFinding::wrong_type`.

Citations (38 cites, 20 symbols; `tooling/01_project-manifest.md`,
`tooling/02_source-selection.md`, `architecture/02_frontend.md`,
`package/15_package-meta-section.md`): stay at `src/manifest/mod.rs` (path unchanged) —
`validate_project_manifest`, `validate_config`, `validate_canvas_auto_paint_ms`,
`DEFAULT_CANVAS_AUTO_PAINT_MS`, `canvas_auto_paint_ms`, `max_buffer_bytes`,
`build_mode_is_app`, `icon_path`, `stdin_log_cap`; → `src/manifest/fields.rs` —
`MAX_DESCRIPTION_BYTES`, `validate_description`, `validate_name`,
`validate_required_string`, `validate_optional_string`, `validate_sources`,
`validate_source_pattern_field`, `validate_kind`, `validate_mode`, `validate_resources`; →
`src/manifest/locator_check.rs` — `validate_libraries`. `spec_citations_resolve` cannot catch
these (the old file still exists); the citation sweep check in §4 does.
`.ai/build-tooling.md:84` "`src/manifest/mod.rs` ~1049" → "`check_libraries` in
`src/manifest/locator_check.rs`". `no_type_strings` names `src/manifest/package.rs` only
(untouched).

## Phases

> **NOTE — keep the checkboxes current as you go. This is not bookkeeping; it is
> the only way anyone can see where the work actually is.**
>
> - Tick `- [x]` **in the same commit as the work it describes** — never in a
>   batch at the end of a phase, never "I'll update the plan after". If it is a
>   separate step it will be skipped, and the list becomes a lie.
> - Use `- [~]` for partially done and say in one line what remains.
> - Mark a task moot with `- [x] ~~text~~ — moot: <evidence>` rather than
>   deleting it. A vanished task is indistinguishable from a forgotten one.
> - Fill the phase's `Commit:` line with the hash(es) the moment it lands.
> - Add any task you discover you needed; an unlisted task that got done is a
>   task the next reader will do twice.
>
> **An unticked box means NOT DONE.** Someone reading this file — including the
> person who has to decide whether to stop — is entitled to trust that.

Common gate, used by every phase:

- **G0 (before the first edit of the letter):**
  `cargo check --all-targets 2>&1 | grep -c '^warning' > /tmp/l_check.txt`,
  `cargo clippy --all-targets 2>&1 | grep -c '^warning' > /tmp/l_clippy.txt`,
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$' > /tmp/l_mfb.txt`,
  `cargo test -p mfb_repository -- --list 2>/dev/null | grep -c ': test$' > /tmp/l_repo.txt`
  (est. 8 min cold).
- **G1:** the four commands again → equal to the G0 files.
- **G2:** `cargo test -p mfb --bin mfb spec_citations_resolve` → passes, and the §4 citation
  sweep check for the phase's new directories → no output (est. 1 min).
- **G3:** `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings`
  → pass (est. 2 min).
- **G4:** `cargo test --test file_size_budget 2>&1 | grep -E '^  (NEW|GREW)'` → no output; every
  `TIGHTEN`/`STALE` row names this phase's files (old `foo.rs` paths are STALE, `mod.rs` hosts
  TIGHTEN); `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`, rerun →
  `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`; commit the ceiling file with the phase.
- **G5:** `cargo fmt --check` → no output.

### Phase 1 — Manifest validator

- [ ] Split `src/manifest/mod.rs` into `mod.rs`, `fields.rs`, `locator_check.rs` per §4.11.
- [ ] Repoint the 11 citations of moved symbols (§4.11).
- [ ] `.ai/build-tooling.md:84` → `check_libraries` in `src/manifest/locator_check.rs`.
- [ ] G4.

Acceptance: manifest validation behaves identically; the citations name the defining files.
  Check: G1; `cargo test -p mfb --bin mfb manifest::` → 0 failed;
  `cargo test --test cli_build_config` → pass; G2 (the sweep check over `src/manifest/`); G3;
  G5 (est. 8 min).
Commit: —

### Phase 2 — Package CLI

- [ ] Split `src/cli/pkg.rs` per §4.4 (drop the `#[path]` on `command_tests`).
- [ ] Split `src/cli/resolve.rs` per §4.5.
- [ ] Sweep the 58 citations (38 pkg + 20 resolve).
- [ ] `scripts/coverage-exceptions.txt`: line 17 → the six `src/cli/pkg/…` lines; line 18 →
      the two `src/cli/resolve/…` lines; each with the existing reason text (§4.4, §4.5).
- [ ] Stale line refs: the `tests/cli` comment (`grep -rn 'src/cli/pkg.rs:566\|src/cli/resolve.rs:253' tests/cli`),
      `src/cli/resolve/closure.rs` (old `resolve.rs:556`) and `src/cli/pkg/command_tests.rs:3`
      per §4.4/§4.5 (`grep -rn 'cli/pkg.rs:\|cli/resolve.rs:' src tests` → 0 after).
- [ ] G4.

Acceptance: the package commands behave identically and keep their coverage treatment.
  Check: G1; `cargo test -p mfb --bin mfb cli::pkg cli::resolve` → 0 failed;
  `cargo test --test cli_pkg_arity --test cli_repo_install --test cli_repo_publish --test cli_repo_auth --test cli_repo_governance --test cli_untrusted_registry_text_is_escaped`
  → pass (these drive `mfb pkg`/`mfb repo` against a local registry); `grep -c '^src/cli/\(pkg\|resolve\)' scripts/coverage-exceptions.txt`
  → 8 and `grep -c '^src/cli/\(pkg\|resolve\)\.rs' scripts/coverage-exceptions.txt` → 0; G2;
  G3; G5 (est. 15 min).
Commit: —

### Phase 3 — Debugger shared code (host-built)

- [ ] Split `src/debugger/values.rs` per §4.10.
- [ ] Split `src/debugger/breakpoints.rs` per §4.6.
- [ ] Split `src/debugger/prompt.rs` per §4.9.
- [ ] Sweep the 43 citations (4 + 11 + 28) and `10_debugger.md:69`'s prose path.
- [ ] G4.

Acceptance: the debugger prompt, breakpoints and value views behave identically.
  Check: G1; `cargo test -p mfb --bin mfb debugger::` → 0 failed; `cargo test --test cli_dbg`
  → pass; G2; G3; G5 (est. 8 min).
Commit: —

### Phase 4 — Debugger backends (macOS on the host, Linux on box 2223)

- [ ] Split `src/debugger/macos.rs` per §4.8.
- [ ] Split `src/debugger/linux.rs` per §4.7.
- [ ] Sweep the 25 citations (12 + 13).
- [ ] G4.

Acceptance: both backends compile and the macOS backend runs.
  Check: G1; `cargo test --test cli_dbg` → pass (drives the macOS backend on the host); G2;
  G3; G5. Then, after committing the phase, box 2223 (the only aarch64-Linux box with cargo;
  `linux/` is compiled nowhere else):
  `git archive HEAD | ssh -p 2223 test@127.0.0.1 'rm -rf ~/mfb-176l && mkdir ~/mfb-176l && tar -x -C ~/mfb-176l && cd ~/mfb-176l && cargo check --all-targets 2>&1 | grep -E "^(error|warning)|src/debugger/linux|Finished"'`
  → a `Finished` line, no `error` line, and no line naming `src/debugger/linux` (est. 15 min). A failure there is
  fixed in a follow-up commit of this phase.
Commit: —

### Phase 5 — Repository client and server

- [ ] Split `repository/src/client.rs` per §4.1.
- [ ] Split `repository/src/server.rs` per §4.2.
- [ ] Sweep the 125 citations (51 + 74).
- [ ] `.ai/net-tls.md:266` → `repository/src/client/transport.rs`; `.ai/testing-gates.md:703`
      → `repository/src/local.rs` (§4.1).
- [ ] G4.

Acceptance: the registry client and server behave identically.
  Check: G1; `cargo test -p mfb_repository` → 0 failed (est. 4 min); `cargo build` of `mfb`
  (it imports `mfb_repository::server::…`) → succeeds; `cargo test --test cli_repo_publish --test cli_repo_auth`
  → pass (live local server round trip); G2; G3; G5 (est. 12 min).
Commit: —

### Phase 6 — Repository store (largest blast radius)

- [ ] Split `repository/src/store.rs` per §4.3 (eight `impl Store` blocks).
- [ ] Rewrite `key_insertion_has_exactly_one_writer`'s source read in
      its `repository/src/store/tests/*.rs` part per §4.3 (justification in §4.3).
- [ ] Sweep the 39 citations.
- [ ] G4.

Acceptance: the store behaves identically and the single-writer invariant is still enforced.
  Check: G1; `cargo test -p mfb_repository` → 0 failed; the writer test still bites: add a
  second `"INSERT INTO keys (owner_id, role, public_key"` string to `store/governance.rs`, run
  `cargo test -p mfb_repository key_insertion_has_exactly_one_writer` → FAILED with "found 2
  insertion sites", remove it, rerun → passes; `cargo test --test cli_repo_install --test cli_repo_governance`
  → pass; G2; G3; G5 (est. 12 min).
Commit: —

## Validation Plan

- Tests: no test added or removed (G1 counts). Each phase runs the module's unit tests and the
  `tests/cli` targets that drive the moved commands end to end against a local registry
  server.
- Coverage check: per-file coverage is not a plan-176 gate — author, 2026-10-17. Evidence: at 7a4b09970,
  `sh scripts/coverage.sh --bins` in a detached worktree (59 min; 4989 unit tests passed), then
  `sh scripts/coverage-check.sh` exited 1. 241 `src/` files are already below the 98% per-file
  floor ("Files below 98% line coverage (GATE FAILURE)"), and overall coverage is 93.67%.
  Examples among plan-176 hosts: `src/ir/shape.rs` 93.21%,
  `src/codegen/engine/control/builder_control.rs` 75.55%, `src/debugger/values.rs` 86.66%. A
  pure move changes no executed line, and moving an inline test block only removes test lines
  from its host's denominator. The pre-existing gate failure is reported to the owner
  separately and is not a plan-176 task.
  The exception-list repoint (Phase 2's `scripts/coverage-exceptions.txt` task) is checked by
  its own `grep -c` acceptance, not by a coverage run.
- Runtime proof: Phase 4 compiles the Linux backend on box 2223 and runs the macOS one through
  `cli_dbg`; Phase 6's negative check proves the rewritten writer test still binds.
- Doc sync: spec citations in the topics listed under References; `.ai/net-tls.md`,
  `.ai/testing-gates.md`, `.ai/build-tooling.md`; `scripts/coverage-exceptions.txt`. No
  observable contract changes, so no spec prose changes beyond paths.
- Final gate (once, after Phase 6): G1, G2, G3 and `cargo test --test file_size_budget` → banner
  has 0 rows (est. 10 min). The full workspace suite runs once, in plan-176-V.

## Decisions

- Rule revised — owner, 2026-10-18 (replaces the 2026-10-14 "code ≤ 1000, test ≤ 250" rule):
  three kinds, non-comment non-blank LOC, scope `.rs`/`.mfb` under `src/` `packages/`
  `examples/` `repository/` `tests/`: `code` (code LOC of a non-test-only file) ≤ 1000 → split
  into modules; `inline` (`#[cfg(test)]` / `TESTING … END TESTING` LOC inside a non-test-only
  file) ≤ 250 → move the tests to a test-only file; `test` (whole test-only file) ≤ 1000 → split
  into modules. Parts target ≤ 900. No exemptions except `GENERATED` files; ceiling rows are
  `ceiling <code|inline|test> <n> <path>`; the guard warns with a banner, never fails; a new
  module is always an acceptable split. I and K moved these files' inline tests out;
  `store/versions.rs` keeps 56 cfg(test) LOC of in-impl helpers, under the 250 inline limit.
- A single function over the code limit is decomposed by extract-method — owner, 2026-10-14.
  None of L's functions is over 1000 (largest part 797), so L does no extraction.
- Byte-identity is the gate for codegen code letters; L's files emit no code, so L's gate is
  the crates' tests (brief, 2026-10-14).
- plan-176-I repoints the writer test's `include_str!`; L replaces it with the directory read —
  author, 2026-10-14.
- Coverage exceptions follow the live-registry boundary per part; pure parts get none — author
  (keeps the gate as strict as the exception file's own rule: "each entry must name the
  specific … boundary").
- Per-file coverage is not a plan-176 gate; the measured evidence is in §Validation Plan
  "Coverage check" — author, 2026-10-17.
- Pre-existing defects found by the map are fixed here: `.ai/testing-gates.md:703`, the three
  stale line refs, `.ai/build-tooling.md:84` — author, 2026-10-14.
- Glob-cycle imports compile without warnings — verified (`/tmp/loc/probe2`).

## Corrections

- 2026-10-18: Rule revised by the owner (code ≤ 1000, inline ≤ 250, test-only file ≤ 1000, no exemptions); file list and split design unchanged; rule text (intro, Goal, Decisions) restated, and the store writer test is now located as "the `store/tests/*.rs` part I chose" instead of I's old `key_roles.rs` name.

## Summary

L is code motion in four subsystems that emit no code, gated by their own tests. The risk is in
the store (one huge `impl` cut into eight blocks, and the one test whose logic reads source
files), the Linux debugger backend (provable only on box 2223), and the coverage-exception
lines for `pkg`/`resolve`. Every external path keeps working through `mod.rs` glob
re-exports; citations follow each symbol to its new file.
