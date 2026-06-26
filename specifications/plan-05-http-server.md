# plan-05 — HTTP Server (routing, single-threaded)

Last updated: 2026-06-25

This document is the **normative definition and implementation plan** for the
built-in `http` **server** — the server-side sibling of the client defined in
`specifications/plan-03-http.md`. It is the same `http` package, the same source
idiom (`http.rs` shim + `http_package.mfb`), and it **layers on the existing native
`net` package** for transport (`net::listenTcp`, `net::accept`, socket I/O,
`net::close`). No new Rust intrinsics or syscalls are introduced.

> **Cross-references.** This plan continues the section lettering of the client
> plan. Sections **A–E** (e.g. §B.4, §C Phase 3, §E.3) refer to
> `plan-03-http.md`; sections **F.*** and phases **S0–S7** are defined in this
> document.

The two halves of `http` are **implementation-independent** — the server may land
before or after the client. They share only the package namespace and the `net`
transport. Names do not collide: the client contributes `read`/`write`/`header`/
`headerOr` and the `Result` record; the server contributes `server`/`handleRequest`/
`serverSSL`/`route`/the response constructors and the
`Request`/`Response`/`Route`/`RequestPart` records.

## F.1 Design in one paragraph

A server is a **single-threaded, blocking, user-driven accept loop**. There is no
event loop, no async, no threads, no per-connection workers. The user constructs a
`Server` resource, builds an **ordered** `List OF Route` mapping path patterns to
handler functions, and calls `http::handleRequest(server, routes)` in their own
`DO/LOOP`. The user obtains a listener (`http::server` returns a `net::Listener`
directly — no separate server resource, §F.2.5; `http::serverSSL` returns a
`tls::Listener` for HTTPS, §F.5.6). Each call accepts one
connection, parses the request into a plain
`Request` value, matches its **path** (query string stripped first) against the
routes **in list order — first match wins**, invokes the matched handler
(`FUNC(Request) AS Response`), writes the returned `Response`, and closes the
connection. A handler that errors becomes a `500`; no matching route becomes a
`404`; the loop never dies on a bad request.

This shape is possible because functions are **first-class** in MFBASIC: a plain
(non-`ISOLATED`) `FUNC` can be stored in a `List`/`Map` and called indirectly
through a variable (`mfbasic.md` function types; `tests/func_collection_*`). The
handler type `FUNC(http::Request) AS http::Response` is a normal function-typed
value.

## F.2 Types

All record types are flat, **copyable value records** (no resource fields). They
are declared in `http_package.mfb` and registered through `http::is_builtin_type`.
The server introduces **no new resource type** — `http::server` returns the
existing `net::Listener` (§F.2.5).

### F.2.1 `Request`

```basic
EXPORT TYPE Request
  method  AS String                          ' "GET", "POST", ... (uppercased)
  path    AS String                          ' "/test/42" — query already stripped, percent-decoded
  rawPath AS String                          ' the request-target as received (pre-decode, pre-strip)
  headers AS Map OF String TO String         ' field names lowercased; last-wins on duplicates
  query   AS Map OF String TO String         ' from "?a=1&b=2"; percent-decoded; last-wins
  params  AS Map OF String TO String         ' from ":testId" / ":bar?" / "*" route captures
  parts   AS Map OF String TO RequestPart    ' multipart/form-data parts, keyed by part name
  body    AS List OF Byte                     ' raw request body (empty for body-less methods)
END TYPE
```

- **`headers`** keys are normalized to **lowercase** (HTTP field names are
  case-insensitive), so `req.headers["host"]` always finds `Host`. Duplicate
  fields collapse **last-wins**; the multi-value `List` representation is a non-goal
  (§F.8).
- **`query`** is parsed from the part after the first `?`, split on `&` then `=`,
  with **percent-decoding** (`%20`, `+` → space) applied to keys and values.
  Last-wins on repeated keys.
- **`params`** is filled by route matching (§F.3): named segments (`:testId`),
  optional segments (`:bar?` — present only when matched), and the wildcard
  remainder under the key `"*"`.
