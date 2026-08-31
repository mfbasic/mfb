//! `http::respondFile` — descriptor entry (source-backed, body
//! `__http_respondFile`). Consumes the `RES fs::File` it serves (see
//! the former source checker's `builtins::http_consumes_argument`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Serve the whole contents of an open `File` as a `200` response, closing the handle"#;

const DESC: &str = r#"`http::respondFile` reads every remaining byte of `file` into memory and returns
a new `http::Response` with `status` `200`, `reason` `"OK"`, `httpVersion`
`"1.1"`, a `headers` map holding the single entry `content-type`, `body` set to
the bytes read, and `ok` `TRUE`.

Unlike every other `http::` call, `respondFile` **closes** its `File`: the call
takes the handle, and it cannot be used again afterwards. That is what makes it
safe — the `File` is closed when `respondFile` returns, and on the failure path
too, so a read error cannot leave the file open. Do not close or reuse the
handle yourself.

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
IMPORT io

SUB main()
  fs::writeText("/tmp/page.html", "<h1>hi</h1>")
  RES f AS fs::File = fs::openFile("/tmp/page.html")
  LET resp AS http::Response = http::respondFile(f, "text/html; charset=utf-8")
  io::print(toString(resp.status) & " " & toString(len(resp.body)) & " bytes")
END SUB
```

prints:

```
200 11 bytes
```

Serve a binary download, letting the content type default:

```
IMPORT fs
IMPORT http
IMPORT io

SUB main()
  fs::writeText("/tmp/report.bin", "raw bytes")
  RES f AS fs::File = fs::openFile("/tmp/report.bin")
  LET resp AS http::Response = http::respondFile(f)
  io::print(toString(resp.status) & " " & toString(len(resp.body)) & " bytes")
END SUB
```

prints:

```
200 9 bytes
```

Turn a missing file into a `404` rather than an error:

```
IMPORT fs
IMPORT http
IMPORT io

SUB main()
  IF fs::fileExists("/tmp/does-not-exist.html") = FALSE THEN
    LET missing AS http::Response = http::status(404, "Not Found")
    io::print(toString(missing.status) & " ok=" & toString(missing.ok))
    EXIT SUB
  END IF
  RES f AS fs::File = fs::openFile("/tmp/does-not-exist.html")
  LET resp AS http::Response = http::respondFile(f, "text/html; charset=utf-8")
  io::print(toString(resp.status))
END SUB
```

prints:

```
404 ok=FALSE
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
                super::req("file", "An open `File` resource opened for reading, such as one from `fs::openFile`. Closed by this call; the handle cannot be used again. Read starts at the handle's current position.", &[], ParameterType::named(super::FILE_TYPE)),
                super::fill("contentType", "The media type to advertise, stored under the header key `content-type`. Optional; omitted or `\"\"` means `application/octet-stream`. Stored verbatim, not validated.", ParameterType::String, ""),
            ],
            return_type: ParameterType::named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__http_respondFile"),
        }],
    });
}
