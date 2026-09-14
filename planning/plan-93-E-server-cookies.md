# plan-93-E: Server-side cookies (request parse + `Set-Cookie` emit)

Last updated: 2026-08-09
Effort: medium (1h–2h)
Depends on: nothing for parsing; shares one cookie-attribute helper with plan-93-F
(see Design). Independent of the gzip letters.

The server neither exposes incoming cookies nor helps a handler set one. This
sub-plan parses the request `Cookie` header into a `req.cookies` map and adds a
`Set-Cookie` builder that survives the server's last-wins header map (which cannot,
today, hold two `Set-Cookie` values). Behavioral outcome: a handler reads
`collections::getOr(req.cookies, "session", "")` and calls
`http::withCookie(resp, "session", id, ...)` to emit a well-formed `Set-Cookie`,
and a client sees every cookie the handler set — even when it sets several.

References:

- `src/builtins/http_package.mfb:__http_serializeHead` (lines 1166-1180) — iterates
  `resp.headers` (a `Map`, last-wins) writing each header line; the reason multiple
  `Set-Cookie` values need special handling.
- `src/docs/spec/stdlib/05_http.md:270-298` — the `Request` record (headers/query/
  params maps) that `cookies` joins.
- `src/builtins/http_package.mfb:__http_partHeader` / header parsing — the request
  header parsing to extend for the `Cookie` line.

## Prerequisites

Shared feature gate: plan-93-A's "tree builds & tests green". No dependency on the
gzip letters. If plan-93-F (client cookies) is also planned, this letter lands
first and owns the shared cookie-attribute serializer.

| Must be true | Command | Status |
|---|---|---|
| Tree builds & tests green | `cargo test` | UNMEASURED — run before starting |

## 1. Goal

- `Request` gains `cookies AS Map OF String TO String` — the parsed request
  `Cookie` header (`name=value; name2=value2`), names case-sensitive per RFC 6265,
  last-wins on duplicates, read with the ordinary `collections::*` accessors.
- `http::withCookie(resp, name, value, opts)` returns a copy of `resp` carrying a
  correctly-serialized `Set-Cookie` header supporting the standard attributes:
  `Path`, `Domain`, `Max-Age`, `Expires`, `Secure`, `HttpOnly`, `SameSite`.
- **Multiple `Set-Cookie` values coexist:** two `withCookie` calls emit two
  `Set-Cookie` lines on the wire (the last-wins header map cannot represent this
  today — this sub-plan fixes the emit path for this one header).
- Malformed cookie names/values (control chars, separators) are rejected or
  percent-escaped per a documented rule, never emitted raw.

### Non-goals (explicit constraints)

- **No cookie jar / no statefulness** on the server — it parses what came in and
  emits what the handler asks. Persistence and cross-request tracking are the
  client's concern (plan-93-F).
- **No change to the general last-wins header model** for any header other than
  `Set-Cookie`. Only `Set-Cookie` gains multi-value emit.
- No signature change to `Request` consumers that ignore `cookies` (it's an added
  field; confirm added fields don't break existing positional record literals —
  see Verified properties).

## 2. Current State

### Measured populations

| What | Count | Command |
|---|---|---|
| cookie handling anywhere in the package | 0 | `grep -ci cookie src/builtins/http_package.mfb → 0` |
| `Request` record fields today | 7 | read `05_http.md:270-279` (method,path,rawPath,headers,query,params,parts,body) |

### Verified properties

- **The response header map is last-wins and serialized by iteration.**
  `__http_serializeHead` (`http_package.mfb:1166-1180`) loops `resp.headers` (a
  `Map OF String TO String`) writing one line per entry. A `Map` keyed by header
  name therefore cannot hold two `Set-Cookie` values — the second overwrites the
  first. **VERIFIED** by reading the serialize loop. The fix must carry set-cookies
  outside the collapsing map (see Design).
- **Adding a `Request` field touches every constructor of it.** `Request[...]`
  positional literals in `__http_handleRequest` and its helpers must all gain the
  new field. **UNVERIFIED count** — census `Request[` before editing:
  `grep -n 'Request\[' src/builtins/http_package.mfb`. Make this the first task.

## 3. Design Overview

Two independent pieces:

1. **Request `cookies`** — in request parsing, split the (last-wins) `cookie`
   header on `;`, each into `name=value`, trim, into a new `cookies` map; add
   `cookies` to the `Request` record and every `Request[...]` constructor.