- **`parts`** is populated only for `Content-Type: multipart/form-data` bodies
  (§F.4.3); empty otherwise. `body` always holds the raw bytes regardless.

### F.2.2 `RequestPart`

```basic
EXPORT TYPE RequestPart
  filename    AS String          ' from the part's Content-Disposition; "" if a plain field
  contentType AS String          ' the part's Content-Type; "" if absent
  body        AS List OF Byte    ' the part's raw bytes
END TYPE
```

One entry per multipart part, keyed in `Request.parts` by the part's `name` (from
its `Content-Disposition`). A file upload has a non-empty `filename`; a plain form
field has `filename = ""` and its text in `body`.

### F.2.3 `Response`

```basic
EXPORT TYPE Response
  status  AS Integer                     ' status code, e.g. 200, 404, 500
  reason  AS String                      ' reason phrase; "" → derived from status
  headers AS Map OF String TO String     ' response headers (names sent as written)
  body    AS List OF Byte                ' response body bytes
END TYPE
```

`Response` is a plain value record. It is **immutable in place** — MFBASIC has no
field-target assignment for plain records (assignment targets are identifiers
only). Handlers build and edit a response with **`WITH`** (functional update
producing a copy) over a `MUT` binding, which reads like mutation:

```basic
MUT resp AS http::Response = http::responseDefault()   ' 200 + "OK"
resp = WITH resp { status := 404 }
resp = WITH resp { body := http::bytes("nope") }
RETURN resp
```

The body is **`List OF Byte`** so binary responses (images, fonts) work; the
`http::ok`/`http::status` constructors and `http::bytes` provide the String-in
convenience (§F.5.3). `reason` may be left `""`; `handleRequest` derives the
standard phrase from `status` when emitting the status line.

### F.2.4 `Route`

```basic
EXPORT TYPE Route
  pattern AS String                              ' "/test/:testId", "/foo/:bar?", "/static/*"
  handler AS FUNC(Request) AS Response
END TYPE
```

`http::route(pattern, handler)` (§F.5.2) is a constructor convenience returning
`Route[pattern, handler]`; the literal form `Route["/", home]` works equally. A
program holds its routes in a `List OF http::Route`, **ordered** — matching is
first-in-list-wins (§F.3.3).

### F.2.5 No server resource — `http::server` returns `net::Listener`

The server introduces **no `http::Server` type**. `http::server(port, …)` returns
the existing `net::Listener` directly, and `http::handleRequest` accepts a
`net::Listener`. There is no server-level mutable state to carry — configuration is
passed per call (the `routes` list), the size cap is a fixed constant, and there is
no connection pool or keep-alive — so a wrapper resource would add a type, a close
function, and the resource-containing-resource question for zero benefit. The
listener is owned by the caller's `RES` binding and closed by lexical drop at scope
exit, exactly as `net::listenTcp` already provides.

Callers therefore `IMPORT net` to name the type (`RES s AS net::Listener`), the
same way the client's callers `IMPORT net` to name `net::Url` (§B.1). If a later
feature ever needs real server state (e.g. a keep-alive pool), promoting to a
dedicated resource is a backward-compatible change.

The **secure** variant `http::serverSSL` (§F.5.6) returns a `tls::Listener`
instead. That one *is* a state-bearing resource — it owns the bound socket **and**
the loaded server credentials (certificate + private key) — so unlike the plaintext
case it justifies a dedicated resource, provided by the `tls` package (not `http`).

## F.3 Routing & path matching

### F.3.1 Match on the path only

`handleRequest` splits the request target at the **first `?`**. Everything before
is the path (percent-decoded into `Request.path`); everything after is the query
(parsed into `Request.query`, §F.4.2). **Route patterns match against the path
only** — a pattern never sees `?...`. A request to `/test/42?debug=1` matches the
pattern `/test/:testId`, binds `testId = "42"`, and exposes `debug = "1"` via
`req.query`.

### F.3.2 Pattern syntax

Patterns are matched **segment by segment** (split on `/`). A segment is one of:

