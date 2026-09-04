# bug-506: HTTP header CRLF injection + request-smuggling toolbox (OS-53/54/55)

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: HIGH
Class: security (injection — request splitting / response splitting / request smuggling)

Status: FIXED — see the STATUS block at the end (found in audit-3, Surface 4 OS-53/54/55; OS-54 agent-demonstrated, OS-54/55 mechanisms lead-verified)

Regression Test: fixtures asserting a `\r`/`\n`/NUL in a method / header value / reason is rejected or escaped, and that duplicate `Content-Length` / whitespace-before-`:` is rejected server-side.

## Summary

The `http` package does not control-byte-check the strings it writes into request
and response heads, and its server-side head parser is permissive in the ways that
make a desync/smuggling toolbox. Three related gaps:

- **OS-54 — client request CRLF via `method`.** `__http_normalizeMethod` rejects
  only `""` and a space, not `\r`/`\n`, so a method containing CRLF injects extra
  request lines/headers.
- **OS-55 — server response splitting.** `__http_serializeHead` interpolates
  `resp.reason`, and each header `key`/`value`, raw — any echoed value
  (`Location`, `Set-Cookie`, a CORS reflection) splits the response.
- **OS-53 — server request smuggling.** Duplicate `Content-Length` is last-wins;
  whitespace before `:` is accepted; obs-fold continuation lines are promoted to
  headers; `Transfer-Encoding: chunked` is matched by substring; a bad
  `Content-Length` becomes 0; the body is not truncated to `Content-Length`.

## Mechanism

```
# src/codegen/builtins/http/helper_normalize_method.rs (OS-54): only "" and " " rejected
  IF strings::contains(method, " ") THEN FAIL ...
  RETURN strings::upper(method)          # \r, \n, NUL pass through

# src/codegen/builtins/http/helper_serialize_head.rs (OS-55): raw interpolation
  head = head & entry.key & ": " & entry.value & crlf   # no control-byte check
  ... "HTTP/1.1 " & status & " " & reason & crlf         # reason raw too
```

Server framing: `helper_header_map_from_head.rs:20`,
`helper_framing_length.rs:14`, `helper_parse_request.rs:53` (OS-53).

bug-262 added a control-byte sweep for header *values* on the client but it does
not cover the method (OS-54) or the server response side (OS-55).

## Reproduction

OS-54 agent-demonstrated against a real upstream (CRLF in the method injected a
second request line). OS-55/OS-53 lead-verified in source (the interpolation sites
and the permissive framing above). A full smuggling PoC needs a front-end proxy;
the parser-level primitives are read directly.

## Best fix

- Reject any `\r`, `\n`, or NUL in a method, a header name, a header value, and
  the reason phrase, at the single serialize/normalize choke points (extend
  bug-262's sweep to the method and to `__http_serializeHead`).
- Server framing: reject a message with more than one `Content-Length`, or with
  both `Content-Length` and `Transfer-Encoding`; match `chunked` as the exact
  final coding, not a substring; reject a header line with whitespace before `:`;
  do not promote obs-fold lines; truncate the body to `Content-Length`.

## Non-goals

No MFBASIC surface change; do not reject a legitimately unusual-but-valid method
token (only the control bytes); keep `http`'s response for well-formed inputs
byte-identical.

## Prior art

audit-2 OS-09 (client request-header CRLF) → this extends the class to the method
and the *server response* side, both uncovered by bug-262. Searched `CRLF`,
`crlf`, `header injection`, `Content-Length`, `Transfer-Encoding`, `smuggling`.

## Reproduction (2026-09-03, fix session)

All three reproduced as claimed, against a scratch `http::handleRequest` echo
server (`/tmp/b506-repro`, release `mfb` at main `4efc93966`) driven by a python
raw-socket peer:

- OS-54: `http::read(u, {}, "GET\r\nX-Injected:1\r\nGET")` reached a python
  listener as `GET\r\nX-INJECTED:1\r\nGET / HTTP/1.1\r\nHost: ...` — the method
  framed an injected header. (A method with a space was already rejected; one
  with only CRLF was not.)
- OS-55: `GET /?to=/x%0d%0aSet-Cookie:%20evil=1%0d%0a%0d%0a<html>injected` through
  a handler doing `http::withHeader(r, "Location", dest)` produced
  `HTTP/1.1 200 OK\r\n...\r\nLocation: /x\r\nSet-Cookie: evil=1\r\n\r\n<html>injected\r\nContent-Length: 35...`
  — a split response.
- OS-53: duplicate `Content-Length: 5` / `10` → 200 with `bodyLen=10`;
  `Content-Length` + `Transfer-Encoding: chunked` → 200; `Host : x` → 200;
  obs-fold ` X-Fold: y` → 200; `Transfer-Encoding: chunked, gzip` → treated as
  chunked; `Content-Length: abc` → 200 with `bodyLen=0`; `Content-Length: 3` with
  10 body bytes → `bodyLen=10`.

## Fix

- OS-54: `__http_normalizeMethod` rejects a method with any control byte
  (`__http_hasControlBytes`, bug-262's sweep) with `ErrInvalidArgument`.
- OS-55: `__http_checkResponse` (called from `__http_buildResponse` on the
  dispatched response) substitutes a built-in `500 Internal Server Error` for a
  response whose reason, or any header name/value, carries a control byte (HTAB
  allowed in a value/reason per RFC 9110; CR/LF/NUL/other C0 and DEL are not —
  `__http_hasFieldControlBytes`). `__http_serializeHead` additionally FAILs on
  the same bytes as a fail-closed backstop.
- OS-53: the server parses its head with a NEW strict parser
  (`__http_requestHeaderMap` + `__http_requestFraming`; the lenient
  `__http_headerMapFromHead`/`__http_framingLength` remain for the client, which
  reads to EOF and is not a trust boundary): a second `Content-Length`,
  `Content-Length` with `Transfer-Encoding`, whitespace before the colon,
  obs-fold, a line with no colon, a non-final or doubled `chunked`, any other
  transfer coding, and a non-digit/signed `Content-Length` all FAIL
  `ErrInvalidFormat` → 400. `__http_parseRequest` slices the body to exactly
  `Content-Length` bytes.

Regression test: `tests/rt_http_header_framing_injection.rs` (RED on main for
every sub-issue; the well-formed exchange is pinned byte-for-byte and was green
before and after). Docs: `handleRequest`/`read`/`write` descriptors,
`src/docs/spec/stdlib/05_http.md`, `.ai/net-tls.md`.

## STATUS: FIXED (624b2dd3f)

Fixed together with bug-507 in one commit: the two bugs share the server read
loop (`__http_readRequestNet`/`Tls` + `__http_frameAdvance`), so the strict
framing rules (506) and the trapped, bounded, incremental scan (507) were
restructured as one change rather than stacking an intermediate loop that
would have been rewritten a second time. Deviation from the fix-bug skill's
one-bug-at-a-time order, deliberate and reported.

Gates on the branch (worktree-B-506, main merged in at 01d1b8716):
`cargo test --no-fail-fast -- --skip artifact_gate_all` → 4598 passed, 0
failed, cargo exit 0 (`/tmp/b506-full.log`); the new RED tests green before and
after the merge; `cargo check --all-targets` clean; `test-accept.sh '*http*'`
14/14; `regen-ncodesum.sh` under bash refreshed 141 goldens of which only the 5
`byte-identity/http` sums moved; `artifact-gate.sh target/release/mfb all` — see
the landing report.