2. **`Set-Cookie` emit that survives last-wins** — the collapsing `Map` cannot
   hold multiple `Set-Cookie`. Chosen approach (see Rejected alternatives): carry
   an **ordered list of already-serialized `Set-Cookie` lines** and emit them in
   `__http_serializeHead` in addition to the header map. `withCookie` appends a
   serialized line built by `__http_serializeCookie(name, value, opts)` — the
   **shared attribute serializer** that plan-93-F reuses for the client's outgoing
   `Cookie`/jar formatting. To avoid a `Response` shape change rippling everywhere,
   store the set-cookie list *inside* the headers map under a reserved sentinel key
   that `__http_serializeHead` expands into N `Set-Cookie:` lines (documented,
   internal) — or add a `setCookies AS List OF String` field to `Response` if a
   census shows few constructors. Decide by the `Response[` census (Open Decisions).

**Correctness/design risk (schedule the emit piece last):** representing multiple
`Set-Cookie` under a last-wins map is the one non-trivial call. Prove it with a
fixture that sets two cookies and asserts two `Set-Cookie` lines on the wire.

Gate is **runtime behavior** (wire bytes / round-tripped cookies), never
byte-identity.

### Rejected alternatives

- **Comma-join multiple cookies into one `Set-Cookie`** — illegal; `Set-Cookie`
  does not fold (its value contains commas in `Expires`). Rejected.
- **Change the whole header model to multi-valued** — large blast radius across
  client and server parsing/emit for a single-header need. Rejected in favor of the
  targeted `Set-Cookie` list.

## Compatibility / Format Impact

- `Request` gains a `cookies` field (every `Request[...]` constructor updated).
- `Response` emit gains multi-`Set-Cookie` support; all other headers keep last-wins.
- New public helper `http::withCookie`. No existing signature changes.

## Phases

### Phase 1 — Parse request `Cookie` → `req.cookies`

- [ ] Census `Request[` constructors (`grep -n 'Request\[' src/builtins/http_package.mfb`)
      and record the count in Corrections.
- [ ] Add `cookies AS Map OF String TO String` to the `Request` record and every
      constructor; parse the `cookie` header into it during request parsing.
- [ ] Tests: fixtures for a single cookie, multiple `name=value` pairs, an absent
      `Cookie` header (empty map), and a malformed pair (documented handling).

Acceptance: a handler reads `collections::getOr(req.cookies, "k", "")` and gets the
sent value; absent header → empty map.
Commit: —

### Phase 2 — `withCookie` + multi-`Set-Cookie` emit (behind the two-cookie fixture)

- [ ] Add `__http_serializeCookie(name, value, opts)` producing one RFC 6265
      `Set-Cookie` value with `Path`/`Domain`/`Max-Age`/`Expires`/`Secure`/
      `HttpOnly`/`SameSite`; reject/escape invalid name/value chars.
- [ ] Add `http::withCookie(resp, name, value, opts) AS Response` appending a
      serialized line via the chosen multi-value carrier.
- [ ] Extend `__http_serializeHead` to emit every carried `Set-Cookie` line (in
      order) alongside the header map.
- [ ] Tests: a fixture setting **two** cookies and asserting **two** `Set-Cookie`
      lines on the wire; an attribute fixture asserting `HttpOnly; Secure;
      SameSite=Lax; Path=/`; an invalid-name fixture.

Acceptance: two `withCookie` calls produce two `Set-Cookie` lines; attributes
serialize per RFC 6265; invalid input is rejected/escaped, never emitted raw.
Commit: —

## Validation Plan

- Tests: fixtures under the http server test home asserting raw emitted headers and
  parsed `req.cookies`.
- Coverage check: green run with the two-`Set-Cookie` fixture present (it exercises
  the multi-value emit path).
- Runtime proof: `http::server` handler that echoes `req.cookies` and sets two
  cookies; curl `-b`/`-c` to confirm round-trip.
- Doc sync: extend `05_http.md` Server §Request record (add `cookies`) and
  §Constructors (add `withCookie`); add a man page
  `src/docs/man/builtins/http/withCookie.md` per `.ai/man_template.md`; update the
  http `types` man page for the new `Request` field.
- Acceptance: full `cargo test` + acceptance harness.

## Open Decisions

- **Multi-`Set-Cookie` carrier** — recommend a reserved sentinel entry in the
  headers map expanded on emit **iff** the `Response[` census shows many
  constructors (avoids editing them all); otherwise add an explicit
  `setCookies AS List OF String` field to `Response`. Decide from the census in
  Phase 2's first task. (§3)
- **Invalid cookie value handling** — recommend reject (error) over silent escape,
  so a handler bug surfaces. (§1)

## Corrections

<Filled in during execution — record the `Request[` and `Response[` census counts
here.>

## Summary

Two loosely-coupled pieces: trivial request-cookie parsing, and the one genuinely
tricky bit — emitting multiple `Set-Cookie` through a last-wins header map, pinned
by a two-cookie fixture. Owns the shared cookie-attribute serializer that
plan-93-F consumes.
