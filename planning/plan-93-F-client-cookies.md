# plan-93-F: Client-side cookie jar (store `resp.cookies`, attach `Cookie`)

Last updated: 2026-10-10
Effort: large (3h–1d)
Depends on: plan-93-E (the `http::Cookie` type, `Response.cookies`, and the client
`Set-Cookie` parse). If plan-93-E is not complete, this plan cannot start, full stop.

The client is stateless one-shot: each `read`/`write` opens a connection, sends one
request, returns, and forgets everything — no session, no redirect following. After
plan-93-E a response carries its cookies as `resp.cookies AS List OF Cookie`, but
nothing remembers them. This sub-plan adds an explicit, caller-threaded **cookie jar**:
a value a program updates from a response and consults before the next request, which
yields the `Cookie` header for the cookies that match the target URL. Behavioral
outcome: a login request's session cookie goes into a jar, and the next request to the
same site is sent authenticated.

References:

- `planning/plan-93-E-server-cookies.md` — `http::Cookie`, `http::SameSite`,
  `Response.cookies`, and the cookie name/value rules this letter reuses.
- `src/docs/spec/stdlib/05_http.md` — §Non-Blocking Client, §Request Flow: the
  `read`/`write` wrappers and the five-call client the jar sits beside.
- `src/codegen/builtins/http/func_read.rs:__http_read`,
  `func_write.rs:__http_write`, `func_start_read.rs` — the request entry points; each
  takes a caller `headers` map, which is how the jar's `Cookie` header gets attached.
- `src/codegen/builtins/http/helper_build_request.rs:__http_buildRequest` — writes the
  caller's extra headers onto the request.
- RFC 6265 §5.1.3–§5.4 (domain-match, path-match, the storage model, the `Cookie`
  header). Cite the exact sections from the document when writing the spec text; this
  plan's summary of them is from memory.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-93-E complete (`Cookie`, `Response.cookies`, client `Set-Cookie` parse) | `ls planning/completed/plan-93-E-*` | NOT MET |
| Tree builds & tests green | `cargo test` | UNMEASURED — run before starting |

If plan-93-E is not complete, this plan cannot start, full stop — this letter does not
re-implement the cookie type or its wire parse, and does not absorb E's scope.

## 1. Goal

- A `CookieJar` value type: in-memory, copyable, no disk persistence. There is no
  constructor function; an empty jar is the record's default (`MUT jar AS http::CookieJar`).
- `http::updateJar(jar, url, resp) AS CookieJar` — returns the jar after storing,
  replacing or deleting each cookie of `resp.cookies` per the RFC 6265 storage rules,
  for a response that came from `url`.
- `http::jarHeaders(jar, url, headers) AS Map OF String TO String` — returns `headers`
  with a `Cookie` entry added for the jar's cookies that match `url`
  (`n1=v1; n2=v2`, longest path first). The result is passed to the existing
  `http::read` / `http::write` / `http::startRead`.
- Expired cookies are not sent and are evicted.

```text
MUT jar AS http::CookieJar
LET login = http::write(loginUrl, form, http::jarHeaders(jar, loginUrl, headers))
jar = http::updateJar(jar, loginUrl, login)
LET page = http::read(pageUrl, http::jarHeaders(jar, pageUrl, headers))
```

### Non-goals (explicit constraints)

- **No implicit global jar.** The jar is an explicit value the caller threads;
  `http::read`/`write` are unchanged and stay cookie-free unless handed the headers.
- **No disk/session persistence** — in-memory value only.
- **No redirect following.** The client still does not follow redirects.
- **No public-suffix list.** Domain matching uses RFC 6265's host/domain rules;
  rejecting a `Domain` that is a public suffix (e.g. `.com`) is a documented
  best-effort guard, not a full PSL implementation.
- **No change to `Cookie` or `Response`.** Both are owned by plan-93-E. What the jar
  needs beyond the wire shape lives in the jar's own entry record.
- **No `SameSite` enforcement.** `SameSite` governs a browser's cross-site requests;
  this client has no notion of an initiating site. The field is stored and ignored.

## 2. Current State

### Measured populations

| What | Count | Command |
|---|---|---|
| jar/session concept in the client | 0 | `grep -rliE 'jar\|session' src/codegen/builtins/http` → no files |

