# bug-858: http client returns an empty/partial body for a close-delimited response (no Content-Length, no chunked)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/http/http-client-close-delimited-body-rt (to add; needs a 127.0.0.1 peer that sends head, pauses, sends body, closes)

`http::read` / `http::write` / the `startRead`..`finish` drive stop reading as soon as the
response head arrives when the head carries neither `Content-Length` nor
`Transfer-Encoding: chunked`. Such a response is legal and common (HTTP/1.0 servers, any
HTTP/1.1 server answering a `Connection: close` request): its body is delimited by the
peer closing the connection (RFC 9112 §6.3 rule 8). Every body byte that arrives in a
later TCP read than the head is silently dropped, so the result depends on segmentation:
a server that flushes the head and body in one write appears to work, one that flushes
them separately yields `body = []` with `status = 200` and no error.

The same early-out makes the body of a `Content-Length` response depend on segmentation:
the parser never truncates to `Content-Length`, so bytes past it are kept when they share a
read with the head and dropped when they do not.

Correct behavior: with neither framing header, the client reads to EOF (as the spec says
it does) and `body` is every byte after the head. With `Content-Length: N`, `body` is
exactly the first N body bytes regardless of how the bytes were segmented.

References:

- src/docs/spec/stdlib/05_http.md, intro: "each request opens a connection, sends one
  request with `Connection: close`, reads until end-of-stream, parses, and returns"
- src/docs/spec/stdlib/05_http.md §Server/Request parsing: "the client always reads to EOF"
- src/docs/spec/stdlib/05_http.md §Body Decoding: `body-section otherwise`
- RFC 9112 §6.3 (message body length), rules 6 and 8

## Failing Reproduction

`/tmp/hardening/B04/repro/closedelim_server.py` (127.0.0.1 only):

```python
import socket, time
s=socket.socket(); s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1); s.bind(('127.0.0.1',18100)); s.listen(1)
for path_resp in range(2):
    c,_=s.accept(); c.recv(65536)
    if path_resp==0:
        c.sendall(b"HTTP/1.1 200 OK\r\nServer: x\r\n\r\n"); time.sleep(0.3); c.sendall(b"hello-close-delimited")
    else:
        c.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nabcdef")
    c.close()
```

`/tmp/hardening/B04/repro/closedelim/src/main.mfb`:

```
IMPORT io
IMPORT http
IMPORT net

FUNC main() AS Nothing
  LET a AS http::Response = http::read(net::toUrl("http://127.0.0.1:18100/"))
  io::print("close-delimited: status=" & toString(a.status) & " bodylen=" & toString(len(a.body)) & " body='" & toString(a.body) & "'")
  LET b AS http::Response = http::read(net::toUrl("http://127.0.0.1:18100/"))
  io::print("content-length 2: bodylen=" & toString(len(b.body)) & " body='" & toString(b.body) & "'")
END FUNC
```

```
python3 closedelim_server.py &
/tmp/hardening/mfb build -q closedelim && ./closedelim/build/closedelim.out
```

- Observed:
  ```
  close-delimited: status=200 bodylen=0 body=''
  content-length 2: bodylen=6 body='abcdef'
  ```
- Expected:
  ```
  close-delimited: status=200 bodylen=21 body='hello-close-delimited'
  content-length 2: bodylen=2 body='ab'
  ```

The non-blocking drive (`startRead` / `ready` / `pump` / `done` / `finish`) shows the same
empty body (`drive /closedelim -> 200 body=`, `/tmp/hardening/B04/async`). Responses that
work today: chunked bodies, and `Content-Length` bodies whose bytes match the header.

## Root Cause

