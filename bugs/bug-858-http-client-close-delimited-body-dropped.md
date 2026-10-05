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
