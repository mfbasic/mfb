# plan-93-E: The `Cookie` type and its wire format (server read + emit, client parse)

Last updated: 2026-10-10
Effort: large (3h–1d) — a guess; the record changes touch every `Request`/`Response`
constructor and their fixtures.
Depends on: bug-1021 (partial by-field record constructor). Plan-93-F (the client
cookie jar) depends on this letter.

The `http` package has no cookie support in either direction. This sub-plan adds one
value type, `http::Cookie`, used on both sides of the wire, and the code that moves it
on and off the wire: the server parses the request `Cookie` header into `req.cookies`
and emits one `Set-Cookie` line per entry of `resp.cookies`; the client parses every
`Set-Cookie` line of a response into the same `resp.cookies`. Behavioral outcome: a
handler reads `collections::get(req.cookies, "session").value`, sets a cookie with
`http::withCookie(resp, http::Cookie[name := "session", value := id, httpOnly := TRUE])`,
and a client reading that response finds the same `Cookie` in `resp.cookies` — even when
the handler set several.

There is no cookie constructor function. A `Cookie` is built with the by-field record
constructor, naming only the fields that differ from their defaults (bug-1021).

References:

- `src/codegen/builtins/http/mod.rs` — the `add_record` registrations for `Response`,
  `Request`, `RequestPart`, `Route`; `Cookie` and the `SameSite` enum register here.
- `src/codegen/builtins/http/helper_serialize_head.rs:__http_serializeHead` — writes
  one line per `resp.headers` entry; gains the `Set-Cookie` lines.
- `src/codegen/builtins/http/helper_check_response.rs:__http_checkResponse` — turns a
  response that cannot be serialized safely into a `500` (bug-506); must cover cookies.
- `src/codegen/builtins/http/helper_parse_request.rs:__http_parseRequest` and
  `helper_request_header_map.rs:__http_requestHeaderMap` — the strict server-side
  request parse the `Cookie` header parse hangs off.
- `src/codegen/builtins/http/helper_parse_response.rs:__http_parseResponse` and
  `helper_header_map_from_head.rs:__http_headerMapFromHead` — the lenient client-side
  parse; the header map collapses duplicates last-wins, so `Set-Cookie` lines are read
  from the head text, not from the map.
- `src/codegen/builtins/http/func_with_header.rs` — the combinator `withCookie` mirrors.
- `src/docs/spec/stdlib/05_http.md` — §Response Record, §Header Model, §Server →
  Value records / Request parsing / Constructors, combinators, static helpers.
- `bugs/bug-1021-by-field-record-constructor-requires-every-field.md`.
- RFC 6265 (cookie syntax, `Set-Cookie` attributes) and its successor draft
  `rfc6265bis` (the `SameSite` attribute — RFC 6265 itself has none). Cite the exact
  sections from the documents when writing the spec text; this plan's summary of them
  is from memory.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| bug-1021 fixed: `Ck[name := "a"]` builds, omitted fields take their defaults | `ls bugs/completed/bug-1021-*` | NOT MET |
| An enum has a default, so an omitted `sameSite` is `Unset` (owner: enums have or will have a default) | Phase 0 probe | UNMEASURED |
| Tree builds & tests green | `cargo test` | UNMEASURED — run before starting |

If any prerequisite is not met, this plan cannot start, full stop.

## 1. Goal

- A `SameSite` enum and a `Cookie` record, exported from `http`, shared by client and
  server (§3 gives the fields).
- `Request` gains `cookies AS Map OF String TO Cookie` — the parsed request `Cookie`
  header, keyed by cookie name (case-sensitive), last-wins on a repeated name.
- `Response` gains `cookies AS List OF Cookie`. On the server it is what a handler sets
  and the server emits, one `Set-Cookie` line per entry, in list order. On the client it
  is what the response parser found, one entry per `Set-Cookie` line, in wire order.
- `http::withCookie(resp, cookie) AS Response` returns a copy of `resp` with `cookie`
  appended to `cookies`.
- **Multiple `Set-Cookie` values coexist** in both directions: two cookies set by a
  handler are two lines on the wire, and a client sees both.
