# bug-188: registry /validate and /publish have no per-owner rate limit or storage quota → CPU/disk exhaustion

Last updated: 2026-07-14
Effort: large (3h–1d)
Severity: MEDIUM
Class: Security

Status: Open
Regression Test: repository/tests/rate_limit_publish (to be added)

Registration on the `mfb-repo` registry is open, so "authenticated" is a
near-anonymous bar (register once, obtain a session). The two most expensive
routes, `POST /validate` (full `parse_mfp_package` + five Ed25519 verifications
per call) and `POST /publish` (permanent ~48 MiB blob writes + unbounded
transparency-log growth), have **no** per-owner rate limit and **no** storage
quota — only a single 64 MiB `DefaultBodyLimit` that bounds one body but not
aggregate abuse. A registered client can loop these to exhaust server CPU
(`/validate`) and disk + DB + Merkle-log rows (`/publish`). The single correct
behavior a fix produces: per-owner throttling on `/validate`+`/publish` and a
per-owner storage/version quota, so a single account cannot exhaust the service.

This is the new finding **REPO-13** (the residual aggregate-DoS half of the
audit-1 REPO-02 body-cap mitigation). See `planning/audit-2-repository.md`.

References:

- `planning/audit-2-repository.md` (REPO-13; related REPO-12 global-bucket lockout)
- `repository/src/server.rs:1712` (`validate_package`), `:1720` (`publish_package`),
  `:645-647` (routes + `DefaultBodyLimit::max(MAX_BODY_BYTES)`).
- Blob persisted at `repository/src/blobstore.rs:200`; log grows per publish
  (`repository/src/store.rs:1115`).
- Rate limiter exists but is applied only to register/challenge/login/signing
  (`repository/src/server.rs:31-50`, `:681`, `:704`, `:1531`, `:1606`).

## Failing Reproduction

```
# register + login to obtain a session (see repository client flow), then:
# CPU exhaustion via /validate (parses + 5 Ed25519 verifies per call, no store):
for i in $(seq 1 1000); do
  curl -sX POST "$REPO/validate" -H 'content-type: application/json' \
    -d "{\"artifact\":\"$BIG_BASE64\", ...}" >/dev/null &
done
# disk/DB/log growth via /publish of distinct versions:
for v in $(seq 1 1000); do
  curl -sX POST "$REPO/publish" -H 'content-type: application/json' \
    -d "{\"ident\":\"me#pkg\",\"version\":\"1.0.$v\", ...}" >/dev/null
done
```

- Observed: unbounded CPU consumption on `/validate` and unbounded disk + DB +
  transparency-log growth on `/publish`; no throttle engages.
- Expected: a per-owner sliding-window limit rejects sustained abuse (429) and a
  per-owner storage/version quota caps total bytes/versions.

## Root Cause

The `RateLimiter` (`server.rs:31-50`) is wired only into the auth routes;
`/validate` and `/publish` call `validate_package_request` with no `allow(...)`
gate and no storage accounting. The 64 MiB `DefaultBodyLimit` bounds a single
request body but nothing aggregate. Blob writes and log-entry inserts are
unconditional once a request passes validation.

## Goal

- A single registered owner cannot drive unbounded CPU via `/validate` or
  unbounded disk/DB/log growth via `/publish`: per-owner sliding-window limits
  reject sustained bursts, and a per-owner blob-bytes/version-count quota caps
  storage.

### Non-goals (must NOT change)

- Legitimate publish/validate throughput within quota.
- The signature-verification logic or `.mfp` format.
- Distributed rate limiting (a fronting proxy is assumed for prod; this is the
  in-process floor).

## Blast Radius

- `server.rs:1712` `validate_package`, `:1720` `publish_package` — add per-owner
  `allow(...)` gates.
- `repository/src/store.rs` / `blobstore.rs` — add per-owner storage accounting +
  quota enforcement at publish.
- `/validate` body cap — should be much smaller than 64 MiB (it only parses, does
  not store).
- Related REPO-12 (global register/login buckets) — separate small fix; not here.

## Fix Design

Add a per-owner sliding-window limiter (reuse the existing `RateLimiter` keyed by
`validate:{owner}` / `publish:{owner}`, matching the `signing:{owner}` pattern
already in place). Add a per-owner storage ledger (sum of stored blob bytes and
version count) checked at publish time; reject with a quota error past the cap.
Lower the `/validate` body cap to the maximum a parse needs. Rejected
alternative: relying solely on the fronting proxy — leaves the in-process service
defenseless when exposed directly.

## Phases

### Phase 1 — failing test + audit
- [ ] Add a test that a burst of `/validate` (and `/publish`) past the cap
      returns 429 / quota errors; confirm it currently succeeds unbounded.

### Phase 2 — the fix
- [ ] Per-owner `allow(...)` gates on both routes; per-owner storage/version
      quota at publish; smaller `/validate` body cap.

### Phase 3 — validation
- [ ] Registry test suite green; legitimate within-quota publish/validate
      unaffected.

## Validation Plan

- Regression test: burst-rejection + quota-rejection tests.
- Runtime proof: the reproduction loop is throttled/quota-capped.
- Full suite: `cd repository && cargo test`.

## Summary

Straightforward service-code change (the limiter primitive already exists); the
only design work is the per-owner storage ledger and choosing sane default
quotas. Pairs with REPO-12 (make register/login buckets per-client).
