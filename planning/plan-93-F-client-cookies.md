# plan-93-F: Client-side cookie jar (`Set-Cookie` capture + `Cookie` attach)

Last updated: 2026-08-09
Effort: large (3h–1d)
Depends on: plan-93-E (reuses its `__http_serializeCookie`/attribute parsing and
the cookie value rules). If plan-93-E is not complete, this plan cannot start,
full stop.

The client is stateless one-shot: each `read`/`write` opens a connection, sends
one request, returns, and forgets everything — no session, no redirect following,
no cookies. This sub-plan adds an explicit, caller-threaded **cookie jar**: a value
a program passes into a jar-aware request, which captures `Set-Cookie` from the
response and, on the next request to a matching host/path, attaches the stored
cookies as a `Cookie` header. Behavioral outcome: a login request stores the
session cookie in a jar, and a subsequent jar-aware request to the same site is
sent authenticated automatically.

References:

- `src/docs/spec/stdlib/05_http.md:189-233` — the non-blocking core and the thin
  `read`/`write` blocking wrappers this adds jar-aware variants beside.
- `src/builtins/http_package.mfb:__http_read` / `__http_write` — the entry points
  to give jar-aware overloads.
- `src/builtins/http_package.mfb:__http_parseResponse` — where `Set-Cookie` lines
  are captured (note: the response header map is last-wins, so raw `Set-Cookie`
  lines must be captured **before** the map collapses them — see Verified props).
- plan-93-E — the shared cookie name/value rules and attribute serializer.

## Prerequisites

Shared feature gate: plan-93-A's "tree builds & tests green". Plus:

| Must be true | Command | Status |
|---|---|---|
| plan-93-E complete (shared cookie serializer + rules exist) | `ls planning/completed/plan-93-E-*` | NOT MET |

If plan-93-E is not complete, this plan cannot start, full stop — this letter does
not re-implement the cookie serializer or absorb E's scope.

## 1. Goal

- A `CookieJar` value type and constructor `http::newJar() AS CookieJar` (an
  in-memory, copyable value — no disk persistence).
- Jar-aware request entry points that thread the jar functionally:
  `http::readWith(jar, url, headers, method) AS (Response, CookieJar)` and
  `http::writeWith(jar, url, body, headers, method) AS (Response, CookieJar)` —
  or, if multi-return is awkward in the surface, `http::attachCookies(jar, url) AS
  Map` + `http::updateJar(jar, url, resp) AS CookieJar` primitives the caller
  composes. (Surface shape is an Open Decision; both give the same behavior.)
- On response, every `Set-Cookie` is parsed (name, value, `Domain`, `Path`,
  `Max-Age`/`Expires`, `Secure`, `HttpOnly`, `SameSite`) and stored/replaced/
  deleted in the jar per RFC 6265 storage rules.
- On request, the jar selects cookies whose domain/path/secure match the target
  URL and emits a single `Cookie: n1=v1; n2=v2` header (longest-path first).
- Expired cookies (`Max-Age`/`Expires` in the past) are not sent and are evicted.

### Non-goals (explicit constraints)

- **No implicit global jar.** The jar is an explicit value the caller threads;
  `http::read`/`write` stay cookie-free and unchanged. No hidden state.
- **No disk/session persistence** — in-memory value only. Serializing a jar to
  storage is a future, out-of-scope concern.
- **No redirect following.** Cookies are captured/sent per explicit request only;
  the client still does not follow redirects (existing behavior).
- **No public-suffix list.** Domain matching uses RFC 6265's host/domain rules;
  rejecting a `Domain` that is a public suffix (e.g. `.com`) is a documented
  best-effort guard, not a full PSL implementation (call out the limitation).
- No change to the `Response` record; the jar is separate.

## 2. Current State

### Measured populations

| What | Count | Command |
|---|---|---|
| cookie handling anywhere in the package | 0 | `grep -ci cookie src/builtins/http_package.mfb → 0` |
| jar/session concept in the client | 0 | `grep -ciE 'jar\|session' src/builtins/http_package.mfb → 0` |

### Verified properties

- **The client is one-shot and stateless.** Spec `05_http.md:6-11,210-233`: each
  `read`/`write` connects, sends one request with `Connection: close`, parses,
  returns a plain copyable `Response`; "Neither entry point follows redirects or
  retries." So a jar must be an explicit caller-threaded value — there is no
  session object to hang it on. **VERIFIED** via spec.
- **Response headers collapse last-wins — `Set-Cookie` must be captured raw.** The
  parser writes headers into a `Map` with `collections::set` (`05_http.md:44-58`),
  so multiple `Set-Cookie` lines collapse to the last. The jar update must read the
  **raw header lines** in `__http_parseResponse` before/alongside the map, not
  `resp.headers["set-cookie"]`. **VERIFIED** via spec Header Model; confirm the
  parse function keeps raw lines available (or add that capture).
- **plan-93-E provides the attribute parser/serializer.** This letter reuses E's
  `__http_serializeCookie` and its name/value validation rather than duplicating
  them. **Precondition, not scope** (Prerequisites).

## 3. Design Overview

Four pieces layered on the existing stateless client, all pure MFBASIC:

1. **`CookieJar` value** — a `List` of stored-cookie records
   `{ name, value, domain, path, secure, httpOnly, sameSite, expiresAt, hostOnly }`.
   `http::newJar()` returns an empty jar. Copyable value semantics (like `Response`).