- A cookie that cannot be emitted safely is never emitted raw: the response is answered
  `500`, exactly like a header carrying a control byte today.

### Non-goals (explicit constraints)

- **No constructor function** — no `http::cookie(...)`, no `http::cookieDefault()`. The
  by-field record constructor is the only way to build a `Cookie`.
- **No cookie jar, no statefulness.** Storing cookies between requests and choosing
  which to send is plan-93-F. This letter only moves cookies on and off the wire.
- **No signed or encrypted cookies.** A cookie's value is the caller's string.
- **No change to the last-wins header model.** `headers` stays a
  `Map OF String TO String` on both records. `Set-Cookie` no longer travels through it
  on the server; on the client the map keeps its last-wins `set-cookie` entry as today.
- **No percent-encoding or decoding of cookie values.** A value goes out and comes in
  byte-for-byte; a caller that needs arbitrary bytes encodes them itself.

## 2. Current State

### Measured populations

| What | Count | Command |
|---|---|---|
| cookie handling in the package | 0 functions; 1 comment mention | `grep -rli cookie src/codegen/builtins/http` → `helper_check_response.rs` only |
| `Request[` constructors in the package | 2 | `grep -rn 'Request\[' src/codegen/builtins/http` (`__http_emptyRequest`, `__http_parseRequest`) |
| `Response[` constructors in the package | 5 | `grep -rn 'Response\[' src/codegen/builtins/http` |
| fixtures building `http::Request[` positionally | 2 | `grep -rln 'http::Request\[' tests examples src/docs` |
| fixtures building `http::Response[` positionally | 1 | `grep -rln 'http::Response\[' tests examples src/docs` |
| `Request` record fields today | 8 | `05_http.md` §Server → Value records |

### Verified properties

- **A response header map cannot hold two `Set-Cookie` values.** `__http_serializeHead`
  loops `resp.headers`, a `Map OF String TO String`, writing one line per entry.
  **VERIFIED** by reading the function. (`withHeader` keys are case-sensitive, so
  `"Set-Cookie"` and `"set-cookie"` do emit two lines today; that is an accident of the
  map, not a mechanism, and the docs must not recommend it.)
- **The control-byte guard only scans `resp.headers`.** `__http_checkResponse` and the
  `FAIL` backstop in `__http_serializeHead` iterate `resp.headers` and check
  `resp.reason`. A cookie list beside the map is outside both. **VERIFIED** by reading
  both functions. Emitting cookies without extending both reopens bug-506.
- **Both records are public and built positionally by callers.** The fixtures in the
  table above construct `http::Request[...]` and `http::Response[...]` with every field
  in order. Adding a field is a compile error for any such program. **VERIFIED** by the
  grep. This is a breaking change to state in the spec and to fix in the three fixtures
  (a fixture edited because the record it constructs gained a field is not a weakened
  test; the assertion it makes is unchanged).
- **The client collapses `Set-Cookie` before anyone can read it.** `__http_parseResponse`
  hands the head text to `__http_headerMapFromHead`, which is last-wins. **VERIFIED** by
  reading `__http_parseResponse`. The cookie parse must read the head text itself.
- **A request has one `Cookie` header in HTTP/1.1.** The strict request parse collapses
  a repeated field last-wins, so a second `Cookie` line replaces the first.
  **UNVERIFIED** against the RFC text — confirm RFC 6265 §5.4 forbids a client sending
  more than one, and document the last-wins outcome either way.

## 3. Design Overview

### The types

```text
ENUM SameSite
  Unset     ; no SameSite attribute on the wire
  Strict
  Lax
  None
END ENUM

TYPE Cookie
  name     AS String
  value    AS String
  path     AS String     ; "" = not set
  domain   AS String     ; "" = not set
  maxAge   AS Integer    ; 0 = not set; > 0 = seconds to live; < 0 = delete now
  expires  AS Integer    ; 0 = not set; else Unix time in seconds
  secure   AS Boolean
  httpOnly AS Boolean
  sameSite AS SameSite
END TYPE
```

