# responseDefault

Return a fresh `200 OK` response value, the base for `WITH` edits.

## Synopsis

```
http::responseDefault() AS Response
```

## Package

`http`

## Imports

```
IMPORT http
```

## Description

`responseDefault` takes no arguments and returns a newly constructed
`http::Response` with `status` `200`, `reason` `"OK"`, `httpVersion` `"1.1"`, an
**empty** headers map, a `body` of the two bytes of the text `"OK"`, and `ok`
`TRUE`. It reads no state and has no side effects; every call returns the same
value. [[src/builtins/http_package.mfb:__http_responseDefault]]
[[src/builtins/http.rs:RESPONSE_DEFAULT]]

It exists because `http::Response` cannot be mutated in place — MFBASIC has no
field-target assignment — so a handler that wants a response shape the other
constructors do not produce starts from this value and rewrites fields with
`WITH`. When `http::ok`, `http::status`, or `http::json` already produce what you
want, use those instead: they set a `content-type` header, which
`responseDefault` does not. [[src/builtins/http_package.mfb:__http_responseWith]]

Two consequences of the returned field values matter when you edit them:

- `ok` is a plain stored field, not a computed one. `WITH resp { status := 500 }`
  leaves `ok` `TRUE`; set it yourself in the same or a following `WITH` if
  anything downstream reads it. [[src/builtins/http_package.mfb:__http_responseDefault]]
- `reason` is `"OK"` rather than `""`. The server fills in a status-appropriate
  reason phrase only when `reason` is empty, so a response whose `status` you
  changed but whose `reason` you did not will be written on the wire as, for
  example, `HTTP/1.1 418 OK`. Set `reason` explicitly, or clear it to `""` to let
  the server derive it. [[src/builtins/http_package.mfb:__http_serializeHead]]
  [[src/builtins/http_package.mfb:__http_reasonPhrase]]

When the response is served, `Content-Length` and `Connection` are always supplied
by the server and any handler-set value for those two names is dropped; the empty
headers map here means no other header is emitted.
[[src/builtins/http_package.mfb:__http_serializeHead]]

## Parameters

`responseDefault` takes no parameters. [[src/builtins/http.rs:arity]]

## Return value

| Type | Description |
| --- | --- |
| `Response` | A new response with `status` `200`, `reason` `"OK"`, `httpVersion` `"1.1"`, `headers` an empty `Map OF String TO String`, `body` the UTF-8 bytes of `"OK"`, and `ok` `TRUE`. [[src/builtins/http_package.mfb:__http_responseDefault]] |

## Errors

No errors.

## Examples

Building a custom status and body with `WITH`, keeping `reason` and `ok`
consistent:

```
IMPORT http

FUNC teapot() AS http::Response
  MUT resp AS http::Response = http::responseDefault()
  resp = WITH resp { status := 418 }
  resp = WITH resp { reason := "I'm a teapot" }
  resp = WITH resp { ok := FALSE }
  resp = WITH resp { body := http::bytes("no coffee here") }
  RETURN resp
END FUNC
```

Starting from the default and adding a header:

```
IMPORT http

LET resp = http::withHeader(http::responseDefault(), "cache-control", "no-store")
io::print(toString(resp.status) & " " & resp.reason)
```

## See also

- `mfb man http ok`
- `mfb man http status`
- `mfb man http json`
- `mfb man http withHeader`
- `mfb man http bytes`
