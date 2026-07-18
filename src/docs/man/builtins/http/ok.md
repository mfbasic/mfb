# ok

Build a `200` response whose body is text and whose content type is plain text.

## Synopsis

```
http::ok(body AS String) AS Response
```

## Package

`http`

## Imports

```
IMPORT http
```

## Description

`ok` is the success constructor for a text handler. It returns a new
`http::Response` with `status` `200`, an empty `reason`, `httpVersion` `"1.1"`, a
`headers` map holding the single entry `content-type` →
`"text/plain; charset=utf-8"`, a `body` of the UTF-8 bytes of `body`, and `ok`
`TRUE`. [[src/builtins/http_package.mfb:__http_ok]]
[[src/builtins/http_package.mfb:__http_responseWith]] [[src/builtins/http.rs:OK]]

It is a pure value constructor: it reads no state, performs no I/O, and mutates
nothing. The same `body` always yields the same response value.

Three details of the returned value matter in practice:

- `reason` is `""`, not `"OK"`. That is deliberate — the server derives a
  status-appropriate reason phrase whenever `reason` is empty, so the response
  goes out on the wire as `HTTP/1.1 200 OK` without you setting anything.
  [[src/builtins/http_package.mfb:__http_serializeHead]]
  [[src/builtins/http_package.mfb:__http_reasonPhrase]]
- `ok` is a plain stored field, computed once here from the status being in
  `200..299`. A later `WITH resp { status := 500 }` leaves `ok` `TRUE`; set it
  yourself if anything downstream reads it.
  [[src/builtins/http_package.mfb:__http_responseWith]]
- The header name is stored lowercased, as `content-type`. `http::withHeader`
  stores the name exactly as you give it, so overriding the content type means
  passing the lowercase `"content-type"`; `"Content-Type"` adds a *second* map
  entry and both are emitted. [[src/builtins/http_package.mfb:__http_withHeader]]
  [[src/builtins/http_package.mfb:__http_serializeHead]]

`body` is not escaped, truncated, or inspected in any way; an empty string
produces a valid `200` with a zero-length body. When the response is served,
`Content-Length` and `Connection` are always supplied by the server, and any
handler-set value for those two names is dropped.
[[src/builtins/http_package.mfb:__http_serializeHead]]

To send a different status use `http::status`, for JSON use `http::json`, and to
add further headers wrap the result with `http::withHeader`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `body` | `String` | The response body. Encoded to bytes as UTF-8. Any string is accepted, including the empty string. [[src/builtins/http.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Response` | A new response with `status` `200`, `reason` `""`, `httpVersion` `"1.1"`, `headers` containing only `content-type` → `"text/plain; charset=utf-8"`, `body` the UTF-8 bytes of `body`, and `ok` `TRUE`. [[src/builtins/http_package.mfb:__http_ok]] |

## Errors

No errors.

## Examples

A plain-text handler:

```
IMPORT http

FUNC home(req AS http::Request) AS http::Response
  RETURN http::ok("welcome")
END FUNC
```

Adding a header to a text response:

```
IMPORT http

LET resp = http::withHeader(http::ok("pong"), "cache-control", "no-store")
io::print(toString(resp.status) & " " & toString(len(resp.body)))
```

## See also

- `mfb man http status`
- `mfb man http json`
- `mfb man http withHeader`
- `mfb man http responseDefault`
- `mfb man http bytes`
