# plan-05-C: HTTP server — man pages, docs, tests

Last updated: 2026-06-25
Effort: medium

Part **C** of plan-05 (HTTP Server). Closes the feature: man pages, user documentation, and the
golden test suite (offline route/parse/multipart/response/static + one gated loopback test). Shared
design lives in the overview: [plan-05-http-server.md](plan-05-http-server.md).

- **Depends on:** plan-05-A (surface) and plan-05-B (server implementation).
- **Spec/design:** overview §F.3 (routing rules), §F.5 (API tables), §F.7 (worked example).

## Phases

### Phase C1 — Man pages

- [ ] `src/man/builtins/http/`: add `server.txt`, `handleRequest.txt`, `route.txt`, `responseDefault.txt`, `ok.txt`, `status.txt`, `json.txt`, `withHeader.txt`, `bytes.txt`, `respondFile.txt`, `respondPath.txt`, and a routing/types note in `package.txt` (no request-accessor pages — handlers use `collections::*`).
- [ ] Wire `build.rs` and `src/man/mod.rs` as the client plan's §C Phase 5 describes.

Acceptance: `mfb man http` surfaces every new server page via the man pipeline.
Commit: —

### Phase C2 — User documentation

- [ ] `specifications/standard_package.md`: extend the "Built-in HTTP Package" section with a **Server** subsection — the `Request`/`RequestPart`/`Response`/`Route` blocks, the §F.5 API tables, the §F.3 routing rules, and the request-parsing/lifecycle summaries; add `net::percentDecode`/`net::parseQuery` to §11.
- [ ] `error_codes.md`: note `http`(server) on `ErrInvalidFormat`, `ErrInvalidArgument`, `ErrOverflow`, and the `7-707-*` transport rows.

Acceptance: the server docs + error-code notes are published; `mfb spec`/docs surface them.
Commit: —

### Phase C3 — Tests (golden)

Mirror `tests/func_http_*`; split by network dependence.

- [ ] Offline route matching: `func_http_match_*` — `__http_matchRoute` over literal, `:param`, `:bar?` present/absent, `/static/*` remainder, trailing-slash normalization, order/first-wins, mid-pattern `*`/`?` rejected (`ErrInvalidArgument`).
- [ ] Offline request parsing: `func_http_parserequest_*` — `GET`+query, `POST`+`Content-Length`, chunked, malformed request line (`400`), oversize (`413`).
- [ ] Offline multipart: `func_http_multipart_*` — a canned `multipart/form-data` body → `parts` map (field + file upload, filename/contentType/body); malformed boundary.
- [ ] Offline helpers: `func_net_percentdecode_valid` / `func_net_parsequery_valid`.
- [ ] Offline responses: `func_http_response_*` — `responseDefault`/`ok`/`status`/`json`/`withHeader`/`bytes`; status-line/reason derivation; reserved `Content-Length` ignored.
- [ ] Offline static: `func_http_respondpath_*` — extension→Content-Type; **traversal attempts (`../`, absolute) → `403`**; missing file → `404`.
- [ ] Networked (gated; on-device, not the default sweep): `http_server_loopback` — a `http::server` on `127.0.0.1` with a small route list, driven by the `http::read` client for `200`/`404`/`500`/path-param/query/static round-trips; hermetic, no external host.

Acceptance: the full offline suite passes deterministically and the gated loopback test passes on-device; acceptance suite green.
Commit: —