| Segment | Meaning | Binds |
|---------|---------|-------|
| literal (`foo`) | must equal the path segment exactly | — |
| `:name` | captures exactly one path segment | `params["name"]` |
| `:name?` | **trailing** optional segment — matches with or without it | `params["name"]` when present |
| `*` | **trailing** catch-all — captures the entire remaining path | `params["*"]` |

Rules:

1. **`:name?` and `*` are legal only as the final segment(s).** A mid-pattern
   optional or wildcard is rejected at registration (`ErrInvalidArgument`) — it
   makes matching ambiguous and is a non-goal. Multiple trailing optionals are
   allowed (`/foo/:a?/:b?`).
2. `:name` matches exactly one non-empty segment; `/test/:id` does **not** match
   `/test` or `/test/42/extra`.
3. `*` matches the rest of the path including embedded `/`; `/static/*` matches
   `/static/css/app.css` and binds `params["*"] = "css/app.css"`.
4. A trailing slash is **normalized away** before matching (`/foo/` ≡ `/foo`),
   except the root path `/` itself.

### F.3.3 Ordered, first-match-wins

Routes are tried **in the order they appear in the list**. The first pattern that
matches handles the request. Precedence is therefore explicit and entirely the
caller's choice — there is no "most specific wins" heuristic. To let a literal beat
a pattern, list it first:

```basic
collections::listOf(
  http::route("/foo/new",   newFoo),     ' literal — listed first, so it wins
  http::route("/foo/:bar?", showFoo)     ' optional-param fallback
)
```

The cost of this rule: a broad pattern placed before a specific literal silently
shadows the literal. This is documented behavior, not a defect — it is how the
order-based model is meant to work.

### F.3.4 No match → 404

If no route matches the path, `handleRequest` emits a built-in **`404`** response
(`text/plain`, body `"Not Found"`). The body/format of the default 404 is fixed in
v1; a user-supplied fallback route (`http::route("/*", myNotFound)` listed last) is
the way to customize it.

## F.4 Request parsing

`handleRequest` reads one full HTTP/1.1 request from the accepted socket and parses
it with pure string/byte code, mirroring the response parser in §B.4.

### F.4.1 Request line & headers

- **Request line** `METHOD <request-target> HTTP/<v>` → `method` (uppercased),
  `rawPath`/`path`/`query` (§F.3.1/§F.4.2). A malformed first line fails
  `ErrInvalidFormat` and is answered with `400`.
- **Headers** are read until the first empty line; each `name: value` becomes a
  `headers` entry with the **name lowercased** and the value OWS-trimmed.
  Duplicates collapse last-wins.

### F.4.2 Path & query decoding

- The request-target is split at the first `?`. The path portion is
  **percent-decoded** into `Request.path`; the raw target is preserved in
  `rawPath`.
- The query portion is split on `&`, each pair on the first `=`, with keys and
  values **percent-decoded** (`%XX` and `+` → space). Result → `Request.query`,
  last-wins on repeats.
- Percent-decoding and query splitting are **new helpers** (§E.3 lists
  query-string parsing as a future `net`/`url` addition). Per the layering
  philosophy in Part A ("the URL/address vocabulary lives with the transport"),
  they are added to **`net`** — `net::percentDecode(s) AS String` and
  `net::parseQuery(s) AS Map OF String TO String` — and consumed by `http`.

### F.4.3 Body & multipart parts

- **Body framing** follows the request headers, mirroring the client's response
  rules (§B.4): `Content-Length: N` reads exactly `N` bytes;
  `Transfer-Encoding: chunked` is de-chunked; body-less methods (`GET`, `HEAD`,
  `DELETE`, no `Content-Length`) yield an empty body. The raw bytes always land in
  `Request.body`.
- **Multipart** parsing runs only when `Content-Type` is `multipart/form-data` with
  a `boundary`. The body is split on the boundary delimiter; each part's own
  headers (`Content-Disposition`, `Content-Type`) are parsed to fill a
  `RequestPart` (`name` → map key, `filename`, `contentType`, `body`). Malformed
  multipart framing fails `ErrInvalidFormat` → `400`. This boundary parser is the
  single largest piece of genuinely new logic the server adds.
