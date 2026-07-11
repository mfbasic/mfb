# bug-28: `net::connectTcp` named-argument alias table lists `timeoutMs` in two positions → 3-arg named form uncallable, and named `timeoutMs` mis-binds to the port slot

Last updated: 2026-07-08
Effort: small (<1h)

`src/builtins/net.rs::call_param_names` gives `CONNECT_TCP` the alias table
`&[&["host", "address"], &["port", "timeoutMs"], &["timeoutMs"]]` (`net.rs:107`).
The string `"timeoutMs"` appears in **both** position group 1
(`["port", "timeoutMs"]`) and position group 2 (`["timeoutMs"]`). The named-argument
normalizer resolves a name to the **first** group containing it, with no
backtracking — `param_names.iter().position(|aliases| aliases.iter().any(|alias|
alias == name))` (`src/syntaxcheck/builtins.rs:1662-1664`). So a named `timeoutMs`
always resolves to index 1, colliding with `port`.

Two concrete failures:

1. `net::connectTcp(host: "example.com", port: 80, timeoutMs: 5000)` — the valid
   3-arg host/port/timeoutMs overload called with named args — is **rejected** with
   a spurious `TYPE_DUPLICATE_ARGUMENT_NAME` ("supplies parameter `port` more than
   once"), because both `port` and `timeoutMs` land at index 1. The 3-arg form is
   thus uncallable via named args.
2. `net::connectTcp(host: "h", timeoutMs: 5000)` (port omitted) is **silently
   accepted** as `connectTcp("h", 5000)` — `timeoutMs` binds to index 1 (the port
   slot), the normalized arg list is `(String, Integer)`, and `resolve_call` accepts
   it as the host+port overload. The intended timeout is used as the destination
   **port**.

The single correct behavior a fix produces: every `connectTcp` overload is callable
with named arguments, and a named `timeoutMs` always binds to the timeout parameter
— never to `port`.

Severity MEDIUM: a reachable correctness defect — one overload is uncallable via
named args, and another form silently mis-binds a security-relevant value (port).

References:

- `src/builtins/net.rs:107` (CONNECT_TCP alias table with `timeoutMs` in two
  position groups).
- `src/syntaxcheck/builtins.rs:1661-1690` (`normalize_builtin_call_arguments`,
  first-match name resolution at `:1662-1664`, duplicate error at `:1677-1687`).
- Contrast: every other `timeoutMs`-bearing builtin (thread SEND/RECEIVE, tls
  CONNECT/ACCEPT, net ACCEPT/POLL/SET_*_TIMEOUT) lists `timeoutMs` in exactly one
  position group — none exhibit this; `connectTcp` is the sole table with a
  cross-position alias duplicate. `connectTcp(address:, timeoutMs:)` (the Address+
  timeout named form) works because `timeoutMs` correctly lands at position 1 with
  position 0 taken by `address`.
- Found during goal-01 review of `src/builtins/**`.

## Failing Reproduction

```
LET s = net::connectTcp(host: "example.com", port: 80, timeoutMs: 5000)  ' (1)
LET t = net::connectTcp(host: "h", timeoutMs: 5000)                       ' (2)
```

- Observed: (1) `TYPE_DUPLICATE_ARGUMENT_NAME: … supplies parameter `port` more
  than once`; (2) accepted, but connects to port 5000 with no timeout.
- Expected: (1) compiles and connects with a 5000 ms timeout; (2) either binds the
  timeout correctly (if a 2-arg host+timeout overload exists) or is a clear error —
  never silently uses 5000 as the port.

Contrast: positional `connectTcp` calls of every overload work; `connectTcp(address:
a, timeoutMs: t)` works.

## Root Cause

The alias table encodes two structurally different overloads (host/port/timeoutMs
vs address/timeoutMs) by putting `timeoutMs` at both position 1 and position 2. The
first-match, non-overload-aware normalizer cannot place a shared name at different
positions depending on the overload, so `timeoutMs` is pinned to position 1.

## Goal

- All `connectTcp` overloads are callable with named args; a named `timeoutMs`
  binds to the timeout parameter in every overload.

### Non-goals (must NOT change)

- Positional-call behavior (correct today).
- Other builtins' alias tables.

## Blast Radius

- `CONNECT_TCP` alias table (`net.rs:107`) and the shared normalizer. A
  metadata-invariant test should assert no alias string repeats across position
  groups in any `call_param_names` table (to catch siblings).

## Fix Design

Options, in order of preference:
(a) Make the normalizer overload-aware — resolve names after selecting an
    arity/overload, not against a merged max-arity table (fixes the whole class);
(b) drop `"timeoutMs"` from the position-1 group and special-case the Address
    overload's timeout naming;
(c) split `connectTcp`'s Address form into a distinct call name with an
    unambiguous positional layout.
Add the "no cross-position alias duplicate" invariant test regardless.

## Phases

### Phase 1 — failing test + audit

- [ ] Function tests for `net::connectTcp` named-arg overloads: the 3-arg
      host/port/timeoutMs form compiles; a named `timeoutMs` never binds to `port`.
      Confirm they fail today.
- [ ] Add the metadata-invariant test (no repeated alias across position groups).
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Apply (a) overload-aware normalization, or (b)/(c) if scoped smaller.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh`; positional connectTcp goldens byte-identical.

## Validation Plan

- Regression test(s): the named-arg connectTcp function tests + the invariant test.
- Runtime proof: a program using `connectTcp(host:, port:, timeoutMs:)` connects
  with the intended timeout.
- Full suite: `scripts/test-accept.sh`.

## Summary

A cross-position alias duplicate in one metadata table breaks named-arg resolution
for `connectTcp`; the robust fix is overload-aware name normalization plus an
invariant test forbidding the duplicate.
