# bug-956: package-manager spec (01–04) drift and small protocol status/message defects

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other

Status: Open
Regression Test: n/a (spec text), plus (proposed) repository/src/server.rs `negative_log_size_is_a_400`

**Owner decision (2026-10-05):** for item 8, `/validate` is a full dry run. It runs every publish check (identity,
manifest, header/manifest agreement (bug-950), signing, size and version rules) except storing the
package and appending to the log. A package that validates cleanly publishes, barring races. The
other items are spec or small protocol fixes as listed.

Items found while exercising the package-manager spec against a local `mfb-repo`
(127.0.0.1, data in /tmp) and /tmp/hardening/mfb-head. Each item is classed separately.

References: src/docs/spec/package-manager/{01_repository-protocol,02_key-store,04_owner-names}.md

## Items

1. **CODE: a negative log size is a 500, not a 400.**
   `curl -w '%{http_code}' '127.0.0.1:47716/log/proof/0?size=-1'` and `'/log/consistency?from=1&to=-1'`
   → `{"error":"log size must not be negative"} [500]`. bug-276 R6 made `Store::log_leaf_hashes` reject
   negative sizes, but `server.rs:log_inclusion_proof`/`log_consistency_proof` map every store error
   through `internal`. Expected 400 (a client error; the neighbouring `index<0`/`from>to` cases are 400).

2. **SPEC: malformed request bodies are not `400` + `{"error": …}`.** 01 says "Control requests use JSON …
   JSON responses" and "malformed input or an invalid/role-swapped proof yields `400`". axum's extractor
   rejections bypass that shape. Measured on `/accounts/register` and `/auth/challenge`: a missing field
   (`{"owner":"carol3"}`) or a wrong type (`"owner":1`) → **422** `text/plain` ("Failed to deserialize the JSON
   body …"). Truncated JSON (`{`) → 400 `text/plain`. No `Content-Type` → **415** `text/plain`. The client
   copes (`read_json_response` falls back to status+body), but a reimplemented client following the spec
   would not expect 422/415. Either document it or add a JSON rejection handler.

3. **SPEC: a consumed pairing row is not deleted.** 01 "Machine Link": "The blob row lives **600 seconds**,
   is deleted on first fetch". `store.rs:take_pairing_blob` runs
   `UPDATE pairing_blobs SET used_at = ?1, blob = x''`. After two successful links,
   `sqlite3 db "select id,length(blob),used_at from pairing_blobs"` → `1|0|1791239334`, `2|0|1791239527`.
   The ciphertext is destroyed, but the row (lookup, salt, approval key) remains until the expiry sweep.

4. **SPEC: 04 "Where Validation Runs" cites a test-only helper.** The table lists `store::create_challenge` as
   "server-side challenge issuance". That function is the "Legacy single-machine helper kept for tests"
   (only test callers: `grep -n '\.create_challenge(' repository/src/*.rs`). `/auth/challenge` uses
   `create_auth_challenge`. Also, `server.rs:challenge` looks the owner up before any validation, so
   invalid names report `unknown owner` rather than the grammar error: `POST /auth/challenge {"owner":"a-b"}` and
   `{"owner":"std"}` → `400 unknown owner`.

5. **SPEC: oversize `size`/`to` are silently clamped.** `GET /log/proof/0?size=999999` → `"size":25` (the current
   size), and `/log/consistency?from=3&to=99999` → `"to":25`, both 200. 01 documents only "(default: current)".
   State the clamp, or refuse a size above the current tree.

6. **SPEC: `MFB_REPO_SERVER_FINGERPRINT` whitespace-only counts as unset.** 02: "A nonempty
   `MFB_REPO_SERVER_FINGERPRINT` must match on first pin (trimmed …)". With `MFB_REPO_SERVER_FINGERPRINT="  "`
   the client prints the TOFU warning and pins. The value is trimmed before the emptiness test. Say "non-blank".
   (Upper-case + padded `" 10A12F… "` matched, as documented. `0000` → `does not match the expected
   MFB_REPO_SERVER_FINGERPRINT 0000; refusing to pin`, as documented.)

7. **SPEC: 02 "Encoding" messages.** A `.prv` file with valid base64 of the wrong length (16 bytes) yields
   `malformed local private key`, with no role, from `crypto::sign`/`public_from_private`, not
   `malformed local auth private key`. A garbage `<owner>.auth.pub` is silently ignored by `mfb repo auth`
   (auth succeeds), so 02's `read_auth_public_key` decode error is never seen there. This agrees with 01's
   "optionally cross-checks" but 02 does not say it.

8. **SPEC/UNCLEAR: `/validate` is not a complete dry run.** 04 labels `client::validate_package` "dry-run a
   publish", but `/validate` returns `valid: true` for a header/manifest `url` mismatch that `/publish` then
   refuses (`400 package header and signed manifest disagree: …`). 01's `/publish` paragraph does document the
   extra check. Measured with /tmp/hardening/B16/py/t_v2.py. See also
   B16-registry-publishes-header-manifest-identity-mismatch.

9. **SPEC (minor): `GET /index/<owner>#<unknown-package>` is 200.** For a registered owner and an unknown package
   it returns 200 with `"versions":[]` and a name binding. 01 lists only "An unknown owner or malformed ident
   yields `400`". The `/packages/…` routes return 404 `unknown package` for the same input.

10. **CODE (message): raw SQLite text in a protocol error.** `/keys/rotate` to a key that is already registered
    (the current ident, or a past one) → `400 failed to register rotated ident key: UNIQUE constraint failed:
    keys.fingerprint` (t_rot.py). Expected a protocol message, e.g. "this key is already registered".

## Verified as documented (not defects)

Registration (role-swapped proof → 400, same key for both roles → role-separation 400, case-duplicate → 409
`owner name 'BOB' is already in use`), login replay → 409 `reused challenge`, unknown challenge 400, bad
signature does not consume the challenge, JWT `alg:none`/tampered `sub` → `expired or malformed session
token`, revoked machine's session → `unknown session token` and its next login → `mismatched local key
fingerprint`, purpose-bound revoke/login challenges, pairing approval failure does not burn the pairing, blob
PUT (401 without/with non-`Bearer` auth, 400 hash mismatch/uppercase, 201 then 200, 413 at 64 MiB+1, 201 at
exactly 64 MiB), HEAD 200/404, GET immutable cache header, `/search` per-IP cap (120 → 429), limit/`%`/`_`/empty
handling, HTML CSP/nosniff/no-referrer headers + escaping + `safe_href` (tab-split `JaVa\tScRiPt:` withheld),
rotation chain + pin-follow notice, transfer chain notice, staged-rotation promotion after a committed
rotation with only `.ident.next.prv` present, reanchor → `PACKAGE_IDENT_REANCHORED`, init-root one-time,
renew-root wrong key/registry-id refusals, reanchor-root → client hard fail, redirect to `http://` refused,
checkpoint ROLLBACK/fork detection, `HOME`/`MFB_HOME` both unset → `HOME is not set`, file modes 0600/0700.
