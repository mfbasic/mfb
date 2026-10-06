# bug-981: the http client writes raw URL text into the request line — `http://h/x HTTP/1.0` sends `GET /x HTTP/1.0 HTTP/1.1`

Last updated: 2026-10-05
Effort: small (<1h)
Severity: HIGH
Class: Security (request-line injection) — owner-confirmed

Status: Resolved (2026-10-05)
Regression Test: tests/net/rt_http_client_request_target_percent_encoding.rs (raw peer logs the request line)

## Resolution

Fixed in `__http_requestTarget` (`helper_request_target.rs`): space, DEL and
non-ASCII bytes in the path/query are now percent-encoded via a new
`__http_percentEncodeTarget` helper (plus `__http_percentByteEncode`,
`__http_isHexDigitChar`) before the target is written into the request line,
matching Node's `new URL(...).pathname + search`. An existing `%XX` escape is
kept verbatim; a bare `%` becomes `%25`. Control bytes stay rejected by the
existing `__http_hasControlBytes` check.

RED: `bug-981 RED: pin the request-line injection exploit` (commit
`0697c5fa7`) — 4 of 5 cases failed with the exact exploit
(`"GET /x HTTP/1.0 HTTP/1.1"` instead of `"GET /x%20HTTP/1.0 HTTP/1.1"`).
GREEN: `bug-981: percent-encode the http client's request target` (commit
`668b6d3d4`) — all 5 cases pass.


**Owner decision (2026-10-05): percent-encode, as Node does.** Space, DEL and non-ASCII bytes in
the path and query are percent-encoded in the request target before it is written, so the
request line is always `METHOD target HTTP/1.1` with a single-token target.

## Measured (oracle side probe `/tmp/urlo/v4`, raw 127.0.0.1 server logging the first line)

| `http::read(net::toUrl(…))` | request line sent |
| --- | --- |
| `http://127.0.0.1:P/a b` | `GET /a b HTTP/1.1` |
| `http://127.0.0.1:P/x HTTP/1.0` | `GET /x HTTP/1.0 HTTP/1.1` |
| `http://127.0.0.1:P/é?q=a b` | raw UTF-8 and a space |

## Root Cause

`__http_requestTarget` copies path+query verbatim. `__http_buildRequest` rejects only bytes
below 0x20 (`__http_hasControlBytes`), not SP, DEL or non-ASCII, all of which RFC 9112 forbids
in a request-target. `mfb spec stdlib http` "Request Construction" promises
`METHOD target HTTP/1.1`, which a target with a space breaks.

## Goal

- The three repros send `GET /a%20b`, `GET /x%20HTTP/1.0`, and `GET /%C3%A9?q=a%20b`.
  Already-encoded `%XX` sequences are not double-encoded (match Node's
  `new URL(...).pathname + search`).
- Control bytes stay rejected as today.

## Golden impact

`__http_requestTarget`'s body changed (now calls the new `__http_percentEncodeTarget`
helper) and three new private helpers (`__http_percentEncodeTarget`,
`__http_percentByteEncode`, `__http_isHexDigitChar`) were added to the `http` package's
assembled MFBASIC source. Every fixture that `IMPORT http`s will therefore show a changed
`.ast`/`.ir`/`.ncode` (the helper section grows and one existing helper's body text
changes) — this is a change-sentinel for the fact the http source changed, not a
correctness signal on its own. Expect diffs under `tests/byte-identity/http/`,
`tests/syntax/http/**`, and any `tests/rt-behavior/http/**`/`tests/rt-error/http/**`
fixture with a `.ncodesum`/`.ast`/`.ir` golden. No other package's assembled source
changed.