- **Size cap.** The request is bounded by the same internal cap the client uses
  (default 64 MiB, §B.4); exceeding it fails `ErrOverflow` → `413`.

## F.5 The server API

### F.5.1 Lifecycle

| Function | Signature | Behavior |
|----------|-----------|----------|
| `http::server` | `FUNC server(port AS Integer, host AS String = "0.0.0.0", backlog AS Integer = 128) AS net::Listener` | Binds and listens (plaintext) via `net::listenTcp`; returns the listener. Blocking; closed by lexical drop. |
| `http::serverSSL` | `FUNC serverSSL(port AS Integer, certPath AS String, keyPath AS String, host AS String = "0.0.0.0", backlog AS Integer = 128) AS tls::Listener` | Binds a **TLS** listener via `tls::listen`, loading the PEM server certificate (`certPath`) and private key (`keyPath`). Returns a `tls::Listener`. Blocking; closed by lexical drop. Works on Linux and macOS (§F.5.6). |
| `http::handleRequest` | `FUNC handleRequest(listener AS net::Listener, routes AS List OF Route) AS Nothing`<br>`FUNC handleRequest(listener AS tls::Listener, routes AS List OF Route) AS Nothing` | **Overloaded by listener type.** Accepts **one** connection (`net::accept`, or `tls::accept` performing the server-side handshake), parses the request, matches the path against `routes` in order, invokes the handler, writes the response, closes the connection. Both overloads share one pure parse/match/dispatch/emit core; only the transport (plain `Socket` vs `TlsSocket`) differs. |

`handleRequest` is **crash-proof**: the handler invocation is wrapped in a `TRAP`.
A handler that fails (any propagated error) is answered with a built-in **`500`**
(`text/plain`, `"Internal Server Error"`) and the loop continues. Connection-level
I/O errors (peer reset mid-read/-write) are caught, the connection is dropped, and
`handleRequest` returns normally — one bad client never tears down the server.

```basic
RES s AS net::Listener = http::server(8080)
DO
  http::handleRequest(s, routes)
LOOP
```

### F.5.2 Routing constructor

| Function | Signature | Behavior |
|----------|-----------|----------|
| `http::route` | `FUNC route(pattern AS String, handler AS FUNC(Request) AS Response) AS Route` | Builds a `Route`; validates the pattern (trailing-only `:name?`/`*`, §F.3.2) and fails `ErrInvalidArgument` on a malformed pattern. |

### F.5.3 Response constructors & combinators

| Function | Signature | Behavior |
|----------|-----------|----------|
| `http::responseDefault` | `FUNC responseDefault() AS Response` | A `200` with reason `"OK"`, body `"OK"`, no extra headers. The convenient starting point for `WITH` edits (not required — `ok`/`status` build directly). |
| `http::ok` | `FUNC ok(body AS String) AS Response` | `200` with `body` (UTF-8) and `Content-Type: text/plain; charset=utf-8`. |
| `http::status` | `FUNC status(code AS Integer, body AS String) AS Response` | Arbitrary status with a text body. |
| `http::json` | `FUNC json(body AS String) AS Response` | `200` with `Content-Type: application/json`. |
| `http::withHeader` | `FUNC withHeader(resp AS Response, name AS String, value AS String) AS Response` | Returns a copy with one header set (sugar over `WITH resp { headers := ... }`). |
| `http::bytes` | `FUNC bytes(text AS String) AS List OF Byte` | UTF-8-encode a `String` into the `body` byte type. |

`Content-Length` and the status-line reason phrase are always supplied by
`handleRequest` on emit; a handler-set `Content-Length` is ignored to preserve
framing (same reservation policy as the client, §B.3).

### F.5.4 Reading the request — no accessors, use `collections::`

There are **no** `http`-specific request accessors. `Request`'s fields are public
maps, so handlers read them with the standard `collections::*` functions:

- path param: `collections::get(req.params, "testId")`
- optional-param presence: `collections::hasKey(req.params, "bar")`
- query value: `collections::get(req.query, "q")`
- header (keys are lowercased on parse, §F.2.1):
  `collections::get(req.headers, "content-type")`

This keeps the surface minimal and avoids duplicating `collections` behavior. (The
client's `http::header(Result, …)` therefore stays the only `header` function — no
`Request` overload is added.)

### F.5.5 Static file helpers

| Function | Signature | Behavior |
|----------|-----------|----------|
| `http::respondFile` | `FUNC respondFile(file AS RES File, contentType AS String = "") AS Response` | Reads the open file fully into a `200` response body; closes the file (the `RES` is consumed). When `contentType = ""`, defaults to `application/octet-stream`. The low-level primitive. |
| `http::respondPath` | `FUNC respondPath(req AS Request, root AS String) AS Response` | Resolves `req.params["*"]` (or `req.path`) under `root`, **safely**, opens it, infers `Content-Type` from the extension, and serves it; `404` when the file is absent. The function most handlers call. |

**Path-traversal safety is built in.** `respondPath` canonicalizes the requested
path and confines it to `root` *before* touching `fs::*`; any path that escapes
`root` (`../`, absolute, symlink-out) yields a `403`, never a file read. This is the
one security-critical behavior in the server and is not left to the caller.

`respondFile` takes ownership via `RES File`, so the file is closed by lexical drop
even on an error path. Both helpers **buffer the whole file** into `Response.body` —
acceptable for a single-threaded dev/static server; streaming and large-file
chunking are non-goals (§F.8). Note the single-threaded cost: serving a large file
blocks the entire server until it finishes.

### F.5.6 Secure server — `http::serverSSL`

`http::serverSSL` is the TLS counterpart of `http::server`. It binds a listening
socket and loads a **PEM server certificate** (`certPath`) and **private key**
(`keyPath`), returning a `tls::Listener` (§F.2.5). The accept loop is otherwise
identical — `http::handleRequest` is overloaded on the listener type (§F.5.1), so
the loop body and the route list are unchanged between plaintext and TLS:

```basic
RES s AS tls::Listener = http::serverSSL(8443, "cert.pem", "key.pem")
DO
  http::handleRequest(s, routes)   ' same routes, same handlers
LOOP
```

**New `tls` primitives required.** `tls` is client-connect-only today
(`tls::connect`). `serverSSL` depends on server-side additions to the `tls` package
(Phase S0): a `tls::Listener` resource owning the bound socket + loaded credentials,
`tls::listen(host, port, certPath, keyPath, backlog)`, and
`tls::accept(listener) AS TlsSocket` (TCP accept + **server-side** handshake). The
per-connection `tls::read`/`write`/`close` are the existing client functions, reused
unchanged.

**Platform.** `serverSSL` is supported on **both Linux and macOS** — no platform
gate. Phase S0 provides the `tls` server backend on each target (OpenSSL via
`dlopen` on Linux; Network.framework on macOS).

**Transport split.** Because `Socket` and `TlsSocket` are distinct resources that
cannot share one variable (§B.5), the two `handleRequest` overloads have separate
transport bodies (`net::accept`/… vs `tls::accept`/…), both feeding the shared pure
parse/match/dispatch/emit core — exactly the structure the client uses for its
plaintext vs TLS exchange branches.

## F.6 Implementation additions

Parallels the client's source-package plan (Part C). `net` gains two small pure
helpers; `http`'s shim and `.mfb` gain the server surface.