### Verified properties

- **The client is one-shot and stateless.** `05_http.md` §Request Flow: each
  `read`/`write` connects, sends one request with `Connection: close`, parses, and
  returns a plain copyable `Response`; neither follows redirects or retries. So a jar
  must be a caller-threaded value — there is no session object to hang it on.
  **VERIFIED** via spec.
- **The request entry points already take a caller `headers` map.** `__http_read(url,
  headers, method)` and `__http_write(url, body, headers, method)`. **VERIFIED** by
  reading both signatures. A jar therefore needs no new request entry point: it only
  has to produce a headers map.
- **`cookie` is an "extra" header the request builder passes through.**
  `__http_isExtraHeader` withholds only `host`, `user-agent`, `accept`, `connection`
  and `content-length`. **VERIFIED** by reading the function.
- **plan-93-E delivers the parsed cookies.** After E, `resp.cookies` holds one `Cookie`
  per `Set-Cookie` line, in wire order, already leniently parsed. **Precondition, not
  scope** (Prerequisites).

## 3. Design Overview

### The types

```text
TYPE JarEntry
  cookie    AS Cookie     ; as received; `domain` and `path` hold the RESOLVED values
  hostOnly  AS Boolean    ; TRUE when the response named no Domain
  expiresAt AS Integer    ; 0 = session cookie; else Unix seconds
END TYPE

TYPE CookieJar
  entries AS List OF JarEntry
END TYPE
```

`Cookie` stays wire-shaped. The two facts a jar needs that are not on the wire — whether
the cookie is host-only, and its absolute expiry (`Max-Age` is relative to when it
arrived) — live on `JarEntry`.

### Three pieces, all pure MFBASIC

1. **Store** — `__http_jarStore(jar, url, cookie, now) AS CookieJar`, one cookie at a
   time; `updateJar` folds it over `resp.cookies`:
   - `domain` empty → host-only, stored domain = the request host;
   - `domain` set → leading `.` dropped, lowercased; rejected unless the request host
     domain-matches it; rejected if it is a public suffix (best-effort list);
   - `path` empty or not starting with `/` → the default path (the request path's
     directory);
   - `secure` cookie from an `http` URL → rejected;
   - expiry: `maxAge > 0` → `now + maxAge`; `maxAge < 0` → delete; else `expires` if
     non-zero (in the past → delete); else 0 (session). `maxAge` wins over `expires`;
   - an entry with the same (name, domain, path) is replaced; a delete removes it.
2. **Select** — `__http_jarCookieHeader(jar, url, now) AS String`: keep entries that
   domain-match (exact host when `hostOnly`), path-match, satisfy `secure` vs the URL
   scheme, and are not expired; sort longest path first; join `name=value` with `; `.
   Empty string when none apply.
3. **Surface** — `http::updateJar` (store + evict expired) and `http::jarHeaders`
   (select, then set `cookie` on a copy of the caller's map; a `cookie` entry the
   caller already supplied wins and the jar adds nothing).

**Correctness risk (schedule last, behind fixtures):** domain-match, path-match and
expiry — the classic cookie-security footguns (a wrong domain-match leaks a cookie
cross-site). Pin each rule with a fixture: host-only vs `Domain` cookie, path-prefix
match, `Secure` withheld over http, expired eviction, and a `Domain=.com`-style
public-suffix rejection.

**Time source:** expiry needs "now" in Unix seconds — `datetime::now().seconds`
(confirm the exact spelling in Phase 1). The helpers take `now` as a parameter so the
fixtures can drive expiry without sleeping; only the two public functions read the
clock.

Gate is **runtime behavior** (the right cookies attached to the right requests), never
byte-identity.

### Rejected alternatives

- **`http::newJar()`** — a constructor function; the record's default is the empty jar.
- **`readWith`/`writeWith` returning `(Response, CookieJar)`** — duplicates every
  request entry point (including the five-call client) and needs a multi-value return.
  The two primitives compose with every existing entry point.
- **Extra fields on `Cookie`** (`hostOnly`, `expiresAt`) — a server handler would see
  fields that mean nothing to it.
- **Reading raw `Set-Cookie` lines in the jar** — plan-93-E already parses them into
  `resp.cookies`; a second parser would drift from it.

## Compatibility / Format Impact

