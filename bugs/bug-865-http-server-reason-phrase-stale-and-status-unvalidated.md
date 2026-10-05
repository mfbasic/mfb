# bug-865: http server emits the handler's stale reason phrase (`418 OK` from the spec's own teapot example) and any status integer verbatim (`HTTP/1.1 -5 OK`, `1000`, `0`)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Coordinator note: repro measured by the review agent only; the coordinator did not re-run it.
Regression Test: tests/rt-behavior/http/http_server_loopback (wire-level check of the teapot status line)

Two related defects on the emit path:

1. **The reason phrase is not server-supplied.** The spec says the reason phrase is always
   server-supplied on emit, but the serializer uses `resp.reason` whenever it is non-empty.
   `http::responseDefault()` carries `reason = "OK"`, so the spec's own worked example,
   `WITH resp { status := 418 }`, goes out as `HTTP/1.1 418 OK`. Every `WITH status :=`
   edit of a constructed response has the same problem.
2. **The status is not validated.** A handler status outside `100..599` is serialized
   verbatim: `HTTP/1.1 0 OK`, `HTTP/1.1 -5 OK`, `HTTP/1.1 1000 Server Error`. These are not
   valid status lines (RFC 9112 §4: 3DIGIT). The server's crash-proofing already turns
   other bad handler responses (control bytes) into a 500.

Correct behavior (owner to confirm the first point; class CODE vs SPEC): the reason phrase is
derived from the status at emit (`__http_reasonPhrase`), or `reason` is honoured only when
the handler set it explicitly and the spec is reworded. A status outside `100..599` becomes
a built-in 500, like the other handler defects.

References:

- src/docs/spec/stdlib/05_http.md §Constructors: "`Content-Length`, the reason phrase,
  and `Connection: close` are always server-supplied on emit"
- Same section, the `WITH resp { status := 418 }` teapot example
- src/docs/spec/stdlib/05_http.md §Response Record: "`ok` … TRUE iff 200 <= status <= 299".
  The same `WITH` edit leaves `ok = TRUE` on a 418 record.

## Failing Reproduction

Server `/tmp/hardening/B04/srv`, handlers:

```
FUNC teapot(req AS http::Request) AS http::Response
  MUT resp AS http::Response = http::responseDefault()
  resp = WITH resp { status := 418 }
  resp = WITH resp { body := http::bytes("I'm a teapot") }
  RETURN resp
END FUNC

FUNC weird(req AS http::Request) AS http::Response
  RETURN http::status(toInt(collections::getOr(req.params, "code", "0")), "w")
END FUNC
```

routes `/teapot`, `/code/:code`. Raw client `/tmp/hardening/B04/py/rawc.py teapot code0 codeneg code1000`:

- Observed status lines:
  ```
  HTTP/1.1 418 OK
  HTTP/1.1 0 OK
  HTTP/1.1 -5 OK
  HTTP/1.1 1000 Server Error
  ```
- Expected: `HTTP/1.1 418 I'm a teapot`; the other three answered `HTTP/1.1 500 Internal Server Error`.

Contrast: `http::status(503, …)` / `http::ok` set `reason = ""`, so they get the table
phrase. A handler reason with CR/LF is correctly turned into a 500 (`/badreason`).

## Root Cause

`src/codegen/builtins/http/helper_serialize_head.rs:__http_serializeHead`:
`MUT reason AS String = resp.reason` / `IF reason = "" THEN reason = __http_reasonPhrase(resp.status)`.
`src/codegen/builtins/http/func_response_default.rs:__http_responseDefault` returns
`Response[200, "OK", …]`. `src/codegen/builtins/http/helper_check_response.rs:__http_checkResponse`
checks control bytes only, never the status range.

## Goal

- The teapot example emits `418 I'm a teapot`; out-of-range statuses become 500.

### Non-goals (must NOT change)

- Server-forced `Content-Length` / `Connection: close`.

## Blast Radius

- `__http_serializeHead`, `__http_checkResponse` (TCP and TLS paths share them).
