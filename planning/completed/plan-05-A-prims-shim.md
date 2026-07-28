# plan-05-A: HTTP server — TLS prims, net/http shim, compiler wiring

Last updated: 2026-06-25
Effort: medium

Part **A** of plan-05 (HTTP Server). Lands the pieces the server needs before its `.mfb` body: the
`tls` server-side primitives, the `net` decode helpers, the `http` shim additions, and the compiler
wiring. Shared design (types, routing, request parsing, API) lives in the overview:
[plan-05-http-server.md](plan-05-http-server.md).

- **Depends on:** the existing `net`/`tls`/`http` client shims.
- **Blocks:** plan-05-B (the `.mfb` server body uses all of these), plan-05-C (docs + tests).
- **Spec/design:** overview §F.2 (types), §F.5 (server API), §F.6 (reuse inventory).

## Phases

### Phase A1 — `tls` server-side primitives (prerequisite for `serverSSL`)

`tls` is client-only today; `http::serverSSL` needs new server-side support in the `tls` package.

- [ ] `tls::Listener` — a new resource owning the bound socket + loaded credentials (server certificate + private key).
- [ ] `FUNC listen(host AS String, port AS Integer, certPath AS String, keyPath AS String, backlog AS Integer = 128) AS Listener` — bind + load PEM credentials.
- [ ] `FUNC accept(listener AS Listener, timeoutMs AS Integer = 0) AS TlsSocket` — TCP accept + **server-side** TLS handshake, yielding the existing `tls::TlsSocket` (`read`/`write`/`close` reused as-is).
- [ ] Land in both existing `tls` backends (`tls.rs:4-8` — OpenSSL via `dlopen` on Linux, Network.framework on macOS) so `serverSSL` carries no platform gate.

Acceptance: a `tls::listen`+`accept` server completes a server-side handshake on both backends, and the returned `TlsSocket` round-trips via the existing `read`/`write`/`close`.
Commit: —

### Phase A2 — `net` decode helpers

In `src/builtins/net_package.mfb` (the source companion — create it there if absent):

- [ ] `__net_percentDecode(s AS String) AS String` + public `FUNC percentDecode`.
- [ ] `__net_parseQuery(s AS String) AS Map OF String TO String` + public `FUNC parseQuery`.
- [ ] Register both in `src/builtins/net.rs` exactly as the client plan registers `toUrl`/`toAddress` (consts, `is_net_call`, `call_param_names`, `call_return_type_name`, `resolve_call`, `arity`, `implementation_name`).

Acceptance: `net::percentDecode`/`net::parseQuery` resolve, typecheck, and pass their `func_net_*_valid` vectors.
Commit: —

### Phase A3 — `http` shim additions: `src/builtins/http.rs`

- [ ] Consts: `SERVER`, `SERVER_SSL`, `HANDLE_REQUEST`, `ROUTE`, `RESPONSE_DEFAULT`, `OK`, `STATUS`, `JSON`, `WITH_HEADER`, `BYTES`, `RESPOND_FILE`, `RESPOND_PATH`; type names `REQUEST_TYPE`, `REQUEST_PART_TYPE`, `RESPONSE_TYPE`, `ROUTE_TYPE`; the `__http_*` targets. (`SERVER`/`SERVER_SSL` are *call* names, not types; no request-accessor calls — handlers use `collections::*` on the `Request` maps, §F.5.4.)
- [ ] `is_builtin_type` → add `Request | RequestPart | Response | Route` (no server type; listeners are the existing `net`/`tls` resources).
- [ ] `is_http_call` → add the new names.
- [ ] `call_param_names` / `call_return_type_name` / `resolve_call` / `expected_arguments` / `arity` for each (function-typed `handler` argument typed `FUNC(Request) AS Response`).
- [ ] `default_argument_padding`: `server` (host, backlog), `respondFile` (contentType), `route` (none).
- [ ] `server` → `net::Listener`; `serverSSL` → `tls::Listener`; `handleRequest` **overloaded by listener type** (`[net::Listener, List OF Route]` and `[tls::Listener, List OF Route]` → `Nothing`); `consumes_argument` for `respondFile`'s `RES File`.

Acceptance: all new http calls/types resolve and typecheck, including the two `handleRequest` overloads and the function-typed handler argument.
Commit: —

### Phase A4 — Wire into the compiler

- [ ] Add the new calls/types to the client plan's §C Phase 4 touch-points (`src/builtins/mod.rs`, `src/resolver.rs`, `src/typecheck.rs`, `src/ir.rs`) — additive to the existing dispatch.
- [ ] Verify the function-typed `handler` argument typechecks and lowers (the first builtin to take a `FUNC(...)` argument — confirm against the `collections::sortBy`/`reduce` precedent).

Acceptance: the server surface compiles end-to-end and the function-typed handler argument lowers correctly.
Commit: —