Every field's default is its "not set" value, so a by-field constructor names only what
it wants:

```text
http::Cookie[name := "theme", value := "dark"]
http::Cookie[name := "session", value := id, path := "/", httpOnly := TRUE,
             secure := TRUE, sameSite := http::SameSite.Lax]
http::Cookie[name := "session", maxAge := -1]        ' delete it
```

`Unset` is distinct from `None`: `Unset` emits no attribute and leaves the choice to the
browser (the `rfc6265bis` "Default" state); `None` emits `SameSite=None`, an explicit
request for cross-site sending.

`maxAge` and `expires` both use 0 for "not set" because 0 is the `Integer` default. A
wire `Max-Age=0` means "expire now", so the record spells that as a negative `maxAge`.
`expires` is whole seconds because an HTTP date has one-second resolution and
`datetime::Instant.seconds` is already that count.

### On a request, only `name` and `value` are real

A client sends `Cookie: a=1; b=2` and nothing else. Every `Cookie` in `req.cookies` has
its other fields at their defaults, whatever attributes it was set with:
`req.cookies["session"].httpOnly` is `FALSE` for an `HttpOnly` cookie. The spec and the
`Cookie` man text must say so in plain words.

### Four pieces

1. **Types** — register `SameSite` (`add_enum`) and `Cookie` (`add_record`); add
   `cookies` to `Request` and `Response`; update every constructor in the census.
2. **Server read** — `__http_parseCookieHeader(value AS String) AS Map OF String TO Cookie`,
   called from `__http_parseRequest` on the `cookie` header:
   - split on `;`, trim each piece;
   - split the piece at the **first** `=` (a value may contain `=`);
   - a piece with no `=`, or an empty name, is skipped — never a `400`;
   - one pair of surrounding double quotes is stripped from the value;
   - no percent-decoding;
   - a repeated name is last-wins.
   An absent header gives an empty map.
3. **Server emit** — `__http_serializeCookie(c AS Cookie) AS String` renders one
   `Set-Cookie` value: `name=value`, then `Path`, `Domain`, `Max-Age`, `Expires`,
   `Secure`, `HttpOnly`, `SameSite`, each only when set. A negative `maxAge` renders
   `Max-Age=0`. `expires` renders as an HTTP date (IMF-fixdate, GMT).
   `__http_serializeHead` writes `Set-Cookie: <value>` for every entry of
   `resp.cookies`, in order, after the header map.
   `__http_cookieIsValid(c AS Cookie) AS Boolean` is the one validity rule, called by
   `__http_checkResponse` (invalid → the built-in `500`) and by the `FAIL` backstop in
   `__http_serializeHead`:
   - `name` is non-empty and an HTTP token;
   - `value` has no control character, space, double quote, comma, semicolon or
     backslash;
   - `path` and `domain` have no control character and no `;`;
   - `sameSite = None` requires `secure`.
4. **Client parse** — `__http_parseSetCookie(line AS String) AS Cookie` is the inverse
   of piece 3, and lenient like the rest of the client parse:
   - attribute names are case-insensitive; an unknown attribute is ignored;
   - a `SameSite` value that is not `Strict`/`Lax`/`None` is `Unset`;
   - wire `Max-Age` ≤ 0 becomes a negative `maxAge`; a non-numeric one is ignored;
   - an `Expires` that does not parse as an HTTP date leaves `expires` at 0;
   - a line with no `=` before its first `;`, or an empty name, yields no cookie.
   `__http_parseResponse` collects every `set-cookie` line of the head text, in order,
   into `resp.cookies`. `resp.headers` is untouched.

`http::withCookie(resp, cookie)` is `WITH resp { cookies := … }` with the cookie
appended, the same shape as `http::withHeader`. It does not validate; emit does, so a
hand-built `WITH` gets the same check.

**Correctness risk (schedule last, behind fixtures):** the emit path. It is the one
place a handler-controlled string reaches the wire, and it sits beside a guard that
today knows nothing about cookies. Pin it with the two-cookie wire fixture and one
fixture per rejected shape before anything else depends on it.