2. **Capture** — `__http_jarStore(jar, url, setCookieLines) AS CookieJar`: parse
   each raw `Set-Cookie` (via E's parser), apply RFC 6265 storage (default domain =
   request host as host-only; default path = request-path directory; reject
   `Domain` not domain-matching the host; a past expiry = delete). Returns a new jar.
3. **Select** — `__http_jarCookieHeader(jar, url) AS String`: filter by domain-match
   + path-match + `Secure`-vs-scheme + not-expired, sort longest-path-first, join
   `name=value` with `; `. Empty string when none apply.
4. **Surface** — jar-aware `readWith`/`writeWith` (or `attachCookies`+`updateJar`)
   that call `__http_jarCookieHeader` before the request (merging with caller
   headers) and `__http_jarStore` after the response, returning both the `Response`
   and the updated jar.

**Correctness risk (schedule last, behind fixtures):** RFC 6265 domain/path
matching and expiry — the classic cookie-security footguns (a wrong domain-match
leaks a cookie cross-site). Pin each rule with a fixture: host-only vs `Domain`
cookie, path-prefix match, `Secure` withheld over http, expired eviction, and a
`Domain=.com`-style public-suffix rejection.

**Time source:** expiry needs "now". Use the existing datetime facility
(`datetime::`) for the current instant; confirm the exact call. Do not invent a
clock. This is the one external dependency of the matching logic.

Gate is **runtime behavior** (the right cookies attached to the right requests),
never byte-identity.

## Compatibility / Format Impact

- New public surface: `CookieJar` type, `http::newJar`, and the jar-aware request
  entry points. Nothing existing changes; `http::read`/`write` remain cookie-free.

## Phases

### Phase 1 — `CookieJar` value, `newJar`, capture

- [ ] Add the `CookieJar`/stored-cookie record types and `http::newJar()`.
- [ ] Add `__http_jarStore` capturing raw `Set-Cookie` lines from a parsed response
      (ensure `__http_parseResponse` exposes raw set-cookie lines; add capture if
      not), applying RFC 6265 storage + defaults + deletion-on-past-expiry.
- [ ] Tests: fixtures for storing a host-only cookie, a `Domain` cookie, overwrite
      by (name,domain,path), and delete via past `Max-Age`.

Acceptance: after a response with `Set-Cookie`, the jar contains the expected
stored cookies with correct domain/path/expiry; a past-expiry cookie is absent.
Commit: —

### Phase 2 — Selection + jar-aware requests (behind the matching fixtures)

- [ ] Add `__http_jarCookieHeader` with domain-match, path-match, `Secure`/scheme,
      expiry filtering and longest-path-first ordering.
- [ ] Add jar-aware `readWith`/`writeWith` (or `attachCookies`+`updateJar`) that
      attach the `Cookie` header (merged with caller headers, caller wins on an
      explicit `Cookie`) and update the jar from the response.
- [ ] Tests: fixtures for path-prefix match, subdomain domain-match, `Secure` cookie
      withheld over http/sent over https, expired-not-sent, ordering, and a
      cross-site cookie **not** leaked; an end-to-end "login sets cookie → second
      request is authenticated" fixture against a local `http::server`.
- [ ] Tests: `Domain=.com` (public-suffix) rejected — best-effort guard fixture.

Acceptance: the end-to-end fixture sends the stored cookie on the second request;
each matching-rule fixture holds, including the cross-site non-leak and the
public-suffix rejection.
Commit: —

## Validation Plan

- Tests: fixtures under the http client test home; drive against a local
  `http::server` (from plan-93-C/D/E) so capture→attach round-trips through real
  wire bytes, plus unit fixtures feeding canned `Set-Cookie` lines to the jar.
- Coverage check: green run with the cross-site non-leak and end-to-end auth
  fixtures present (they exercise domain-match and the full jar loop).
- Runtime proof: a program that logs into a local server, prints the jar, issues a
  second `readWith`, and prints the authenticated response.
- Doc sync: extend `05_http.md` with a client Cookies section (jar model, matching
  rules, the no-PSL/no-persistence limitations); add man pages for `newJar` and the
  jar-aware entry points; update the http `types` man page for `CookieJar`.
- Acceptance: full `cargo test` + acceptance harness on Linux + macOS.

## Open Decisions

- **Surface shape** — recommend the composable primitives
  `http::attachCookies(jar, url) AS Map` + `http::updateJar(jar, url, resp) AS
  CookieJar` (avoids a tuple/multi-return in the language surface and lets callers
  use them with the existing `read`/`write`). Alternative: `readWith`/`writeWith`
  returning `(Response, CookieJar)` if the surface supports ergonomic multi-return.
  Decide from what the language expresses cleanly. (§1, §3)
- **Clock source** — recommend `datetime::` for "now" (confirm the exact call);
  needed for expiry. (§3)
- **Public-suffix handling** — recommend a small built-in reject-list for obvious
  suffixes (`.com`, `.org`, single-label domains) documented as best-effort, not a
  full PSL. (§Non-goals)

## Corrections

<Filled in during execution.>

## Summary

The largest cookie piece and the one with real security stakes: RFC 6265
domain/path/secure matching and expiry, where a wrong rule leaks a cookie
cross-site. It is pure MFBASIC over an explicit caller-threaded jar (no hidden
state, no redirects, no persistence) and reuses plan-93-E's serializer. Every
matching rule — especially the cross-site non-leak — is pinned by a fixture.
