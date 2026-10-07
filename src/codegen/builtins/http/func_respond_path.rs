//! `http::respondPath` — descriptor entry (source-backed, body
//! `__http_respondPath`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Serve a request's path as a static file from under a root directory"#;

const DESC: &str = r#"`http::respondPath` turns a request into a static-file response: it derives a
relative path from `req`, resolves it under `root`, checks that the result really
is inside `root`, infers a content type from the file extension, and serves the
file with `http::respondFile`. It is the whole of the built-in static-file
handler.

The relative path is taken from `req.params["*"]` when the matched route captured
a wildcard remainder, and from `req.path` otherwise. One leading `/` is stripped,
and a path that is then empty becomes `index.html`. The result is joined to `root`
with `fs::pathJoin`.

The steps then run **in this order**, and the order is observable:

1. A purely **lexical** check rejects a `..` segment in the relative
   path that would climb back above `root` — before anything touches the
   filesystem, so it cannot act as an existence oracle. A path like
   `../secret.txt` or `../../../../etc/hosts` is `403` whether or not the
   named file exists; a request cannot distinguish "escaped and found" from
   "escaped and not found" by timing or status code.
2. If `fs::fileExists` reports the joined candidate is not an existing regular
   file, a `404` is returned. Directories are not regular files, so a request for
   a directory yields `404`; there is no directory listing and no implicit
   `index.html` inside a subdirectory.
3. Otherwise `fs::isWithin(root, candidate)` decides containment. If it reports
   the candidate is not inside `root`, a `403` is returned and the file is never
   opened. An error raised by `isWithin` itself is trapped and treated as *not
   contained*, so it also yields `403`.

Every escaping path is `403` regardless of existence: step 1 catches a lexical
`..` escape before the existence probe ever runs, and step 3 catches a symlink
that points out of `root` for a candidate that happens to exist (which step 1,
being lexical, cannot see). Only a path that is genuinely inside `root` and
missing reaches `404`. Both responses are built with `http::status`, so each
carries a plain-text body (`"Not Found"` / `"Forbidden"`), `content-type`
`text/plain; charset=utf-8`, and `ok` `FALSE`.

Step 3 is where the symlink defense lives, and it is worth being precise about
what it does and does not guarantee. `fs::isWithin` canonicalizes both paths
with the host `realpath` resolution — collapsing `..`, following every
symbolic link, and resolving relative paths against the working directory —
then compares at a separator boundary. It requires every component of the
candidate, including the file itself, to exist (`mfb man fs isWithin`), which
is exactly why it must run AFTER the existence check: calling it on a
genuinely missing in-root file would raise `ErrPathNotFound`, trapped here to
*not contained*, turning a legitimate `404` into a `403`. The lexical check in
step 1 has no such requirement, which is why it runs first and independently.

However, `respondPath` **checks and then opens**, using `fs::openFile` rather
than the atomic `fs::openWithin`. That leaves the time-of-check/time-of-use race
inherent to any check-then-open: a component of the path can be replaced with a
symlink after `isWithin` returns and before the open happens. Under a threat model
where an attacker can create symlinks inside `root`, this is not an airtight
confinement boundary.

The content type is inferred from the lowercased text after the final `.`, and
only when that dot comes after the final `/`, so an extensionless name or a dot
that belongs to a directory component is not treated as an extension. The
recognized extensions are `html`/`htm`, `css`, `js`/`mjs`, `json`, `txt`/`text`,
`xml`, `csv`, `png`, `jpg`/`jpeg`, `gif`, `svg`, `ico`, `webp`, `woff`, `woff2`,
`ttf`, `pdf`, and `wasm`. Anything else, including no extension at all, is served
as `application/octet-stream`.

The whole file is buffered into the response body, exactly as in
`http::respondFile`; there is no streaming and no range support. Serving a large
file occupies the single-threaded server for the duration of the read.

A missing or inaccessible file is reported as a `404`/`403` response, not a
raised error (see above). `respondPath` raises `ErrInvalidArgument` from
joining or checking the path, `ErrAccessDenied`/`ErrInvalidPath`/
`ErrPathNotFound`/`ErrReadFailed`/`ErrWriteFailed` from opening the file once
`fs::fileExists` has already reported it exists (a race between the check and
the open), and `ErrResourceClosed`/`ErrResourceMoved` from the same race
against the open handle inside `http::respondFile`. `ErrOutOfMemory` means
the program ran out of memory."#;

