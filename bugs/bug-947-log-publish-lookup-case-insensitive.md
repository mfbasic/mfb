# bug-947: `/log/publish` returns another release's entry for idents/versions that differ only in ASCII case

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: (proposed) repository/src/store.rs `publish_log_lookup_is_case_sensitive`

`Store::publish_log_entry` finds a release's publish log entry with
`payload LIKE '{"ident":<i>,"version":<v>,%' ESCAPE '\'`. SQLite `LIKE` is
**case-insensitive for ASCII** by default, so the REPO-14 escaping fix still
cross-matches any earlier entry whose ident or version differs only in case.
The registry stores such releases as separate rows (`packages.ident` and
`package_versions.version` are compared exactly), so they are reachable:
`alice#toolbox` and `ALICE#toolbox`, or `1.0.0-RC` and `1.0.0-rc` (semver treats
pre-release tags case-sensitively). `ORDER BY idx ASC LIMIT 1` then returns the
**older** release's entry for the newer one.

The same lookup feeds `GET /index` (`versions[].logEntry`) and
`GET /packages/<ident>/audit` (`publishes[]` index/leaf/proof), so all three
surfaces name the wrong entry.

Effects: `mfb repo publish` reports a failure (exit 1) after a publish that
succeeded, and `mfb pkg verify --proof` hard-fails with
`PACKAGE_ATTESTATION_INVALID` for the later release, permanently. No forgery is
possible, because the client checks the leaf against the content. The correct
behavior is that `GET /log/publish?ident=&version=` matches ident and version
byte-exactly.

References:

- src/docs/spec/package-manager/01_repository-protocol.md, "Transparency Log": "`GET /log/publish?ident=<i>&version=<v>` → the publish entry's `{"index", "leafHash"}`", and "`mfb pkg verify --proof` additionally demands a verifying inclusion proof for each Verified dependency's publish entry."
- bugs/completed/bug-276 / REPO-14 (the LIKE-wildcard escape, same function)

## Failing Reproduction

Local registry `mfb-repo --dbpath /tmp/.../db.sqlite --datapath /tmp/.../data --listen 127.0.0.1:47716`,
`MFB_HOME`/`HOME` under /tmp, `MFB_REPO_URL=http://127.0.0.1:47716`, binary /tmp/hardening/mfb-head.

```
mfb repo register alice && mfb repo auth alice
mfb init-pkg toolbox && mfb repo publish alice toolbox          # alice#toolbox@0.1.0, OK
cp -r toolbox tb2; set "ident": "ALICE#toolbox" in tb2/project.json
mfb repo publish alice tb2
curl '127.0.0.1:47716/log/publish?ident=ALICE%23toolbox&version=0.1.0'
```

- Observed:
  ```
  Published ALICE#toolbox@0.1.0 as 936c68ec…
  Publish logged at index 10 (leaf 3bb7bca0…)
  error: publish log leaf does not match ALICE#toolbox@0.1.0: the registry served an entry for different contents
  ```
  (exit 1). `GET /index/ALICE%23toolbox` also lists `logEntry {index: 6, leafHash: 851323…}`, and
  `/packages/ALICE%23toolbox/audit` lists `publishes[0].index = 6` with alice#toolbox's proof. curl returns `{"index":6,"leafHash":"851323…"}`, which is alice#toolbox's entry, not 10.
  A consumer that did `mfb pkg add 'ALICE#toolbox'` then runs `mfb pkg verify --proof`:
  `error[6-605-0004 PACKAGE_ATTESTATION_INVALID] … publish log leaf does not match ALICE#toolbox@0.1.0`.
- Same with versions: publishing `alice#urltest@0.9.0-RC` and then `@0.9.0-rc` → the second
  publish exits 1 with the same "publish log leaf does not match … 0.9.0-rc".
- Expected: `/log/publish` returns index 10 for `ALICE#toolbox@0.1.0`; both publishes and
  `pkg verify --proof` succeed.

Contrast: idents/versions that differ in anything other than ASCII case resolve correctly
(the REPO-14 `_`/`%` cross-match test passes).

## Root Cause

`repository/src/store.rs:Store::publish_log_entry` uses
`WHERE kind = 'publish' AND payload LIKE ?1 ESCAPE '\'`, and `LIKE` folds ASCII case
(no `PRAGMA case_sensitive_like`). Every other identity lookup (`package_owner`,
`package_version_exists`) uses `=`.

## Goal

- `/log/publish` matches the ident/version prefix exactly. Two options: `substr(payload, 1, length(?1)) = ?1`,
  `instr`-based matching, or `GLOB` with escaped metacharacters. Better still, store ident/version in columns or
  look the index up through `package_versions` (which already records `logEntry`).

### Non-goals (must NOT change)

- The log payload format and leaf hashes.

## Blast Radius

- `store.rs:publish_log_entry` — fixed by this bug.
- `server.rs` index/audit builders that call `publish_log_entry` (`package_index`, `package_audit`) — fixed by the same change.
- `store.rs:search_packages` — uses `lower(...) LIKE` deliberately (case-insensitive search); unaffected.
