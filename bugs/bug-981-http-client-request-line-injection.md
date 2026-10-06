# bug-981: the http client writes raw URL text into the request line — `http://h/x HTTP/1.0` sends `GET /x HTTP/1.0 HTTP/1.1`

Last updated: 2026-10-05
Effort: small (<1h)
Severity: HIGH
Class: Security (request-line injection) — owner-confirmed

Status: Open
Regression Test: tests/rt-behavior/http/request-target-percent-encoded-rt (raw 127.0.0.1 server logs the request line)

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
