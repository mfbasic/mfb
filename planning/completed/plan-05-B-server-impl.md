# plan-05-B: HTTP server — `.mfb` server implementation

Last updated: 2026-06-25
Effort: large

Part **B** of plan-05 (HTTP Server). The genuinely new logic: the `http_package.mfb` server body —
route matching, the multipart parser, and the accept→parse→dispatch→respond lifecycle over both TCP
and TLS transports. Shared design lives in the overview:
[plan-05-http-server.md](plan-05-http-server.md).

- **Depends on:** plan-05-A (TLS prims, `net` helpers, `http` shim, compiler wiring).
- **Blocks:** plan-05-C (docs + tests).
- **Spec/design:** overview §F.3 (routing), §F.4 (request parsing), §F.5.3/§F.5.5 (constructors,
  static helpers).

## Phases

### Phase B1 — `http_package.mfb` server implementation

Add to `http_package.mfb` (header already imports `net`/`strings`/`collections`/`errorCode`; add `IMPORT fs`):

- [ ] Export `Request`, `RequestPart`, `Response`, `Route`.
- [ ] Pure core (shared with / adapted from the client): request-line/header parser, chunk decoder, `__http_matchRoute(routes, path)` (segment walker producing the bound `params`), the multipart parser, percent-decode/query via `net::*`.
- [ ] Lifecycle: `__http_server` (wraps `net::listenTcp`), `__http_serverSSL` (wraps `tls::listen`), and the two `handleRequest` overloads — TCP body via `net::accept`/`read`/`write`/`close`, TLS body via `tls::accept`/`read`/`write`/`close`; both run accept → read-to-frame → parse → match → `TRAP`-wrapped handler → emit → close over the shared pure core (the two transport bodies cannot share one socket variable — `Socket` vs `TlsSocket`, §B.5).
- [ ] Constructors/static: the §F.5.3 response constructors and §F.5.5 static helpers.
- [ ] Honor the client plan's §C Phase 3 source-package constraints (reserved words not identifiers; ≤ 8 params; no field-target assignment — build with constructors / `WITH`, thread parser cursor state through a small node record; cross-file visibility needs `EXPORT`; escape `\r\n` in literals).

Acceptance: an end-to-end request flows accept→parse→dispatch→respond over both transports; a handler crash routes to `500`; no match → `404`.
Commit: —
