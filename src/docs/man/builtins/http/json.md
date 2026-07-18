# json

Build a `200` response whose body is a JSON document.

## Synopsis

```
http::json(body AS String) AS Response
```

## Package

`http`

## Imports

```
IMPORT http
```

## Description

`json` is the JSON success constructor. It returns a new `http::Response` with
`status` `200`, an empty `reason`, `httpVersion` `"1.1"`, a `headers` map holding
the single entry `content-type` → `"application/json"`, a `body` of the UTF-8
bytes of `body`, and `ok` `TRUE`.
[[src/builtins/http_package.mfb:__http_json]]
[[src/builtins/http_package.mfb:__http_responseWith]] [[src/builtins/http.rs:JSON]]

It is exactly `http::ok` with a different content type: the same underlying
constructor, `200` fixed, and `"application/json"` in place of
`"text/plain; charset=utf-8"`. Note the media type carries no `charset`
parameter — JSON is defined as UTF-8, and none is added.
[[src/builtins/http_package.mfb:__http_json]]

It is a pure value constructor: it reads no state, performs no I/O, and mutates
nothing. The same `body` always yields the same response value.

**`body` is not validated, escaped, or serialized.** It is taken verbatim and
encoded as UTF-8. `json` does not parse it, does not check that it is
well-formed, and does not convert an MFBASIC value into JSON text. Build the
document with the `json` package (`json::stringify`) or by hand, then pass the
resulting string here. Passing a non-JSON string produces a `200` response
labelled `application/json` whose body is not JSON; passing `""` produces a
valid `200` with a zero-length body.
[[src/builtins/http_package.mfb:__http_responseWith]]

Three details of the returned value matter in practice:

- `reason` is `""`, not `"OK"`. That is deliberate — when the response is
  serialized, an empty reason is replaced by a phrase derived from the status
  code, so the response goes out as `HTTP/1.1 200 OK` without you setting
  anything. [[src/builtins/http_package.mfb:__http_serializeHead]]
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

When the response is served, `Content-Length` and `Connection` are always
supplied by the server, and any handler-set value for those two names is
dropped. [[src/builtins/http_package.mfb:__http_serializeHead]]

To return JSON under a status other than `200`, use `http::status` and override
the content type with `http::withHeader`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `body` | `String` | The JSON document, sent verbatim. Encoded to bytes as UTF-8. Any string is accepted, including the empty string and text that is not valid JSON. [[src/builtins/http.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Response` | A new response with `status` `200`, `reason` `""`, `httpVersion` `"1.1"`, `headers` containing only `content-type` → `"application/json"`, `body` the UTF-8 bytes of `body`, and `ok` `TRUE`. [[src/builtins/http_package.mfb:__http_json]] |

## Errors

No errors.

## Examples

A JSON handler:

```
IMPORT http

FUNC info(req AS http::Request) AS http::Response
  RETURN http::json("{\"ok\":true}")
END FUNC
```

Serializing a value with the `json` package before sending it:

```
IMPORT http
IMPORT json
IMPORT io
IMPORT collections

SUB main()
  LET fields = Map OF String TO json::Json { "name" := json::JsonStr["mfb"] }
  LET resp = http::json(json::stringify(json::JsonObj[fields]))
  io::print(toString(resp.status) & " " & collections::get(resp.headers, "content-type"))
END SUB
```

Prints `200 application/json`.

Returning JSON with an error status, by overriding the content type:

```
IMPORT http
IMPORT io

SUB main()
  LET resp = http::withHeader(http::status(422, "{\"error\":\"invalid\"}"), "content-type", "application/json")
  io::print(toString(resp.ok))
END SUB
```

Prints `FALSE`.

## See also

- `mfb man http ok`
- `mfb man http status`
- `mfb man http withHeader`
- `mfb man http responseDefault`
- `mfb man json stringify`