- New public surface: `http::CookieJar`, `http::JarEntry`, `http::updateJar`,
  `http::jarHeaders`. Nothing existing changes; `http::read`/`write` remain cookie-free.

## Phases

### Phase 1 — `CookieJar`, storage, `updateJar`

- [ ] Register `JarEntry` and `CookieJar` in `http/mod.rs`; confirm
      `MUT jar AS http::CookieJar` builds and is empty.
- [ ] Confirm the "now in Unix seconds" call and record it in Corrections.
- [ ] Add `__http_jarStore` and `http::updateJar`.
- [ ] Tests, feeding hand-built `Response` values with `cookies` set: a host-only
      cookie; a `Domain` cookie; default path from the request path; overwrite by
      (name, domain, path); delete via negative `maxAge`; delete via past `expires`;
      `maxAge` wins over `expires`; a `Domain` that does not match the host is dropped;
      a `secure` cookie from an `http` URL is dropped.

Acceptance: after `updateJar`, the jar holds the expected entries with the right
domain, path, `hostOnly` and `expiresAt`; a deleted or rejected cookie is absent.
Commit: —

### Phase 2 — Selection + `jarHeaders` (behind the matching fixtures)

- [ ] Add `__http_jarCookieHeader` with domain-match, path-match, `secure`/scheme,
      expiry filtering and longest-path-first ordering.
- [ ] Add `http::jarHeaders`.
- [ ] Tests: path-prefix match; subdomain domain-match; a host-only cookie **not** sent
      to a subdomain; `secure` cookie withheld over http and sent over https;
      expired-not-sent (driven through the `now` parameter); ordering; a cross-site
      cookie **not** leaked; a caller-supplied `cookie` header left untouched.
- [ ] Tests: `Domain=.com` (public-suffix) rejected — best-effort guard fixture.
- [ ] End-to-end fixture against a local `http::server`: a login handler sets a session
      cookie (plan-93-E's `withCookie`), the client stores it with `updateJar`, and a
      second request built with `jarHeaders` arrives with that cookie in `req.cookies`.

Acceptance: the end-to-end fixture sends the stored cookie on the second request; each
matching-rule fixture holds, including the cross-site non-leak and the public-suffix
rejection.
Commit: —

## Validation Plan

- Tests: fixtures under `tests/rt-behavior/http/`; unit fixtures that hand `updateJar`
  a built `Response`, plus the end-to-end fixture through real wire bytes against a
  local `http::server`.
- Coverage check: green run with the cross-site non-leak and end-to-end fixtures
  present (they exercise domain-match and the full jar loop).
- Runtime proof: a program that logs into a local server, prints the jar, issues a
  second request with `jarHeaders`, and prints the authenticated response.
- Doc sync (`.ai/specifications.md`, `.ai/spec-content.md`, `.ai/man-content.md`):
  - `05_http.md`: a client Cookies section — the jar model, the storage and matching
    rules, and the limitations (no persistence, no redirects, no public-suffix list,
    `SameSite` not enforced).
  - Man text lives in the registry descriptors: the `CookieJar` / `JarEntry` record
    descriptions in `http/mod.rs` and new `func_update_jar.rs` / `func_jar_headers.rs`
    descriptors. Verify by rendering each page with `mfb man`.
- Acceptance: full `cargo test` + the acceptance harness on Linux + macOS.

## Open Decisions

- **Public-suffix handling** — recommend a small built-in reject-list for obvious
  suffixes (`.com`, `.org`, single-label domains) documented as best-effort, not a
  full PSL. (§Non-goals)
- **`JarEntry` visibility** — recommend exported and readable, so a program can print
  or inspect a jar. Alternative: an opaque jar with private fields, which would need
  accessor functions. (§3)

## Corrections

<Filled in during execution.>

## Summary

The jar is the stateful half of cookies and the one with real security stakes: RFC 6265
domain/path/secure matching and expiry, where a wrong rule leaks a cookie cross-site.
It is pure MFBASIC over an explicit caller-threaded value (no hidden state, no
redirects, no persistence), consumes the `Cookie` values plan-93-E already parses into
`resp.cookies`, and adds two functions that compose with every existing request entry
point. Every matching rule — especially the cross-site non-leak — is pinned by a
fixture.
