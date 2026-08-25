//! `http::respondFile` — descriptor entry (source-backed, body
//! `__http_respondFile`). Consumes the `RES fs::File` it serves (see
//! `syntaxcheck::builtins::http_consumes_argument`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Serve the whole contents of an open `File` as a `200` response, consuming the handle"#;

const DESC: &str = r#"`http::respondFile` reads every remaining byte of `file` into memory and returns
a new `http::Response` with `status` `200`, `reason` `"OK"`, `httpVersion`
`"1.1"`, a `headers` map holding the single entry `content-type`, `body` set to
the bytes read, and `ok` `TRUE`.

Unlike every other `http::` call, `respondFile` **consumes** its `File`: the
handle is moved into the call and is unusable afterward. Ownership passing to the callee is
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
whole.

`respondFile` is the low-level primitive. It does not look at any request, resolve
any path, or guess a content type from a filename — it only knows about the open
handle it is given. Most handlers should call `http::respondPath`, which resolves
a request path under a root directory, enforces containment, infers the content
type from the extension, and then calls this function."#;

const EX: &str = r#"Serve one known file with an explicit content type:

```
IMPORT fs
IMPORT http

FUNC page(req AS http::Request) AS http::Response
  RES f AS fs::File = fs::openFile("./public/page.html")
  RETURN http::respondFile(f, "text/html; charset=utf-8")
END FUNC
```

Serve a binary download, letting the content type default:

```
IMPORT fs
IMPORT http

FUNC download(req AS http::Request) AS http::Response
  RES f AS fs::File = fs::openFile("./data/report.bin")
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
  RES f AS fs::File = fs::openFile("./public/page.html")
  RETURN http::respondFile(f, "text/html; charset=utf-8")
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_respondFile(RES file AS fs::File, contentType AS String) AS Response
  LET data AS List OF Byte = fs::readAllBytes(file)
  MUT ct AS String = contentType
  IF ct = "" THEN
    ct = "application/octet-stream"
  END IF
  MUT h AS Map OF String TO String = Map OF String TO String {}
  h = collections::set(h, "content-type", ct)
  RETURN Response[200, "OK", "1.1", h, data, TRUE]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "respondFile",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File[, String]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("file", "An open `File` resource opened for reading, such as one from `fs::openFile`. Consumed by the call — the handle is moved, closed on return, and unusable afterward. Read starts at the handle's current position.", &[], ParameterType::named(super::FILE_TYPE)),
                super::fill("contentType", "The media type to advertise, stored under the header key `content-type`. Optional; omitted or `\"\"` means `application/octet-stream`. Stored verbatim, not validated.", ParameterType::String, ""),
            ],
            return_type: ParameterType::named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__http_respondFile"),
        }],
    });
}
