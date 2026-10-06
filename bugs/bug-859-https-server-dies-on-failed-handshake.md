# bug-859: one non-TLS client kills an `http::serverSSL` accept loop — `handleRequest` re-raises the handshake failure

Last updated: 2026-10-05
Effort: small (<1h)
Severity: HIGH
Class: Security

Status: Open
Regression Test: tests/rt-behavior/http/https-server-survives-bad-handshake-rt (to add)

**Owner decision (2026-10-05):** a failed handshake still raises `ErrTlsFailed` from `http::handleRequest` so the
caller can see and log it. It costs only that connection: the failed connection is closed,
and the listener stays open and keeps accepting new connections. The next `handleRequest`
call on the same listener serves the next client normally. Because the error is still
raised, the spec's and man page's accept-loop examples must TRAP it (e.g. log and continue)
instead of `DO http::handleRequest(s, routes) LOOP UNTIL FALSE` with no handler. The
"crash-proof / never dies" prose is reworded to say the *listener* survives.

A single TCP connection to an `http::serverSSL` listener that sends plaintext (or any
bytes that are not a TLS ClientHello) makes `http::handleRequest(tls::Listener, …)` raise
`ErrTlsFailed` (77070008). The spec's own accept loop (`DO http::handleRequest(s, routes)
LOOP UNTIL FALSE`) has no TRAP, so the process exits and the HTTPS service is gone. Any
port scanner, health probe or `curl http://` against an HTTPS port triggers it. It is a
single-packet remote DoS.

Correct behavior: the failed handshake costs only that connection. `handleRequest`
closes it and returns normally, as it already does for every post-accept peer error.

References:

- src/docs/spec/stdlib/05_http.md §Server/Lifecycle: "It is crash-proof: … a peer I/O
  error drops the one connection without failing the loop. … a malformed request can cost
  its own connection, never the process."
- `mfb man http handleRequest`: "Crash-proofing. The accept loop never dies on a bad
  client." The same page also says "an HTTPS handshake/record failure raises
  ErrTlsFailed". The two sentences contradict each other: a handshake failure *is* a bad
  client. The owner must pick one. The spec text and the safety argument favour "never
  dies".

## Failing Reproduction

```
cd /tmp/hardening/B04/tlss && openssl req -x509 -newkey rsa:2048 -nodes -keyout key.pem -out cert.pem -days 2 -subj "/CN=127.0.0.1" -addext "subjectAltName=IP:127.0.0.1"
```

`/tmp/hardening/B04/tlssrv/src/main.mfb`:

```
IMPORT io
IMPORT http
IMPORT tls
IMPORT collections

FUNC home(req AS http::Request) AS http::Response
  RETURN http::ok("tls hello " & req.method & " " & req.path & " q=" & collections::getOr(req.query, "a", "-"))
END FUNC

FUNC main() AS Nothing
  RES s AS tls::Listener = http::serverSSL(18443, "/tmp/hardening/B04/tlss/cert.pem", "/tmp/hardening/B04/tlss/key.pem", "127.0.0.1")
  MUT rs AS List OF http::Route = []
  rs = collections::append(rs, http::route("/*", home))
  FOR i = 1 TO 3
    http::handleRequest(s, rs)
  NEXT
END FUNC
```

```
/tmp/hardening/mfb build -q tlssrv && ./tlssrv/build/tlssrv.out &
curl -sk -i "https://127.0.0.1:18443/x/y?a=%41"      # request 1: fine
printf 'GARBAGE NOT TLS\r\n\r\n' | nc -w 2 127.0.0.1 18443   # request 2
pgrep -fl tlssrv.out || echo "server dead"
```

- Observed: request 1 returns `HTTP/1.1 200 OK … tls hello GET /x/y q=A`. After request 2
  the server prints
  ```
  Error: 7-707-0008
  TLS handshake, certificate validation, SNI validation, or protocol operation failed.
  ```
  and exits (`server dead`). The third iteration never runs.
- Expected: request 2's connection is dropped, `handleRequest` returns, and the loop goes
  on to serve the next client.

Contrast: the plaintext `http::server` loop survives every malformed-request case tried
(garbage bytes, obs-fold, duplicate Content-Length, 70 KB unterminated head, idle/slow
clients, all answered 400/408/431 with the process still running, from
`/tmp/hardening/B04/py/rawc.py`), because its peer errors all happen after `tcp::accept`.

## Root Cause

`src/codegen/builtins/http/func_handle_request.rs:__http_handleRequestSSL` (`BODY_SSL`)
starts with `RES sock AS tls::Socket = tls::accept(listener)` and has no TRAP on it.
`tls::accept` performs the server-side handshake (spec 17_transports: its `timeoutMs`
"bounds two steps … the wait for an inbound connection … once a connection is taken, its
handshake"), so a peer-caused handshake failure raises `ErrTlsFailed` out of
`handleRequest`. The plaintext twin has nothing comparable, because `tcp::accept` does no
peer I/O.

Fix direction: TRAP the accept. For `ErrTlsFailed`, and for `ErrTimeout` if a handshake
deadline is ever passed, `RETURN` after the failed connection is gone. Keep raising the
listener-level failures (`ErrResourceClosed`/`ErrResourceMoved`/`ErrInvalidArgument`).
This only works if `tls::accept` can report "the handshake failed" separately from "the
listener is broken". If it cannot, it needs a distinct code or a lower-level accept.

## Goal

- After the `nc` probe, `handleRequest` returns normally and the next `curl -sk` request gets `200`.
- A broken listener (closed handle) still raises out of `handleRequest`.

### Non-goals (must NOT change)

- `tls::accept`'s own contract for direct callers.
- Do not "fix" this by documenting that the caller must TRAP. That leaves the spec's worked example DoS-able.

## Blast Radius

- `__http_handleRequestSSL`: fixed by this bug.
- `__http_handleRequest` (plaintext): unaffected (no peer I/O in `tcp::accept`).
- Any user loop that calls `tls::accept` directly has the same hazard. That is caller-owned, out of scope.

## Golden impact

None expected. The owner decision settles this as a documentation fix: `handleRequest`
already raises `ErrTlsFailed` for a failed handshake and already keeps the listener/accept
loop usable for the next call (verified directly against the base compiler: a caller that
`TRAP`s the call, as the corrected examples now show, serves request 3 normally after
request 2's garbage probe). Only `DESC`/`EX` registry doc strings in
`func_handle_request.rs` and prose in `src/docs/spec/stdlib/05_http.md` changed — neither
feeds codegen, so no `.ast`/`.ir`/`.ncode`/`.run` golden is expected to move.