**Reuse inventory (beyond Part C's):**

| Server need | Reused as | Source |
|-------------|-----------|--------|
| Listen / accept / socket read / write / close | `net::listenTcp`, `net::accept`, `net::read`/`readText`, `net::write`/`writeText`, `net::close` | `src/builtins/net.rs` |
| TLS listen / accept / read / write / close | `tls::listen`/`tls::accept` (new — Phase S0), `tls::read`/`write`/`close` (existing) | `src/builtins/tls.rs` |
| Request line / header parsing, chunk decode | the client's response-parsing helpers, adapted (mirror image) | `http_package.mfb` (Part C) |
| First-class handler values in a `List` | function types + indirect call | `mfbasic.md`; `tests/func_collection_*` |
| `WITH` response edits | functional update | `tests/types-with-update-owned` |
| File open / read / close for static serving | `fs::openFile`/`read`/`close` (`RES File`) | `fs` |

**Genuinely new logic:** the path-pattern matcher (`:param`/`:param?`/`*`, segment
walk), percent-decoding + query parsing (`net`), the **multipart/form-data boundary
parser**, the path-traversal-safe static resolver, and the accept→parse→dispatch→
respond lifecycle with its `404`/`500`/crash-proof error routing.

### Phase S0 — `tls` server-side primitives (prerequisite for `serverSSL`)

`tls` is **client-only** today. `http::serverSSL` needs new server-side support in
the `tls` package (landing in the `tls` shim/codegen, not `http`):

- `tls::Listener` — a new resource owning the bound socket + loaded credentials
  (server certificate + private key).
- `FUNC listen(host AS String, port AS Integer, certPath AS String, keyPath AS String, backlog AS Integer = 128) AS Listener` — bind + load PEM credentials.
- `FUNC accept(listener AS Listener, timeoutMs AS Integer = 0) AS TlsSocket` — TCP
  accept + **server-side** TLS handshake, yielding the existing `tls::TlsSocket`
  (whose `read`/`write`/`close` are reused as-is).

These are **server-side** additions to the two `tls` backends that already exist
(`tls.rs:4-8` — OpenSSL via `dlopen` on Linux, Network.framework on macOS), so
`serverSSL` carries no platform gate. Hard prerequisite: without these primitives,
`http::serverSSL` cannot be implemented.

### Phase S1 — `net` decode helpers

In `src/builtins/net_package.mfb` (the source companion from Phase 1 — create it
there if not yet present):

- `__net_percentDecode(s AS String) AS String` + public `FUNC percentDecode`.
- `__net_parseQuery(s AS String) AS Map OF String TO String` + public
  `FUNC parseQuery`.

Register both in `src/builtins/net.rs` exactly as Phase 1 registers `toUrl`/
`toAddress` (consts, `is_net_call`, `call_param_names`, `call_return_type_name`,
`resolve_call`, `arity`, `implementation_name`).

### Phase S2 — `http` shim additions: `src/builtins/http.rs`

Add to the shim from Phase 2:

- Consts: `SERVER`, `SERVER_SSL`, `HANDLE_REQUEST`, `ROUTE`, `RESPONSE_DEFAULT`,
  `OK`, `STATUS`, `JSON`, `WITH_HEADER`, `BYTES`, `RESPOND_FILE`, `RESPOND_PATH`;
  type names `REQUEST_TYPE`, `REQUEST_PART_TYPE`, `RESPONSE_TYPE`, `ROUTE_TYPE`; the
  `__http_*` targets. (`SERVER`/`SERVER_SSL` are *call* names, not types — `server`
  returns `net::Listener`, `serverSSL` returns `tls::Listener`. No request-accessor
  calls — handlers use `collections::*` on the `Request` maps, §F.5.4.)
- `is_builtin_type` → add `Request | RequestPart | Response | Route` (no server
  type; listeners are the existing `net`/`tls` resources).
- `is_http_call` → add the new names.
- `call_param_names` / `call_return_type_name` / `resolve_call` /
  `expected_arguments` / `arity` for each (function-typed `handler` argument typed
  as `FUNC(Request) AS Response`).
- `default_argument_padding`: `server` (host, backlog), `respondFile`
  (contentType), `route` (none).
- `call_return_type_name`/`resolve_call`: `server` → `net::Listener`; `serverSSL`
  → `tls::Listener` (both existing transport resources — no new `http` resource).
  `handleRequest` is **overloaded by listener type**, resolving both
  `[net::Listener, List OF Route]` and `[tls::Listener, List OF Route]` → `Nothing`.
  `consumes_argument` for `respondFile`'s `RES File`.

### Phase S3 — `http_package.mfb` server implementation

Add to `http_package.mfb` (header already imports `net`, `strings`, `collections`,
`errorCode`; add `IMPORT fs`):

- Export `Request`, `RequestPart`, `Response`, `Route`.
- **Pure (shared with, or adapted from, the client):** request-line/header parser,
  chunk decoder, `__http_matchRoute(routes, path)` (the segment walker producing the
  bound `params`), the multipart parser, percent-decode/query via `net::*`.
- **Lifecycle:** `__http_server` (wraps `net::listenTcp`), `__http_serverSSL`
  (wraps `tls::listen`), and the two `handleRequest` overloads — a TCP body via
  `net::accept`/`read`/`write`/`close` and a TLS body via
  `tls::accept`/`read`/`write`/`close`. Both run accept → read-to-frame → parse →
  match → `TRAP`-wrapped handler call → emit → close over the shared pure core; the
  two transport bodies cannot share one socket variable (`Socket` vs `TlsSocket`,
  §B.5).
- **Constructors/static:** the §F.5.3 response constructors and §F.5.5 static
  helpers (reading the request needs no `http` code — handlers call
  `collections::*` on the `Request` maps, §F.5.4).

Honor the source-package constraints from §C Phase 3 (reserved words not
identifiers; ≤ 8 params per function — the handler signature is a single `Request`;
no field-target assignment, build with constructors / `WITH`, thread parser cursor
state through a small node record; cross-file visibility needs `EXPORT`; escape
`\r\n` in literals).

### Phase S4 — Wire into the compiler

Same touch-points as §C Phase 4 (`src/builtins/mod.rs`, `src/resolver.rs`,
`src/typecheck.rs`, `src/ir.rs`); this only **adds the new calls/types** to the
existing dispatch. Verify the function-typed `handler` argument typechecks and
lowers (the first builtin to take a `FUNC(...)` argument — confirm against the
`collections::sortBy`/`reduce` precedent, which already accept function values).

### Phase S5 — Man pages

`src/man/builtins/http/`: add `server.txt`, `handleRequest.txt`, `route.txt`,
`responseDefault.txt`, `ok.txt`, `status.txt`, `json.txt`, `withHeader.txt`,
`bytes.txt`, `respondFile.txt`, `respondPath.txt`, and a routing/types note in
`package.txt`. (No request-accessor pages — handlers use `collections::*`.) Wire `build.rs` and
`src/man/mod.rs` as §C Phase 5 describes.

### Phase S6 — User documentation

`specifications/standard_package.md`: extend the "Built-in HTTP Package" section
with a **Server** subsection — the `Request`/`RequestPart`/`Response`/`Route`
blocks, the §F.5 API tables, the routing rules (§F.3), and the
request-parsing/lifecycle summaries. Add `net::percentDecode`/`net::parseQuery` to
§11. `error_codes.md`: note `http`(server) on `ErrInvalidFormat`,
`ErrInvalidArgument`, `ErrOverflow`, and the `7-707-*` transport rows.

### Phase S7 — Tests (golden)

Mirror `tests/func_http_*`. Split by network dependence.

**Offline (deterministic, the bulk):**

- `func_http_match_*` — drive `__http_matchRoute` over patterns: literal, `:param`,
  `:bar?` present/absent, `/static/*` remainder, trailing-slash normalization,
  order/first-wins, mid-pattern `*`/`?` rejected (`ErrInvalidArgument`).
- `func_http_parserequest_*` — fixed raw requests: `GET` with query, `POST` with
  `Content-Length`, chunked, malformed request line (`400`), oversize (`413`).
- `func_http_multipart_*` — a canned `multipart/form-data` body → `parts` map
  (field + file upload, filename/contentType/body); malformed boundary.
- `func_net_percentdecode_valid` / `func_net_parsequery_valid`.
- `func_http_response_*` — `responseDefault`/`ok`/`status`/`json`/`withHeader`/
  `bytes`; status-line/reason derivation; reserved `Content-Length` ignored.
- `func_http_respondpath_*` — extension→Content-Type; **traversal attempts (`../`,
  absolute) → `403`**; missing file → `404`.

**Networked (gated; on-device, not the default sweep):**

- `http_server_loopback` — a `http::server` on `127.0.0.1` with a small route list;
  drive it with the `http::read` client for `200`/`404`/`500`/path-param/query/
  static round-trips; hermetic, no external host.

## F.7 Worked example

```basic
IMPORT http
IMPORT net
IMPORT collections

' All handlers share FUNC(http::Request) AS http::Response.

FUNC home(req AS http::Request) AS http::Response
  RETURN http::ok("welcome")
END FUNC

FUNC showTest(req AS http::Request) AS http::Response
  RETURN http::ok("test id = " & collections::get(req.params, "testId"))
END FUNC

FUNC showFoo(req AS http::Request) AS http::Response
  ' /foo/:bar?  — bar may be absent
  IF collections::hasKey(req.params, "bar") THEN
    RETURN http::ok("foo bar = " & collections::get(req.params, "bar"))
  END IF
  RETURN http::ok("foo, no bar")
END FUNC

FUNC teapot(req AS http::Request) AS http::Response
  MUT resp AS http::Response = http::responseDefault()   ' 200 / "OK"
  resp = WITH resp { status := 418 }
  resp = WITH resp { body := http::bytes("I'm a teapot") }
  RETURN resp
END FUNC

FUNC serveStatic(req AS http::Request) AS http::Response
  RETURN http::respondPath(req, "./public")               ' traversal-safe
END FUNC

FUNC main() AS Integer
  LET routes AS List OF http::Route = collections::listOf(
    http::route("/",             home),
    http::route("/test/:testId", showTest),
    http::route("/foo/:bar?",    showFoo),
    http::route("/teapot",       teapot),
    http::route("/static/*",     serveStatic)
  )

  RES s AS net::Listener = http::server(8080)
  DO
    http::handleRequest(s, routes)   ' accept 1, strip query, match path,
  LOOP                               ' call handler (err->500), no match->404
END FUNC
```

A request to `GET /test/42?debug=1` matches `/test/:testId` (the `?debug=1` is
stripped before matching), binds `testId = "42"`, makes `debug = "1"` available via
`collections::get(req.query, "debug")`, and returns `200 test id = 42`.

## F.8 Server non-goals for v1

- **Concurrency.** Single-threaded, one request at a time, in the user's loop. No
  threads, no async, no per-connection workers. (A future thread-per-connection
  mode is a clean wrapper once the value-based request/response core is proven.)
- **mTLS / client certificates; SNI multi-cert selection.** `http::serverSSL`
  (§F.5.6) presents a single server certificate/key and works on Linux and macOS;
  mutual-TLS (verifying client certs) and per-hostname certificate selection are out
  of scope.
- **Keep-alive / pipelining.** Each `handleRequest` is one accept-parse-respond-
  close. `Connection: close` semantics.
- **Streaming bodies.** Request bodies are buffered to the size cap; responses
  (incl. static files) are buffered fully into `Response.body`. No chunked
  *response* generation, no large-file streaming.
- **Multi-value headers/query** (last-wins; no `Map OF String TO List`).
- **Mid-pattern `*`/`:param?`** (trailing-only).
- **Compression, HTTP/2+, WebSockets, range requests, ETag/conditional GET.**
- **Configurable size cap** (fixed 64 MiB).
- **Sessions, cookie-parsing helpers, templating, generic middleware chains** —
  each is a sibling package or a later addition, not a server concern. (Middleware,
  without closures, is better expressed as explicit handler composition or built-in
  `handleRequest` policy than as a generic chain.)

These keep the server v1 a small, blocking, batteries-light router honoring the
requested shape: an ordered `List OF Route` of path-pattern → `FUNC(Request) AS
Response`, matched in list order on the path alone, driven by a user-owned
`DO/LOOP` over `http::handleRequest(server, routes)`.
