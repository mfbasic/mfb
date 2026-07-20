# withHeader

Return a copy of a response with one header name set to a value

## Synopsis

```
http::withHeader(resp AS Response, name AS String, value AS String) AS Response
```

## Package

`http`

## Imports

```
IMPORT http
```

`http` is a built-in package, so `IMPORT http` needs no manifest dependency.
[[src/builtins/http.rs:augmented_project]]

## Description

`http::withHeader` returns a new `http::Response` that is a copy of `resp` with
`name` mapped to `value` in its `headers` map. Every other field — `status`,
`reason`, `httpVersion`, `body`, `ok` — is carried over unchanged. `resp` itself
is not modified; `Response` is a plain copyable value record, so this is sugar
over `WITH resp { headers := ... }` and calls chain naturally.
[[src/builtins/http_package.mfb:__http_withHeader]]

The header map is an ordinary `Map OF String TO String`, and the name is used as
the map key **exactly as given**, with no case normalization. Two consequences
follow, and both bite in practice:

- Setting a name that is already present replaces its value. Setting a name that
  differs only in case *adds a second entry*, and both are emitted on the wire.
  The response constructors store their content type lowercased as
  `content-type`, so overriding it means passing `"content-type"` — passing
  `"Content-Type"` sends two content-type headers.
  [[src/builtins/http_package.mfb:__http_responseWith]]
- Response header names go out on the wire spelled the way you wrote them. This
  is the opposite of the request side, where field names are lowercased during
  parsing, so a handler reads request headers in lowercase but writes response
  headers in whatever case it chooses. [[src/builtins/http_package.mfb:Response]]

Two names cannot be set this way. `Content-Length` and `Connection` are framing
headers the server always supplies itself; when the response is serialized, any
entry whose name matches either of them case-insensitively is dropped, and the
server's own correct values are appended. Setting them here is therefore silently
ineffective rather than an error.
[[src/builtins/http_package.mfb:__http_serializeHead]]

`name` and `value` are stored verbatim — not validated, escaped, or scanned for
control characters. Do not build a header value out of unvalidated request data
without checking it yourself.

The first parameter is also accepted under the name `response`.
[[src/builtins/http.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `resp` | `Response` | The response to copy. Not modified. Also accepted under the name `response`. [[src/builtins/http.rs:call_param_names]] |
| `name` | `String` | The header name, used as the map key exactly as written. Matching an existing entry replaces it; differing in case adds a second entry. [[src/builtins/http_package.mfb:__http_withHeader]] |
| `value` | `String` | The header value, stored verbatim. Any string is accepted, including the empty string. [[src/builtins/http.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Response` | A copy of `resp` whose `headers` map additionally maps `name` to `value`; all other fields are unchanged. [[src/builtins/http.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The updated `headers` map or the copied `Response` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Add a caching directive to a text response:

```
IMPORT http

FUNC ping(req AS http::Request) AS http::Response
  RETURN http::withHeader(http::ok("pong"), "cache-control", "no-store")
END FUNC
```

Override the content type set by a constructor — note the lowercase name:

```
IMPORT http

FUNC page(req AS http::Request) AS http::Response
  LET base AS http::Response = http::ok("<h1>hi</h1>")
  RETURN http::withHeader(base, "content-type", "text/html; charset=utf-8")
END FUNC
```

Chain several headers:

```
IMPORT http

FUNC api(req AS http::Request) AS http::Response
  MUT resp AS http::Response = http::json("{\"ok\":true}")
  resp = http::withHeader(resp, "x-request-id", "abc123")
  resp = http::withHeader(resp, "cache-control", "no-store")
  RETURN resp
END FUNC
```

## See also

- `mfb man http ok`
- `mfb man http status`
- `mfb man http json`
- `mfb man http responseDefault`
- `mfb man http handleRequest`
