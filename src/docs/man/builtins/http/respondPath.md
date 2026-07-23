# respondPath

Serve a request's path as a static file from under a root directory

## Synopsis

```
http::respondPath(req AS Request, root AS String) AS Response
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

`http::respondPath` turns a request into a static-file response: it derives a
relative path from `req`, resolves it under `root`, checks that the result really
is inside `root`, infers a content type from the file extension, and serves the
file with `http::respondFile`. It is the whole of the built-in static-file
handler. [[src/builtins/http_package.mfb:__http_respondPath]]

The relative path is taken from `req.params["*"]` when the matched route captured
a wildcard remainder, and from `req.path` otherwise. One leading `/` is stripped,
and a path that is then empty becomes `index.html`. The result is joined to `root`
with `fs::pathJoin`. [[src/builtins/http_package.mfb:__http_respondPath]]

The steps then run **in this order**, and the order is observable:

1. If `fs::fileExists` reports the joined candidate is not an existing regular
   file, a `404` is returned. Directories are not regular files, so a request for
   a directory yields `404`; there is no directory listing and no implicit
   `index.html` inside a subdirectory.
   [[src/builtins/http_package.mfb:__http_respondPath]]
2. Otherwise `fs::isWithin(root, candidate)` decides containment. If it reports
   the candidate is not inside `root`, a `403` is returned and the file is never
   opened. An error raised by `isWithin` itself is trapped and treated as *not
   contained*, so it also yields `403`.
   [[src/builtins/http_package.mfb:__http_respondPath]]

Because existence is tested first, an escaping path that does not exist is
answered `404`, not `403`; only an escaping path that *does* exist reaches the
containment check. Both responses are built with `http::status`, so each carries
a plain-text body (`"Not Found"` / `"Forbidden"`), `content-type`
`text/plain; charset=utf-8`, and `ok` `FALSE`.
[[src/builtins/http_package.mfb:__http_status]]

The containment check is where the traversal defense lives, and it is worth being
precise about what it does and does not guarantee. `fs::isWithin` canonicalizes
both paths with the host `realpath` resolution — collapsing `..`, following every
symbolic link, and resolving relative paths against the working directory — then
compares at a separator boundary. That defeats `..` traversal, a symlink pointing
out of the root, and an absolute path smuggled in through `fs::pathJoin` (which
restarts at any absolute component).
[[src/target/shared/code/fs/paths.rs:lower_fs_is_within_helper]]
[[src/target/shared/code/fs/paths.rs:lower_fs_path_join_helper]]

However, `respondPath` **checks and then opens**, using `fs::openFile` rather
than the atomic `fs::openWithin`. That leaves the time-of-check/time-of-use race
inherent to any check-then-open: a component of the path can be replaced with a
symlink after `isWithin` returns and before the open happens. Under a threat model
where an attacker can create symlinks inside `root`, this is not an airtight
confinement boundary. [[src/builtins/http_package.mfb:__http_respondPath]]

The content type is inferred from the lowercased text after the final `.`, and
only when that dot comes after the final `/`, so an extensionless name or a dot
that belongs to a directory component is not treated as an extension. The
recognized extensions are `html`/`htm`, `css`, `js`/`mjs`, `json`, `txt`/`text`,
`xml`, `csv`, `png`, `jpg`/`jpeg`, `gif`, `svg`, `ico`, `webp`, `woff`, `woff2`,
`ttf`, `pdf`, and `wasm`. Anything else, including no extension at all, is served
as `application/octet-stream`.
[[src/builtins/http_package.mfb:__http_extContentType]]

The whole file is buffered into the response body, exactly as in
`http::respondFile`; there is no streaming and no range support. Serving a large
file occupies the single-threaded server for the duration of the read.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `req` | `Request` | The request to serve. Only `params["*"]` and `path` are read. Also accepted under the name `request`. [[src/builtins/http.rs:call_param_names]] |
| `root` | `String` | The directory that files are served from and confined to. Interpreted by `fs::pathJoin` and `fs::isWithin`; may be absolute or relative to the working directory, and must exist for the containment check to succeed. [[src/builtins/http_package.mfb:__http_respondPath]] |

## Return value

| Type | Description |
| --- | --- |
| `Response` | A `200` response carrying the file's bytes and an inferred `content-type` on success; a `404` plain-text response when no regular file exists at the resolved path; a `403` plain-text response when the resolved path is not contained in `root`. [[src/builtins/http.rs:call_return_type_name]] [[src/builtins/http_package.mfb:__http_respondPath]] |

## Errors

A missing file and an escaping path are **not** errors — they are returned as
`404` and `403` responses. The errors below are propagated from the `fs` calls
that run once the path has been accepted.

| Code | Name | Raised when |
| --- | --- | --- |
| `77030001` | `ErrPathNotFound` | The confined file disappears between the existence check and the open. [[src/target/shared/code/error_constants.rs:ERR_PATH_NOT_FOUND_CODE]] |
| `77030003` | `ErrAccessDenied` | The host denies read access to the confined file. [[src/target/shared/code/error_constants.rs:ERR_ACCESS_DENIED_CODE]] |
| `77030002` | `ErrInvalidPath` | The resolved path is unusable as a path — a non-directory used as a directory component, an over-long path, or a symlink loop. [[src/target/shared/code/error_constants.rs:ERR_INVALID_PATH_CODE]] |
| `77020001` | `ErrRead` | The host read of the opened file fails partway through. [[src/target/shared/code/error_constants.rs:ERR_READ_CODE]] |
| `77020002` | `ErrOutput` | The file cannot be opened for a host reason not classified above. [[src/target/shared/code/error_constants.rs:ERR_OUTPUT_CODE]] |
| `77010001` | `ErrOutOfMemory` | A path copy, the body byte list, the header map, or the `Response` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

A catch-all static route — the `*` capture supplies the relative path:

```
IMPORT http
IMPORT net
IMPORT collections

FUNC serveStatic(req AS http::Request) AS http::Response
  RETURN http::respondPath(req, "./public")
END FUNC

SUB main()
  MUT routes AS List OF http::Route = []
  routes = collections::append(routes, http::route("/static/*", serveStatic))
  RES s AS net::Listener = http::server(8080)
  DO
    http::handleRequest(s, routes)
  LOOP UNTIL FALSE
END SUB
```

Serve the site root, where an empty path resolves to `index.html`:

```
IMPORT http

FUNC home(req AS http::Request) AS http::Response
  RETURN http::respondPath(req, "./public")
END FUNC
```

Fall back to a custom page instead of the built-in `404` body:

```
IMPORT http

FUNC serveStatic(req AS http::Request) AS http::Response
  LET resp AS http::Response = http::respondPath(req, "./public")
  IF resp.status = 404 THEN
    RETURN http::status(404, "no such page")
  END IF
  RETURN resp
END FUNC
```

## See also

- `mfb man http respondFile`
- `mfb man http route`
- `mfb man http handleRequest`
- `mfb man http status`
- `mfb man fs isWithin`
- `mfb man fs openWithin`
