# bug-476 — `http::handleRequest` serves a request and writes no response at all

- **Severity:** HIGH — the `http` package's whole purpose is serving requests,
  and the documented server does not serve. Every client sees an empty reply.
- **Status:** open
- **Found by:** plan-108-F, running every `mfb man` example on every page. Eight
  of `http`'s 38 examples fail, all of them the server-shaped ones.
- **Platforms:** macOS AArch64 verified. Not platform-specific by inspection —
  the failing code is the MFBASIC body, not a per-OS emitter.

## Reproduction

```mfbasic
IMPORT http
IMPORT tcp
IMPORT io

SUB main()
  LET routes AS List OF http::Route = []
  RES s AS tcp::Listener = http::server(18085)
  io::print("listening")
  http::handleRequest(s, routes)
  io::print("served one")
END SUB
```

```
$ printf 'GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n' \
    | nc -w 40 127.0.0.1 18085
                       <-- nothing, for the full 40 seconds
$ cat server.log
listening
served one             <-- the call returned, having written nothing
```

An empty route list must answer **404 Not Found** per the page ("a path
matching no route is answered with 404 Not Found"). It answers nothing. Adding
a matching route changes nothing:

```mfbasic
routes = collections::append(routes, http::route("/", home))   ' same: empty reply
```

`curl` sees it as `* Empty reply from server`.

## The transport is not at fault

The same accept/read/write sequence written directly against `tcp` works
perfectly on the same machine and port range:

```mfbasic
RES l AS tcp::Listener = tcp::listen("0.0.0.0", 18084, 8)
RES s AS tcp::Socket = tcp::accept(l)
LET got AS List OF Byte = tcp::read(s, 65536)
io::print("read " & toString(len(got)) & " bytes")
tcp::write(s, "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi")
```

```
listening
read 54 bytes
wrote
```

and the client receives the full response. So `tcp::listen`, `tcp::accept`,
`tcp::read` and `tcp::write` are all fine; the fault is inside
`__http_handleRequest`.

## Where to look

`src/codegen/builtins/http/func_handle_request.rs`, `const BODY`. The SUB has
three silent exits, and the symptom ("returned, wrote nothing") means it took
one of them:

```mfbasic
  IF len(raw) = 0 THEN
    EXIT SUB                     ' <-- (a) read loop produced nothing
  END IF
  ...
  tcp::write(sock, __http_serializeHead(resp)) TRAP(e)
    EXIT SUB                     ' <-- (b) head write failed, silently
  END TRAP
  IF len(resp.body) > 0 THEN
    tcp::write(sock, resp.body) TRAP(e)
      EXIT SUB                   ' <-- (c) body write failed, silently
    END TRAP
  END IF
```

(a) is the likely one. The read loop is

```mfbasic
    MUT chunk AS List OF Byte = []
    chunk = tcp::read(sock, 65536) TRAP(e)
      RECOVER []
    END TRAP
    IF len(chunk) = 0 THEN
      reading = FALSE
```

so if the assignment-with-`TRAP` leaves `chunk` empty — or `tcp::read` raises
where the bare call above does not — the loop ends on the first pass with `raw`
empty and the SUB returns before writing. The raw-`tcp` control above proves
`tcp::read` itself returns 54 bytes for exactly this request, so the difference
is in how the body calls it.

Worth checking against bug-468 (a record-field assignment that silently parses
as equality and is discarded) — an assignment whose result is dropped has the
same signature as what is seen here.

## Also worth fixing while there

All three `EXIT SUB`s discard the error. A server that cannot answer should not
look identical to one that answered — at minimum the write failures should be
distinguishable in a debug build.

## Related

- The eight failing `mfb man http` examples: `handleRequest#1`, `#2`,
  `respondPath#1`–`#3`, `server#1`, `serverSSL#1`, `#2`. They are correct as
  written; they fail because the function they demonstrate does not work.
- bug-474, bug-475 — also found by plan-108's example runs.