**HTTP dates.** `expires` needs a Unix-seconds → IMF-fixdate formatter on emit and the
inverse on the client. `datetime::format` and `datetime::parse` exist; confirm in
Phase 0 that they produce and accept `Sun, 06 Nov 1994 08:49:37 GMT` exactly, and use
them. If they cannot, that is a blocker to report, not something to hand-roll around.

Gate is **runtime behavior** (wire bytes; cookies round-tripped through a real
exchange), never byte-identity.

### Rejected alternatives

- **`withCookie(resp, name, value, opts)`** — `opts` had no type; a record replaces it.
- **A constructor function** (`http::cookie(...)`, `http::cookieDefault()`) — the
  language builds records with record constructors; bug-1021 makes that practical.
- **`req.cookies AS Map OF String TO String`** — simpler to read, but a second cookie
  shape; one type on both sides is the point of this plan.
- **`Response.cookies AS Map OF String TO Cookie`** — a response cookie's identity is
  (name, domain, path); a map keyed by name drops a legal second cookie.
- **A reserved sentinel key in `headers`** — visible to `len`/iteration, leaks to the
  client's `Response`, and cannot be told apart from a handler's own header.
- **Comma-joining cookies into one `Set-Cookie`** — illegal; the header does not fold
  (`Expires` contains a comma).
- **Making every header multi-valued** — a far larger change for one header's need.

## Compatibility / Format Impact

- **Breaking:** `Request` and `Response` each gain a field, so a positional
  `http::Request[...]` / `http::Response[...]` with the old field count no longer
  compiles. A by-field constructor is unaffected once bug-1021 lands.
- New exported names: `http::Cookie`, `http::SameSite`, `http::withCookie`.
- Wire: the server may now emit `Set-Cookie` lines. No other header changes.
- A handler that set `Set-Cookie` through `http::withHeader` keeps working as before.

## Phases

### Phase 0 — Prerequisite probes

- [ ] Build `Ck[name := "a"]` against a record with an enum field and confirm the
      omitted field holds the enum's default; record the result.
- [ ] Confirm `http::SameSite.None` is a legal member name (`None` is not a keyword;
      prove it with a compile).
- [ ] Confirm `datetime::format` / `datetime::parse` round-trip an IMF-fixdate; record
      the exact calls in Corrections.
- [ ] Re-run the three census greps in §2 and record the counts in Corrections.

Acceptance: every probe result is written in Corrections with its command.
Commit: —

### Phase 1 — Types and the record changes

- [ ] Register `SameSite` and `Cookie` in `http/mod.rs` with full descriptions.
- [ ] Add `cookies` to `Request` and `Response`; update the 2 + 5 package constructors
      and the 3 fixtures so the tree builds.
- [ ] Add `http::withCookie`.
- [ ] Tests: a fixture that builds cookies with the by-field constructor (minimal, all
      fields, the delete form) and checks every field; a `withCookie` fixture checking
      order and that the original response is unchanged.

Acceptance: the tree builds; `resp.cookies` and `req.cookies` exist and are empty where
nothing set them; the full suite is green.
Commit: —

### Phase 2 — Server read: `Cookie` header → `req.cookies`

- [ ] Add `__http_parseCookieHeader` and call it from `__http_parseRequest`.
- [ ] Tests: one cookie; several pairs; no `Cookie` header (empty map); a value
      containing `=`; a quoted value; a piece with no `=` (skipped, request still
      served); a repeated name (last wins); attribute fields at their defaults.

Acceptance: a handler reads `collections::get(req.cookies, "k").value` and gets the
sent value; a malformed pair never fails the request.
Commit: —

### Phase 3 — Client parse: `Set-Cookie` lines → `resp.cookies`

- [ ] Add `__http_parseSetCookie` and collect the lines in `__http_parseResponse`.
- [ ] Tests, feeding canned response bytes: two `Set-Cookie` lines give two entries in
      wire order; every attribute; mixed-case attribute names; unknown `SameSite` →
      `Unset`; `Max-Age=0` → negative; unparseable `Expires` → 0; an unusable line is
      skipped; `resp.headers` is unchanged from today.

