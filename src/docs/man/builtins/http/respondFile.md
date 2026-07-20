# respondFile

Serve the whole contents of an open `File` as a `200` response, consuming the handle

## Synopsis

```
http::respondFile(file AS File) AS Response
http::respondFile(file AS File, contentType AS String) AS Response
```

## Package

`http`

## Imports

```
IMPORT fs
IMPORT http
```

`http` and `fs` are built-in packages, so neither `IMPORT` needs a manifest
dependency. `IMPORT fs` is what a caller needs to obtain the `File`;
`http::respondFile` itself imports `fs` internally.
[[src/builtins/http.rs:augmented_project]]

## Description

`http::respondFile` reads every remaining byte of `file` into memory and returns
a new `http::Response` with `status` `200`, `reason` `"OK"`, `httpVersion`
`"1.1"`, a `headers` map holding the single entry `content-type`, `body` set to
the bytes read, and `ok` `TRUE`.
[[src/builtins/http_package.mfb:__http_respondFile]]

Unlike every other `http::` call, `respondFile` **consumes** its `File`: the
handle is moved into the call and is unusable afterward.
[[src/builtins/http.rs:consumes_argument]] Ownership passing to the callee is
what makes the handle safe — the `File` is closed by lexical drop when
`respondFile` returns, and that also happens on the failure path, so a read error
cannot leak the descriptor. The caller must not close or reuse the handle.

The whole file is buffered into the response body before anything is sent. This
is fine for the modest static assets a development or embedded server serves, but
it is not a streaming API: a large file is held entirely in the arena, and while
it is being read the single-threaded server is not handling other connections.

The read starts at the file's *current* position, not at byte zero, because
`fs::readAllBytes` reads from wherever the handle is positioned. A handle you have
already read from serves only the remainder; open the file fresh to serve it
whole. [[src/builtins/http_package.mfb:__http_respondFile]]

`respondFile` is the low-level primitive. It does not look at any request, resolve
any path, or guess a content type from a filename — it only knows about the open
handle it is given. Most handlers should call `http::respondPath`, which resolves
a request path under a root directory, enforces containment, infers the content
type from the extension, and then calls this function.
[[src/builtins/http_package.mfb:__http_respondPath]]

## Overloads

**`http::respondFile(file AS File) AS Response`**

Serves `file` with the content type `application/octet-stream`. This is not a
separate implementation: the missing argument is filled in as the empty string
during lowering, and an empty `contentType` is what selects the default.
[[src/builtins/http.rs:default_argument_padding]]

**`http::respondFile(file AS File, contentType AS String) AS Response`**

Serves `file` with `contentType` as the `content-type` header value, stored
lowercased under the key `content-type`. Passing `""` explicitly is identical to
omitting the argument and yields `application/octet-stream`.
[[src/builtins/http_package.mfb:__http_respondFile]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `file` | `File` | An open `File` resource opened for reading, such as one from `fs::openFile`. Consumed by the call — the handle is moved, closed on return, and unusable afterward. Read starts at the handle's current position. [[src/builtins/http.rs:call_param_names]] [[src/builtins/http.rs:consumes_argument]] |
| `contentType` | `String` | The media type to advertise, stored under the header key `content-type`. Optional; omitted or `""` means `application/octet-stream`. Stored verbatim, not validated. [[src/builtins/http_package.mfb:__http_respondFile]] |

## Return value

| Type | Description |
| --- | --- |
| `Response` | A response with `status` `200`, `reason` `"OK"`, `httpVersion` `"1.1"`, `headers` containing only `content-type`, `body` set to the bytes read from `file`, and `ok` `TRUE`. An empty file yields a valid `200` with a zero-length body. [[src/builtins/http.rs:call_return_type_name]] [[src/builtins/http_package.mfb:__http_respondFile]] |

## Errors

`respondFile` raises no errors of its own; every error below is propagated from
the `fs::readAllBytes` it performs on `file`.

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `file` has already been closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77020001` | `ErrRead` | `file` cannot be repositioned or measured, was not opened for reading, or the host read fails partway through. [[src/target/shared/code/error_constants.rs:ERR_READ_CODE]] |
| `77010001` | `ErrOutOfMemory` | The body byte list, the header map, or the `Response` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Serve one known file with an explicit content type:

```
IMPORT fs
IMPORT http

FUNC page(req AS http::Request) AS http::Response
  RES f AS File = fs::openFile("./public/page.html")
  RETURN http::respondFile(f, "text/html; charset=utf-8")
END FUNC
```

Serve a binary download, letting the content type default:

```
IMPORT fs
IMPORT http

FUNC download(req AS http::Request) AS http::Response
  RES f AS File = fs::openFile("./data/report.bin")
  RETURN http::respondFile(f)
END FUNC
```

Turn a missing file into a `404` rather than an error:

```
IMPORT fs
IMPORT http

FUNC maybe(req AS http::Request) AS http::Response
  IF fs::fileExists("./public/page.html") = FALSE THEN
    RETURN http::status(404, "Not Found")
  END IF
  RES f AS File = fs::openFile("./public/page.html")
  RETURN http::respondFile(f, "text/html; charset=utf-8")
END FUNC
```

## See also

- `mfb man http respondPath`
- `mfb man http status`
- `mfb man http route`
- `mfb man fs openFile`
- `mfb man fs readAllBytes`