`src/codegen/builtins/http/helper_framing_length.rs:__http_framingLength` returns `0` for
"no Content-Length, not chunked" (its comment says "0 for none"). Then
`src/codegen/builtins/http/helper_frame_complete.rs:__http_frameComplete` returns
`have >= framing`, which is `have >= 0`, so it is TRUE as soon as `\r\n\r\n` is seen.
`src/codegen/builtins/http/func_done.rs:__http_done` returns TRUE on `frameComplete`, so
`__http_read` / `__http_write` leave their `WHILE __http_done(s) = FALSE` loop after the
read that completed the head. Close-delimited framing is never represented; it needs a
"read to EOF" sentinel (say `-2`) that `frameComplete` treats as never complete, which leaves
`closed` as the only exit.

The Content-Length half: `src/codegen/builtins/http/helper_parse_response.rs:__http_parseResponse`
passes `bodySection` (everything after the head) to `__http_decodeBody` without truncating
it to the framing length, while `frameComplete` stops reading once `have >= Content-Length`.

## Goal

- A head with neither `Content-Length` nor chunked `Transfer-Encoding` reads to EOF; the repro's first line prints `bodylen=21`.
- A `Content-Length: N` body is exactly N bytes; the repro's second line prints `bodylen=2 body='ab'`.
- 204/304 and HEAD-style handling stay as they are.

### Non-goals (must NOT change)

- The chunked path (`__http_chunkedComplete`) and the server-side framing (`__http_frameAdvance`).
- The 64 MiB response cap still applies to a close-delimited body.

## Blast Radius

- `__http_read`, `__http_write`, `http::done` (and so `startRead`-driven programs): all fixed by this bug.
- `src/codegen/builtins/http/helper_framing_length.rs:__http_framingLength` is client-only (the server uses `__http_requestFraming`). The server is unaffected: a request with no framing has no body (RFC 9112 §6.3 rule 7).

## Golden impact

- `tests/byte-identity/http/golden/http_codegen_cover_rt.ir` and its five
  per-target `.ncode` sums move: `__http_framingLength`'s body (sentinel `0` →
  `-2`, plus a comment), `__http_frameComplete`'s body (a new `IF framing = -2`
  arm) and `__http_decodeBody`'s body (new Content-Length truncation) are all
  `Body::mfb` source injected verbatim into every program that imports `http`,
  so their instruction/line content changes everywhere, not just at the call
  site. Measured: `scripts/artifact-gate.sh target/release/mfb http` → 6/7
  mismatches, all `http_codegen_cover_rt` (`.ir` + 5 `.ncode` sums); no other
  http fixture's goldens move.
- No `tests/rt-behavior/http/**` or `tests/syntax/http/**` `.ast`/`.ir`/`.run`
  golden changes: none of those fixtures embed the raw helper source in a way
  `test-accept.sh` diffs (confirmed by running the existing `rt_http_*`
  cargo-test suites below — all green, unchanged).

## STATUS: FIXED

`__http_framingLength` now returns `-2` (not `0`) for "neither Content-Length
nor chunked", and `__http_frameComplete` treats `-2` as its own case (`RETURN
FALSE` — never complete on content; only `state.closed` ends the read).
`__http_decodeBody` now truncates a Content-Length body to exactly that many
bytes before returning it.

Regression test: `tests/net/rt_http_close_delimited_body.rs` — three cases
against a real `std::net::TcpListener` peer (bound once per test, never
bind-then-release): a close-delimited body split across reads (blocking
`http::read`), the same for the non-blocking `startRead`/`pump`/`done`/`finish`
drive, and a Content-Length response with extra bytes sharing the head's read.
RED on current main (`bodylen=0`/`bodylen=6` instead of `21`/`2`); green after
the fix.

Checks run (all under the shared build lock):
- `cargo test --test rt_http_close_delimited_body` → 3 passed (RED before,
  green after).
- `cargo test --test rt_http_chunked_frame_completion --test
  rt_http_finish_state_err_validation --test rt_http_header_framing_injection
  --test rt_http_async_stream --test rt_http_handle_request_serves --test
  rt_http_server_dos` → 19 passed, 0 failed (no regression in the sibling http
  suites).
- `scripts/spec-census.sh --render stdlib` → 0 leaks; `--citations stdlib` →
  0 miss-path/miss-line/miss-symbol (the `[[...]]` provenance markers into the
  touched helpers still resolve).
- `scripts/man-run-examples.sh http --run` → every client-side example
  (`http::read`, `http::startRead`, …) still runs; the 8 failures reported are
  exactly the pre-registered blocking-server examples in
  `scripts/man-examples-not-run.txt` (`http::handleRequest#1/#2`,
  `http::respondPath#1-3`, `http::server#1`, `http::serverSSL#1/#2`),
  unrelated to this change.