Acceptance: `http::read` of a response with N `Set-Cookie` lines returns N cookies with
the right fields.
Commit: —

### Phase 4 — Server emit (behind the two-cookie fixture)

- [ ] Add `__http_cookieIsValid` and `__http_serializeCookie`.
- [ ] Extend `__http_checkResponse` and the `__http_serializeHead` backstop to cover
      `resp.cookies`; emit one `Set-Cookie` line per entry in `__http_serializeHead`.
- [ ] Tests on wire bytes: **two** cookies → **two** `Set-Cookie` lines in order; an
      attribute fixture (`Path=/; HttpOnly; Secure; SameSite=Lax`); `maxAge := -1` →
      `Max-Age=0`; `expires` → the exact HTTP date; `Unset` → no `SameSite`.
- [ ] Tests on rejection, each answered `500` with no `Set-Cookie` on the wire: an empty
      name; a `\r\n` in the value; a `;` in `path`; `SameSite.None` without `secure`.
- [ ] Round-trip fixture: a local `http::server` handler sets two cookies, `http::read`
      returns both in `resp.cookies` with equal fields; a second request carrying a
      `cookie` header is echoed back from `req.cookies`.

Acceptance: two `withCookie` calls produce two `Set-Cookie` lines; every rejected shape
is a `500`; the round-trip fixture holds.
Commit: —

## Validation Plan

- Tests: fixtures under `tests/rt-behavior/http/`, beside `http_server_loopback`,
  asserting raw emitted head bytes, parsed `req.cookies`, and parsed `resp.cookies`.
- Coverage check: the two-`Set-Cookie` wire fixture and the four rejection fixtures are
  present and green; they are the only tests that exercise the emit guard.
- Runtime proof: the round-trip fixture, plus `curl -i -b 'a=1; b=2'` against a handler
  that echoes `req.cookies` and sets two cookies, confirming two `Set-Cookie` lines.
- Doc sync (`.ai/specifications.md`, `.ai/spec-content.md`, `.ai/man-content.md`):
  - `05_http.md`: `cookies` on §Response Record and on `Request` under Value records;
    the `Cookie` and `SameSite` types; the request-side "only `name` and `value`" rule;
    the `Cookie` header parse rules under Request parsing; `Set-Cookie` emit, validity
    and the `500` under Lifecycle; the client `Set-Cookie` parse under Response
    Parsing; `withCookie` in the combinators table; the positional-constructor break.
  - Man text lives in the registry descriptors, not in files: the `Cookie` / `SameSite`
    / `Request` / `Response` record descriptions in `http/mod.rs`, a new
    `func_with_cookie.rs` descriptor, and the `withHeader` description (stop it reading
    as the way to set a cookie). Verify by rendering each page with `mfb man`.
- Acceptance: full `cargo test` + the acceptance harness.

## Open Decisions

- **`maxAge` zero rule** — recommend 0 = not set, negative = delete, matching the
  owner's rule for `expires`. Alternative: keep the wire meaning (0 = delete) and
  accept that a default `Cookie` would then mean "delete". (§3)
- **`expires` unit** — recommend Unix seconds (`datetime::Instant.seconds`).
  Alternative: milliseconds. (§3)
- **Emit when both `maxAge` and `expires` are set** — recommend emitting both; a
  browser prefers `Max-Age`. (§3)
- **An invalid cookie** — recommend the `500` (the existing rule for a response that
  cannot be serialized safely) over dropping the one cookie, so a handler bug is not
  silent. (§3)

## Corrections

<Filled in during execution — the Phase 0 probe results and census counts go here.>

## Summary

One `Cookie` record, built with the by-field constructor and used on both sides of the
wire, plus the three pieces of wire code around it: the request `Cookie` header into
`req.cookies`, `resp.cookies` out as `Set-Cookie` lines, and `Set-Cookie` lines back
into `resp.cookies` on the client. The risky part is emit, which must extend the
bug-506 control-byte guard to the new list. It starts once bug-1021 lands. Plan-93-F
builds the client jar on top.
