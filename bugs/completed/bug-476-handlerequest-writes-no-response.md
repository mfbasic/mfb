# bug-476 — `http::handleRequest` serves a request and writes no response at all

- **Severity:** HIGH — the `http` package's whole purpose is serving requests,
  and the documented server does not serve. Every client sees an empty reply.
- **Status:** FIXED (36947920e, audit-3 bug-497 shared the root cause; witness `tests/rt_http_handle_request_serves.rs`)
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

## Root cause (verified 2026-08-31)

**Not in `http` at all, and not (a).** Instrumenting `__http_handleRequest` with
`io::print` at every branch showed the read loop working perfectly — 54 bytes
read, frame complete, a correct `404 Not Found` `Response` built, a 105-byte head
serialized — and then exit **(b)**: `tcp::write` raised `ErrConnectionClosed`
(77070004) while the peer was still connected. Probe writes of a literal string
at four points before it all succeeded and reached the client, so neither the
socket nor its `closed` flag was at fault; only

```mfbasic
tcp::write(sock, __http_serializeHead(resp))
```

failed, while `LET h AS String = __http_serializeHead(resp)` /
`tcp::write(sock, h)` — the identical bytes — succeeded.

`tcp::write` is one member with two overloads, and the *lowering* is chosen at
codegen from the second argument's static type
(`builder_values.rs:lower_runtime_helper_call`, `"tcp.write"` →
`tcp.writeText` for a `String`). That probe was
`CodeBuilder::static_type_name`, whose `NirValue::Call` arm is a hand-written
table of a dozen builtins — a **call result** answered `None`, so the selector
fell to the BYTES form and marshalled the `String*` through the collection path:
a garbage element count, a failed `write(2)`, and `ErrConnectionClosed` raised
with nothing on the wire, straight into the silent `EXIT SUB`.

The defect is general, not http's: the same probe drives ~10 members'
code-form selection. It is written up separately, for findability, as
**bug-483** (`bugs/completed/bug-483-overload-code-form-selection-cannot-type-a-call-result.md`).

**Fix:** `CodeBuilder::overload_arg_type`
(`src/codegen/memory/value/builder_value_semantics.rs`), a probe used only for
code-form selection, which resolves a call against the same return-type tables
`emit_call` uses before falling back to the registry resolver. All nine
type-driven selectors in `lower_runtime_helper_call`, plus
`net_connect_is_address_form` and `net_poll_is_list_form`, read it.
`static_type_name` itself is deliberately NOT widened — it also gates the
in-place append/set fast path, numeric-result typing and the slice
specialisation, including the `x = collections::append(x, f())` aliasing
decision.

**Regression tests:** `tests/rt_http_handle_request_serves.rs` (this bug's
contract: 200 for a matched route, 404 for an unmatched one, both reaching the
wire) and `tests/rt_native_write_overload_call_argument.rs` (the mechanism, one
case per selector shape). Both RED before, GREEN after.

## Where the original triage pointed (kept for the record)

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

## Also worth fixing while there — considered, declined

All three `EXIT SUB`s discard the error. Deliberately left alone:

- With the root cause fixed, a write failure here means the client really did go
  away mid-response, and dropping the connection and returning normally is the
  **documented** contract on the `handleRequest` page ("A write that fails
  mid-response drops the connection and returns normally"). Changing it is a man
  page change, not a bug fix.
- `http` has no logging facility and does not import `io`. Making a library SUB
  print to a server's stdout on every disconnected client is a worse default than
  silence, and MFBASIC has no debug-only build to scope it to.

What actually made the failure invisible was that the error was a *lie*
(`ErrConnectionClosed` on a live connection), and that is fixed.

## Related

- The eight failing `mfb man http` examples: `handleRequest#1`, `#2`,
  `respondPath#1`–`#3`, `server#1`, `serverSSL#1`, `#2`. They are correct as
  written; they fail because the function they demonstrate does not work.
- bug-474, bug-475 — also found by plan-108's example runs.