- `scripts/artifact-gate.sh target/release/mfb http` → 6/7 mismatches, all
  `http_codegen_cover_rt` (see Golden impact above); no other diff.

Spec updated in the same change: `src/docs/spec/stdlib/05_http.md` §Body
Decoding now states the Content-Length truncation and the close-delimited
read-to-EOF case explicitly.

### Regression found in review, fixed in the same bug

Coordinator review measured that the `-2` sentinel above made EVERY response
with no `Content-Length` and no chunked framing wait for EOF, which wrongly
included `1xx`/`204`/`304` (both `204` and `304` commonly carry no
`Content-Length`). RFC 9112 §6.3 rules 1-2 make those statuses categorically
bodyless regardless of any framing header present — matching
`__http_decodeBody`, which already discards the body for `204`/`304` on that
basis.

Fix: a new helper, `__http_statusOfHead` (duplicates
`__http_parseStatusLine`'s status-line walk but never raises, returning `-1`
for anything it cannot parse — a malformed status line still surfaces from
`finish`'s parse, not from `done`'s completion check, so `done`'s declared
Errors table is unchanged). `__http_frameComplete` now checks the status via
this helper and returns `TRUE` immediately for `100..199`, `204`, or `304`,
before the Content-Length/chunked/close-delimited arms.

Measured against the coordinator's probe (`/tmp/bs16-rev/b858`, a server that
replies then holds the socket open 6s):

| case | before this fix | after this fix |
|---|---|---|
| `204 No Content` | 6s | 0s |
| `304 Not Modified` | 6s | 0s |
| `200 OK` + `Content-Length: 2` | 0s | 0s (unchanged) |

New RED-then-green cases in `tests/net/rt_http_close_delimited_body.rs`:
`status_204_completes_without_waiting_for_the_peer_to_close` and
`status_304_completes_without_waiting_for_the_peer_to_close`, both against a
peer that holds the connection open 6s after replying and assert the client
finishes in under 3s. RED on the first fix (6.16s / 6.31s measured); green
after this one. The other 3 cases in the same file (close-delimited split
reads, the non-blocking drive, Content-Length truncation) stay green —
re-ran after this change, no regression.

**Known limitation, documented rather than fixed.** A `HEAD` response is also
categorically bodyless (RFC 9112 §6.3 rule 1), but `PendingState` (the
`EXPORT RECORD` a `Stream`'s STATE carries) has no `method` field — only
`sentAll`/`closed`/`raw`/`err` — and `__http_startExchange` discards the
`method` it was given after building the request. `__http_frameComplete`
therefore cannot distinguish a `HEAD` response from a `GET` response carrying
the same headers, so a `HEAD` reply whose head declares `Content-Length` (the
length the body *would* have been for `GET`) still falls through to the
Content-Length arm and waits for however many bytes never arrive — exiting
only via the peer's close (`state.closed`), not immediately at the head.
Measured: a `HEAD` request against a server that answers
`HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n` (correctly sending zero body
bytes) and then holds the socket open 6s before closing — the client reports
the correct `status=200 bodylen=0`, but takes 6s instead of 0s. This is not a
regression from either fix above (the Content-Length arm's `have >= framing`
behavior for a nonzero `framing` is unchanged by this bug); it is a
pre-existing gap this bug doc does not close, because closing it needs
`method` threaded into `PendingState` — a new field on an `EXPORT` record,
which is an observable API surface change beyond what this bug doc settles
and so is left for a follow-up bug/plan rather than folded in here.
