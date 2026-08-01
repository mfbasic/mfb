# bug-419: registry `/auth/challenge` and `/signing` rate-limiters are keyed on the unauthenticated `owner` name → anonymous, targeted account-lockout DoS

Last updated: 2026-07-28
Effort: small (<1h)
Severity: MEDIUM
Class: Security (targeted denial-of-service / account availability)

Status: Open
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