const EX: &str = r#"A catch-all static route — the `*` capture supplies the relative path:

```
IMPORT http
IMPORT tcp
IMPORT collections

FUNC serveStatic(req AS http::Request) AS http::Response
  RETURN http::respondPath(req, "./public")
END FUNC

FUNC main() AS Nothing
  MUT routes AS List OF http::Route = []
  routes = collections::append(routes, http::route("/static/*", serveStatic))
  RES s AS tcp::Listener = http::server(8080)
  DO
    http::handleRequest(s, routes)
  LOOP UNTIL FALSE
END FUNC
```

Serve the site root, where an empty path resolves to `index.html`:

```
IMPORT http
IMPORT tcp
IMPORT collections

FUNC home(req AS http::Request) AS http::Response
  RETURN http::respondPath(req, "./public")
END FUNC

FUNC main() AS Nothing
  MUT routes AS List OF http::Route = []
  routes = collections::append(routes, http::route("/", home))
  RES s AS tcp::Listener = http::server(8080)
  DO
    http::handleRequest(s, routes)
  LOOP UNTIL FALSE
END FUNC
```

Fall back to a custom page instead of the built-in `404` body:

```
IMPORT http
IMPORT tcp
IMPORT collections

FUNC serveStatic(req AS http::Request) AS http::Response
  LET resp AS http::Response = http::respondPath(req, "./public")
  IF resp.status = 404 THEN
    RETURN http::status(404, "no such page")
  END IF
  RETURN resp
END FUNC

FUNC main() AS Nothing
  MUT routes AS List OF http::Route = []
  routes = collections::append(routes, http::route("/static/*", serveStatic))
  RES s AS tcp::Listener = http::server(8080)
  DO
    http::handleRequest(s, routes)
  LOOP UNTIL FALSE
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_respondPath(req AS Request, root AS String) AS Response
  MUT rel AS String = ""
  IF collections::hasKey(req.params, "*") THEN
    rel = collections::getOr(req.params, "*", "")
  ELSE
    rel = req.path
  END IF
  IF strings::startsWith(rel, "/") THEN
    rel = __http_slice(rel, 1, len(rel))
  END IF
  IF rel = "" THEN
    rel = "index.html"
  END IF
  ' bug-862: reject a `..` escape lexically, before any filesystem probe, so
  ' a traversal attempt is 403 whether or not its target exists. Without
  ' this, fs::fileExists below decided 404-vs-403 first, letting a remote
  ' client use the existence oracle to probe for any file on the host.
  IF __http_pathEscapesRoot(rel) THEN
    RETURN __http_status(403, "Forbidden")
  END IF
  LET parts AS List OF String = [root, rel]
  LET candidate AS String = fs::pathJoin(parts)
  IF fs::fileExists(candidate) = FALSE THEN
    RETURN __http_status(404, "Not Found")
  END IF
  MUT within AS Boolean = FALSE
  within = fs::isWithin(root, candidate) TRAP(e)
    RECOVER FALSE
  END TRAP
  IF within = FALSE THEN
    RETURN __http_status(403, "Forbidden")
  END IF
  LET ct AS String = __http_extContentType(candidate)
  RES f AS fs::File = fs::openFile(candidate)
  RETURN __http_respondFile(f, ct)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "respondPath",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Request, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("req", "The request to serve. Only `params[\"*\"]` and `path` are read. Also accepted under the name `request`.",
                    &["request"],
                    ParameterType::named(super::REQUEST_TYPE),
                ),
                super::req("root", "The directory that files are served from and confined to. Interpreted by `fs::pathJoin` and `fs::isWithin`; may be absolute or relative to the working directory, and must exist for the containment check to succeed.", &[], ParameterType::String),
            ],
            return_type: ParameterType::named(super::RESPONSE_TYPE),
            errors: vec!["ErrAccessDenied", "ErrInvalidArgument", "ErrInvalidPath", "ErrOutOfMemory", "ErrPathNotFound", "ErrReadFailed", "ErrResourceClosed", "ErrResourceMoved", "ErrWriteFailed"],
            body: Body::mfb(BODY, "__http_respondPath"),
        }],
    });
}
