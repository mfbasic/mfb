# audit-3 — Surface 9: supply chain — install / resolve / registry client

Part of `planning/goal-08-platform-security-review.md`. Finding prefix `SUP-`.
Repo @ `5e32d05e9`. Untrusted party: a malicious or MITM'd registry, or a
spoofed dependency source. They must not get an unverified, substituted, or
downgraded package accepted at install/build time, or run code at install time.

**Verdict: 4 MEDIUM · 2 LOW · 2 NTH. No CRITICAL, no HIGH.** One MEDIUM
(SUP-02, a terminal-injection that forges the `[Verified]` line the operator
reads) is **demonstrated live** and re-verified by the lead. The transport and
install-verification core is sound — https enforced, no cert/hostname bypass, no
install-time code execution, no on-disk blob cache, cache re-verified on use,
dependency confusion closed at the import boundary. The weaknesses are in
*integrity of the version selection* (the `/index` list is unsigned and the
opt-in TUF hash that would cover it is dead code), *credential confinement*
across redirects, and *install-side binding* of the fetched blob to the lock.

## SUP-01 — `/index` version list is unauthenticated; `snapshot.indexHash` is decoded then discarded (downgrade / freeze)

- **Severity:** MEDIUM — re-open of audit-2 SUP-03 / `bugs/skipped/bug-189`.
- **Location:** `repository/src/client.rs:1236-1242` (`fetch_index` — signs only
  the name binding), `repository/src/server.rs:521` (`versions[]` unsigned),
  `repository/src/client.rs:810,898` (`DelegatedMetadata.index_hash`),
  `:976` (`verify_pinned_metadata` — consumes only `server_key` +
  `snapshot_version`), `src/cli/resolve.rs:96` (`update` prints, never enforces,
  a backward move).
- **Threat / impact:** integrity/availability of version selection. A registry
  (or a compromised honest one) can omit newer entries from `versions[]` or flip
  a `state` to `yanked`, forcing a downgrade to an older validly-signed (and
  possibly vulnerable) release. Every artifact still passes §3.5, so the
  downgrade is silent.
- **Mechanism:** the only signature over `/index` covers
  `(owner, identFingerprint)`; `versions[]` (`version`, `hash`, `state`, `abi`)
  is unsigned. The TUF layer that exists to close this computes
  `snapshot.indexHash` and cross-checks it against `timestamp.indexHash`
  (`client.rs:891`) — then **discards** it. **Lead-verified dead code:**
  `grep -rn 'index_hash\|indexHash' repository/src src/` shows the field written
  at `client.rs:810,898`, cross-checked at `:867,887,891`, and the server
  computing `index_canonical_hash()` at `server.rs:1950,1982` — but the client
  never recomputes the digest over the idents it fetched and compares. So even
  the opt-in `mfb repo trust` path does not detect a stale/partial index,
  contradicting `src/docs/spec/package-manager/01_repository-protocol.md:1024`
  ("a mirror or MITM cannot serve a stale or partial index undetected").
- **Reproduction:** not demonstrated end-to-end (needs a registry that can sign
  a valid `/ident` + name binding). Mechanism is a direct read.
- **MFB trigger program (spike):** none possible — CLI/registry-protocol
  boundary, not reachable from a `.mfb` program.
- **Best fix:** make the index binding load-bearing — serve a per-ident signed
  version list (server signature over a canonical `(ident,[(version,hash,state)])`
  encoding, verified in `fetch_index`), and make `snapshot.indexHash`
  enforceable so `verify_pinned_metadata`'s `index_hash` stops being dead. Cheap
  independent floor (no protocol change): thread the previous `Lock` into
  `resolve()` and **refuse** (not merely print) a selection lower than the locked
  version absent an explicit `@version` / `--allow-downgrade`.
- **Non-goals:** no registry HTTP contract break (add a route/field); no
  weakening of per-blob SHA-256 or the §3.5 chain; don't make `mfb repo trust`
  mandatory in a way that breaks a local dev registry.
- **Prior art:** **bug-189** (`bugs/skipped/`, "Partially Fixed — SUP-03
  downgrade defense remaining"). audit-2 SUP-01 (plaintext) and SUP-02 (blind
  TOFU) *are* fixed (`ensure_transport_security:86`; `pin_server_key` +
  `MFB_REPO_SERVER_FINGERPRINT`, `local.rs:323-359`). **New:** the `index_hash`
  dead-code observation shows the documented opt-in remedy does not close it.
  → filed as an augmentation to bug-189.

## SUP-02 — Registry-controlled strings reach the operator's terminal unsanitized, forging verification output

