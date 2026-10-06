# bug-986: an http client HEAD request waits for the server to close when the response carries a non-zero Content-Length

Last updated: 2026-10-06
Effort: small (<1h) once the owner decides the API question below
Severity: LOW
Class: Correctness

Status: Open (needs an owner decision)
Regression Test: (proposed) a HEAD case in `tests/net/rt_http_close_delimited_body.rs`: the peer answers `200` with `Content-Length: 5` and no body, then holds the socket 6 s; the read must finish well under the hold time

`http::read(url, headers, "HEAD")` (and a `startRead` stream driven to `done` with method
`"HEAD"`) does not finish when the response head arrives. It waits until the server closes
the connection, if the head carries a non-zero `Content-Length`. A response to HEAD has no
body regardless of its framing headers (RFC 9112 §6.3 rule 1), and servers routinely send the
`Content-Length` the matching GET would have. The result is correct (`bodylen=0`); only the
completion is late. Against a server that keeps the connection open, the client stalls.

**Correct behavior:** a response to a HEAD request is complete at the end of its head, like
1xx/204/304 (bug-858).

## Root cause

`__http_frameComplete` decides completion from the response head alone (bug-858 added the
1xx/204/304 arm). It cannot tell a HEAD response from a GET response with the same headers:
`http::PendingState` (the `Stream`'s `EXPORT` STATE record) holds only
`sentAll`/`closed`/`raw`/`err`, and `__http_startExchange` drops the `method` argument once
the request is built.

**Owner decision needed:** fixing this means recording the method (or a "bodyless" flag) on
the stream. Either add a field to the exported `http::PendingState` record, which is
user-visible API, or keep the flag somewhere that isn't exported. bs16-agent-d stopped here
because it is an API surface change beyond bug-858's doc.

## Failing Reproduction

Measured by the coordinator on 2026-10-06 with main at c6dd68acc
(`/tmp/main-dbg-target/release/mfb`), macOS aarch64.

```
mfb init h   # src/main.mfb:
IMPORT http
IMPORT net
IMPORT io
IMPORT os

FUNC main AS Integer
  LET u = net::toUrl("http://127.0.0.1:" & os::getEnv("P") & "/")
  LET h AS Map OF String TO String = Map OF String TO String {}
  LET r = http::read(u, h, "HEAD") TRAP(e)
    io::print("err " & e.message)
    RETURN 1
  END TRAP
  io::print("status=" & toString(r.status) & " bodylen=" & toString(len(r.body)))
  RETURN 0
END FUNC
```

The peer is a python server that sends the response bytes, then holds the socket 6 s before
closing (`/tmp/bs16-rev/b858/srv.py <response>`).

| response | elapsed | output |
|---|---|---|
| `HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n` | 6 s | `status=200 bodylen=0` |
| `HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n` (control) | 0 s | `status=200 bodylen=0` |

Expected: 0 s for both.

## Related

- Found by bs16-agent-d while fixing bug-858 (close-delimited bodies). Reproduced by the
  coordinator at main, so it predates bug-858's fix, which leaves the Content-Length arm
  unchanged.
