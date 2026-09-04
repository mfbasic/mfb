# bug-506: HTTP header CRLF injection + request-smuggling toolbox (OS-53/54/55)

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: HIGH
Class: security (injection — request splitting / response splitting / request smuggling)

Status: Open (found in audit-3, Surface 4 OS-53/54/55; OS-54 agent-demonstrated, OS-54/55 mechanisms lead-verified)

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
