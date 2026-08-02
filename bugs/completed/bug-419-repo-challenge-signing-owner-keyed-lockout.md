# bug-419: registry `/auth/challenge` and `/signing` rate-limiters are keyed on the unauthenticated `owner` name → anonymous, targeted account-lockout DoS

Last updated: 2026-07-28
Effort: small (<1h)
Severity: MEDIUM
Class: Security (targeted denial-of-service / account availability)

Status: FIXED (see block at end)
Regression Test: repository/tests — a flood of `/auth/challenge {"owner":"victim"}`
from one IP must not cause the victim's own challenge/login to return 429.

The `mfb-repo` registry server rate-limits `/auth/challenge` and `/signing` using a
key built from the **unauthenticated, attacker-controlled `request.owner`**, and the
check runs **before** any owner/auth validation:

```rust
// challenge (server.rs:953)
if !state.rate_limiter.allow(&format!("challenge:{}", request.owner), 20, 60) {
    return Err(too_many_requests());
}
// signing (server.rs:2534)
if !state.rate_limiter.allow(&format!("signing:{}", request.owner), 60, 60) {
    return Err(too_many_requests());   // runs before verify_session_token (2538)
}
```

An anonymous attacker sends 20 `POST /auth/challenge {"owner":"victim",
"authFingerprint":""}` per 60s from a single IP; the victim's own (21st) challenge
then returns 429. Because a challenge is the **mandatory prerequisite for
`/auth/login`** (login needs a challenge nonce to sign), sustaining 20 req/min locks
the targeted account out of login entirely. Identically, flooding `signing:victim`
(60/60s) blocks a victim holding a valid session from obtaining publish
attestations → targeted publish-DoS.

Neither route has a per-IP key or a global ceiling — unlike `/register` and
`/login`, which bug-188/REPO-12 moved to a **per-IP** key for exactly this
lockout-avoidance reason. goal-05's audit-2 explicitly called `/auth/challenge` and
`/signing` "correctly per-owner", mis-judging per-owner-*name* as safe: `owner` is
unauthenticated attacker input, so "per-owner" is really "per-attacker-chosen
victim."

References:

- `repository/src/server.rs:953` (`challenge`), `:2534` (`signing`, before
  `verify_session_token` at :2538); contrast the per-IP keying that bug-188/REPO-12
  applied to `/register`,`/login`. `planning/completed/audit-2-repository.md`
  mis-classified these as safe. Found during goal-07.

## Failing Reproduction

Requires a live server (not run). Mechanism is direct from source: the rate-limit
key is an attacker-controlled string, the check is the first statement in each
handler, the cap is 20/60s (challenge) / 60/60s (signing), and there is no per-IP
or global backstop.

- Observed: 20 attacker `challenge:{owner}` hits/60s → the real owner's challenge
  returns 429 → cannot obtain a login nonce.
- Expected: an attacker cannot exhaust another account's challenge/signing budget
  (rate-limit per-IP and/or add a global ceiling, as `/register`/`/login` do).

## Root Cause

The rate-limit key is derived from the unauthenticated `owner` field, so anyone can
consume any account's challenge/signing allowance.

## Goal

- `/auth/challenge` and `/signing` are rate-limited by peer IP (and/or a global
  ceiling), so no anonymous client can lock out a specific account.

### Non-goals (must NOT change)

- The owner-existence probe semantics; the cryptographic auth checks. A per-owner
  limit *in addition to* a per-IP limit is fine — the per-IP one is the missing
  backstop.

## Blast Radius

- `repository/src/server.rs:953` (challenge), `:2534` (signing) — re-key to peer IP
  like `/register`/`/login`. Check for any other handler whose rate-limit key is an
  unauthenticated request field.

## STATUS: FIXED (a7ce385b7; fast-forwarded to main, tip 0633b2b54)

Reproduced from source (a live server was not required — the mechanism is direct):
each handler's first statement is `rate_limiter.allow(&format!("<route>:{}",
request.owner), ...)`, with `request.owner` unauthenticated attacker input and the
check preceding all auth. A RED test
(`challenge_rate_limit_cannot_lock_out_a_victim_by_owner_name`) confirmed the exact
mechanism: under owner-keying, an attacker's 25-request flood naming "victim" from
`10.9.9.9` made the victim's *own* valid challenge from `10.1.1.1` return **429**
(not a proxy — the victim used their real auth fingerprint, so only the rate limiter
could 429 it). The signing twin was proven the same way (the second-IP victim got
429 instead of the expected 400).

Fix: both buckets re-keyed to `peer.ip()` via a `ConnectInfo<SocketAddr>` extractor,
with `AUTH_GLOBAL_CEILING` as a secondary backstop, exactly following the per-IP
precedent bug-188/REPO-12 gave `/register` and `/login`. New `CHALLENGE_PER_IP_MAX
= 20` / `SIGNING_PER_IP_MAX = 60` preserve the prior numeric budgets, now spent per
client instead of per victim-name.

Blast-radius sweep: the only other `rate_limiter.allow` sites keyed on a request
field are `blob:{claims.sub}` and `{route}:{claims.sub}` — both derived from the
*authenticated* session, so they are safe and unchanged. The non-goals hold: owner
existence probe and crypto auth checks are untouched.

Tests: `challenge_rate_limit_cannot_lock_out_a_victim_by_owner_name` (new);
`signing_is_rate_limited_per_owner` rewritten to
`signing_is_rate_limited_per_client_ip`;
`challenge_rate_limit_trips_after_the_window_cap` retargeted to the per-IP constant;
~20 existing per-IP call sites threaded a peer IP. Full `mfb-repo` suite (the
workspace this fix lives in): **314 + 21 passed**, re-verified after merging main
(bug-417, disjoint) in.

Deviation: skipped `cargo fmt --all` (§9). Local rustfmt 1.9.0 churns ~513 committed
files (the repo is not maintained fmt-clean under it); the new blocks were
hand-formatted to match the in-file `/login` precedent instead.

Commit: `a7ce385b7` (worktree-B-419)