- **Severity:** MEDIUM — **demonstrated (lead-reproduced).**
- **Location:** `repository/src/client.rs:1471` (`read_json_response` returns the
  server's `error` verbatim); print sites `src/cli/mod.rs:32,36`
  (`eprintln!("error: {message}")`), `src/rules/mod.rs:104`
  (`eprintln!("               {}", detailed_message)`),
  `src/cli/pkg.rs:1874` (`Release State: {}`), `src/cli/resolve.rs:436-437`.
  Unused sanitizer: `src/terminal_safe.rs` (`safe`).
- **Threat / impact:** integrity of the trust decision. The registry authors an
  error string that renders as a forged success line, so the operator reads
  `[Verified]` for a package that was not verified.
- **Mechanism:** `read_json_response` returns `error.error` (a free-form
  `String` bounded only by `MAX_JSON_BYTES`, `server.rs:690`) unmodified; it
  flows to `eprintln!` with no escaping. `terminal_safe::safe` exists for exactly
  this threat (its doc cites bug-24/bug-210) but the census stopped at `.mfp`
  header fields — **lead-verified** `grep -rn 'terminal_safe' src/` returns only
  `src/cli/pkg.rs:1837,1839,1847,1855,2036` and `src/audit/{text,json}.rs`; no
  registry-response string and not `pkg info`'s own `Release State` line.
- **Reproduction (lead-run):** a loopback registry returning
  `{"error":"\x1b[2K\rok: uses toolbox - [Verified]  ‮EVIL"}` on `GET /ident`;
  then in a scratch project `MFB_HOME=… MFB_REPO_URL=http://127.0.0.1:7799
  mfb pkg add 'alice#toolbox'`. Observed (`cat -v`):
  `error: ^[[2K^Mok: uses toolbox - [Verified]  M-bM-^@M-.EVIL` — raw
  erase-line + CR (wipes the `error: ` prefix) and U+202E RLO. Expected: every
  control byte rendered `\u{XXXX}` the way `mfb pkg info` renders the same bytes
  from a `.mfp`.
- **MFB trigger program (spike):** none possible — the untrusted input is an
  HTTP response body, not a `.mfb` program. The command repro above and
  `spikes/audit-3/SUP-02/` (the loopback-registry harness) stand in its place.
- **Best fix:** route every externally-sourced string through
  `terminal_safe::safe` at the print site (the single choke point): wrap
  `message` in `dispatch_command_error` (`cli/mod.rs:32,36`) and
  `detailed_message` in `show_general_diagnostic` (`rules/mod.rs:104`), and the
  registry-sourced operands at `pkg.rs:1874` and `resolve.rs:436-437`. Add a
  test asserting an ESC/`\u{202e}` registry error renders escaped.
- **Non-goals:** no `ErrorResponse` wire change; escape, don't truncate; don't
  alter the `severity[code NAME]:` header shape goldens pin.
- **Prior art:** extends **bug-24** and **bug-210** (both
  `bugs/completed/`, whose scope was `.mfp` header fields + `audit/text.rs`); no
  prior item covers the registry-response source. → **filed as bug-489.**

## SUP-03 — A cross-origin 307/308 redirect re-sends the request body, leaking the session token

- **Severity:** MEDIUM.
- **Location:** `repository/src/client.rs:152` (`ensure_redirect_target` — no
  same-origin check), `:134` (`redirect_policy`), body-borne credential at
  `:1379`/`:1391` and the 11 credentialed callers listed in the finding notes
  (publish, validate, link, rotate, transfer, tokens, org, release-state).
- **Threat / impact:** confidentiality of the session token (and, for
  `/publish`, the entire `.mfp`; for `/machines/link`, the sealed ident
  keypair). Triggerable by the configured registry, or by an open-redirect /
  subdomain-takeover / CDN misconfig on an honest one.
- **Mechanism:** **lead-verified** `ensure_redirect_target`
  (`client.rs:152-175`) rejects only non-https and blocked IP *literals*; a
  hostname target like `https://attacker.example/` passes. reqwest strips
  `Authorization`/`Cookie` on a cross-host hop (headers only), but 307/308
  preserve method and body, and the `session_token` is a **body** field — lead
  confirmed `grep -n session_token repository/src/client.rs` places it in the
  JSON body of every control-plane call (`:352,392,482,1042,…`). So a `POST
  /publish` answered `307 Location: https://attacker.example/x` re-posts the
  token (and payload) to the attacker.
- **Reproduction:** not demonstrated — the guard requires an https target, so a
  loopback harness cannot drive it without a trusted cert. The three code paths
  (guard, 307/308 body-preserve, token-in-body) are read directly and
  unconditional.
- **MFB trigger program (spike):** none possible — CLI/network boundary.
- **Best fix:** reject a redirect hop whose `(scheme,host,port)` differs from the
  configured registry origin for every route except `GET /blob/<hash>` (the only
  legitimately-redirecting, content-address-verified route). Cheapest form: give
  the credentialed `post_json`/`put_blob` calls a second client built with
  `redirect::Policy::none()` (no control-plane route is documented to redirect),
  keeping the shared client for blob GETs.
- **Non-goals:** don't break the presigned-URL 302 on `GET /blob`; don't
  reintroduce a per-call `Client`; keep the existing https-only / IP-literal
  checks.
- **Prior art:** none (searched redirect/SSRF/token-leak/bearer/sessionToken).
  bug-420 item 2 added the redirect guard for the SSRF/downgrade half; the
  credential half is new. → **filed as bug-490.**

## SUP-04 — `mfb pkg install` never binds the downloaded blob to the lockfile's `ident`/`selected`

- **Severity:** MEDIUM (constrained).
- **Location:** `src/cli/resolve.rs:415-431` (`install`), contrast
  `src/cli/pkg.rs:1226-1233` (`add_package_from_registry` *does* check
  `header.ident != full_ident`); `src/manifest/package.rs:366-371` (the only
  pinned-version comparison — its `Err` is discarded by all three callers at
  `:409,:471,:558`).
- **Threat / impact:** integrity. `install` fetches by hash and runs §3.5, but
  §3.5 only proves the artifact is *self-consistent and signed by the pinned
  owner* — nothing compares the installed header to the lock's `ident` /
  `selected` / `name`. Combined with SUP-01 (unsigned `/index`), a registry that
  maps `version → hash` freely can serve any of the owner's artifacts for a
  requested version.
- **Mechanism:** `fetch_blob(&package.hash)` → `install_verified_package(…,
  &package.name, …)` → `classify_installed_package`, whose `verify_attestation`
  binds to `package.ident`/`version` read from *the same file*. The would-be
  gate `installed_package_files` returns its pinned-version error into `let Ok(…)
  else { return … }` at all three call sites — dead as a gate. **Lead-verified:**
  `grep -rn 'installed_package_files' src/` → three lossy callers.
- **Constraint (why MEDIUM not HIGH):** to be *silently* accepted the
  substituted artifact must be signed by the same owner's pinned key **and**
  carry the same internal `name` (the resolver loads `packages/<name>.mfp` and
  prefixes symbols by the artifact's own name), so a mismatch usually degrades to
  a link/type failure rather than clean substitution.
- **Reproduction:** not demonstrated (needs a signing-capable registry).
- **MFB trigger program (spike):** none possible — CLI/registry boundary.
- **Best fix:** in `install`, parse the blob and refuse unless
  `header.ident == package.ident && header.version == package.selected &&
  header.name == package.name` — the same three-way check `pkg verify` already
  runs (`pkg.rs:2167`). Separately, route `installed_package_files`' pinned
  error to the build, or delete it so it stops reading as an enforced gate.
- **Non-goals:** compare against the lock's `selected`, not the manifest's
  `version` (an ABI floor for `pin:false`); don't break `file://`/source-dir
  deps that have no lock entry.
- **Prior art:** none for the install-side binding; audit-2 SUP-04 said install
  is "not blind" — still true; this narrows *which* property it proves.
  → **filed as bug-491.**

## SUP-05 — `put_blob`'s error path reads the response body with no size cap

- **Severity:** LOW. **Location:** `repository/src/client.rs:1329-1331`
  (`response.text()` unbounded, vs `read_body_capped` everywhere else).
- **Threat:** a registry answering a non-2xx `PUT /blob` with a multi-GB body
  forces an unbounded allocation in the publishing client (bounded only by the
  600 s `BLOB_TIMEOUT`). Integrity unaffected.
- **Best fix:** replace with `read_body_capped(response, MAX_JSON_BYTES,
  "repository error body")`, matching `fetch_blob`'s failure branch (`:1261`).
- **Prior art:** **bug-276 R3** (`bugs/completed/`) named this exact site and
  fixed the `fetch_blob` sibling but missed this one. → note on bug-276 (small).
- **Spike:** none — network boundary.

## SUP-06 — Key/session files chmod'd after creation and written through symlinks; `~/.mfb` never restricted

- **Severity:** LOW. **Location:** `repository/src/local.rs:386-390`
  (`write_private_file`: `fs::write` then `set_permissions(0o600)`), `:380-384`
  (`create_private_dir`), base dir `src/cli/mod.rs:160-173`.
- **Threat:** a local unprivileged user on a shared build host (not the
  registry). (a) A umask-022 window where a session JWT / ident key is briefly
  world-readable; (b) `fs::write`/`set_permissions` follow symlinks, so a link
  pre-planted at the target is written through; (c) `~/.mfb` itself keeps umask
  perms.
- **Best fix:** create with the final mode atomically
  (`OpenOptions…mode(0o600)`, `DirBuilder::mode(0o700)`, `create_new` where an
  overwrite isn't intended); restrict the base `~/.mfb`.
- **Prior art:** none (audit-1 recorded the 0600 *end-state* as a positive; the
  ordering wasn't examined). **Spike:** none.

## SUP-07 — `is_blocked_redirect_ip` misses 6to4, NAT64 and `0.0.0.0/8`

- **Severity:** NTH. **Location:** `repository/src/client.rs:177-206`.
- **Mechanism:** the IPv6 arm folds `::ffff:v4` but not `2002:<v4>::/16` (6to4)
  or `64:ff9b::/96` (NAT64); the IPv4 arm blocks only `0.0.0.0`, not
  `0.0.0.0/8`. Kept NTH because the function's own doc concedes the far larger
  hole (a *hostname* resolving to an internal address is out of scope), so an
  attacker uses a DNS name.
- **Best fix:** add the two prefixes and widen to `0.0.0.0/8`, or resolve the
  hop host and reject any resolved internal address. **Spike:** none.

## SUP-08 — Registry-supplied `hash` interpolated into the blob URL with no hex check

- **Severity:** NTH. **Location:** `repository/src/client.rs:1250` +
  `src/cli/resolve.rs:900`.
- **Mechanism:** `hash` reaches `format!("{}/blob/{}", …)` unvalidated; the
  scheme+authority prefix is fixed (no cross-host escape) and the post-fetch
  `sha256 != hash` check fails the request anyway, so no consequence is traced —
  recorded only because validating is free.
- **Best fix:** reject a `hash` not exactly 64 lowercase hex chars at the top of
  `fetch_blob`/`blob_exists`/`put_blob`. **Spike:** none.

## Checked and clean (recorded so a later audit does not re-derive)

- **No TLS bypass** — `rustls-tls`, `default-features = false`; no
  `danger_accept_invalid_*`; hostname check is webpki default
  (`repository/Cargo.toml:47`; tree-wide grep clean).
- **No install/resolve-time code execution** — `load_import_edges` decodes the
  blob in memory (`resolve.rs:730`, temp-file staging removed); no build hooks;
  no dependency `.mfb` is run. `Command::new` only in `build/test_mode.rs`.
- **No zip-slip / traversal** — `validate_package_name` forbids a leading `.`;
  `stage_package_blob` uses `create_new` (O_EXCL) + `rename`; vendor filenames
  re-validated at the install site (`pkg.rs:1313`), not trusted from the decoder.
- **Cache re-verified on use** — every build re-runs §3.5 on `packages/*.mfp`
  (`build/mod.rs:303`) and re-hashes vendored libs against the signed section-10
  table (`build/mod.rs:741`).
- **Dependency confusion closed** — an `IMPORT` of an undeclared package is
  `IMPORT_PACKAGE_NOT_DECLARED` (`resolver/packages.rs:9`).
- **Ident rotation is signature-chained** — `follow_ident_chain`
  (`client.rs:552`) verifies each link under the previous key;
  `PACKAGE_IDENT_REANCHORED` on a broken chain.
- **Transparency-log checks fail closed** — checkpoint rollback/fork
  (`client.rs:640`), pin-after-proof (`verify_log_consistency:745`, bug-276 R2),
  leaf binding (`verify_publish_inclusion:715`, bug-273); `verify_inclusion`'s
  `index >= tree_size` guard makes a negative server `i64` fail closed
  (`log.rs:76`).
- **audit-2 SUP-01/SUP-02 genuinely fixed** (see SUP-01 prior-art).

## Coverage

Read in full: `repository/src/client.rs:1-1520` (all production code),
`src/cli/resolve.rs:1-1010`, `src/cli/mod.rs:1-232`, `src/manifest/url.rs`,
`repository/src/local.rs:1-400`, `src/cli/build/packages.rs:55-250`,
`src/cli/build/native_libs.rs:100-185`, `src/terminal_safe.rs`,
`repository/src/log.rs:60-215`, `repository/src/validation.rs:9-38`.

Read in the parts that matter: `src/cli/pkg.rs` (add/install/select/verify
halves; publish/remove/doc halves not read), `src/cli/repo.rs` (dispatch +
register/auth/trust/link; later arms skimmed), `src/manifest/libraries.rs`
(bare-name + vendor hash; locator body not read).

Gaps: `src/cli/init.rs` grepped only (zero network hits — that is its security
answer); the `.mfp` decoder and `repository/src/crypto.rs` deferred to the
Surface 1 and Surface 6 passes. SUP-01/03/04/05 are "not demonstrated" (each
needs a signing-capable registry or a trusted-TLS endpoint); SUP-02 is the one
with a live repro, re-run by the lead.
