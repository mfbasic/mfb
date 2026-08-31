//! `http::route` — descriptor entry (source-backed, body `__http_route`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Build a `http::Route` binding a validated path pattern to a request handler."#;

const DESC: &str = r#"`route` pairs a path `pattern` with a `handler` and returns a `http::Route` — a
two-field record holding exactly that `pattern` and `handler`.

It is a convenience over the record literal `http::Route[pattern, handler]`, and the
only difference is validation: `route` checks the pattern's shape at construction
and fails with `ErrInvalidArgument` on a malformed one, whereas the literal form
accepts anything. Nothing else is normalized or rewritten — the pattern is stored
verbatim.

**Segmentation.** A pattern is split into segments for both validation and
matching. A single trailing `/` is stripped first (except from the root `/`
itself), a leading `/` is dropped, and the remainder is split on `/`. The
patterns `/` and `""` both yield **zero** segments and so always validate.

**Segment kinds**, as interpreted later by the matcher:

- A **literal** segment must equal the request's path segment exactly.
- `:name` captures exactly one required segment into `params["name"]`.
- `:name?` captures one **optional** segment; when the request path has run out
  of segments the pattern segment is skipped and no key is added to `params`.
- `*` captures all remaining segments, rejoined with `/`, into `params["*"]`.
  When nothing remains it still binds, to the empty string.

**What validation actually rejects.** Only two shapes fail:

1. A segment equal to `*` that is not the last segment.
2. A segment ending in `?` that is followed by any segment not ending in `?`.

Everything else passes. In particular `route` does **not** reject an empty
capture name (`:`), duplicate capture names, an empty segment from a doubled
slash, or a segment such as `*x` that merely contains `*` — `*x` is validated and
matched as a plain literal, since only the exact string `*` is special.

Note that the optional-segment rule keys off the trailing `?` alone, without
requiring a leading `:`. A literal segment that happens to end in `?` therefore
counts as optional **for validation** — forcing every following segment to also
end in `?` — while the matcher still treats it as a required literal, because
matching requires both a leading `:` and a trailing `?`. Avoid literal segments
ending in `?`.

**Ordering.** Routes live in an ordered `List OF http::Route` and `http::handleRequest`
tries them in list order, first match wins. `route` imposes no specificity
ranking, so to let a literal beat an overlapping pattern, append it first.

`route` performs no I/O and registers nothing globally; it is a pure constructor
whose result the caller stores."#;

const EX: &str = r#"A route table, literal before pattern so `/foo/new` is not swallowed by `:bar`:

```
IMPORT http
IMPORT collections

FUNC newFoo(req AS http::Request) AS http::Response
  RETURN http::ok("new foo")
END FUNC

FUNC showFoo(req AS http::Request) AS http::Response
  RETURN http::ok("foo " & collections::getOr(req.params, "bar", "(none)"))
END FUNC

FUNC serveStatic(req AS http::Request) AS http::Response
  RETURN http::ok("static " & collections::getOr(req.params, "*", ""))
END FUNC

SUB main()
  MUT routes AS List OF http::Route = []
  routes = collections::append(routes, http::route("/foo/new", newFoo))
  routes = collections::append(routes, http::route("/foo/:bar?", showFoo))
  routes = collections::append(routes, http::route("/static/*", serveStatic))
END SUB
```

Catching a rejected pattern — the wildcard is not the final segment. This prints
`rejected: wildcard '*' must be the final segment`:

```
IMPORT http
IMPORT io

FUNC home(req AS http::Request) AS http::Response
  RETURN http::ok("hi")
END FUNC

SUB register(p AS String)
  LET r AS http::Route = http::route(p, home)
  io::print("ok: " & r.pattern)
  EXIT SUB
TRAP (e)
  io::print("rejected: " & e.message)
  EXIT SUB
END TRAP
END SUB

SUB main()
  register("/a/*/b")
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_route(pattern AS String, handler AS FUNC(Request) AS Response) AS Route
  __http_validatePattern(pattern)
  RETURN Route[pattern, handler]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "route",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, FUNC(Request) AS Response"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("pattern", "Path pattern, segments separated by `/`. A leading `/` and one trailing `/` are insignificant. Segments may be literals, `:name`, a trailing `:name?`, or a final `*`. Stored verbatim on the returned `http::Route`.", &[], ParameterType::String),
                // The handler is the STRUCTURED function type `FUNC(Request) AS
                // Response` (parsed, not a `Named` blob): the matcher compares it
                // element-wise, so a wrong-shaped handler (`FUNC(Integer) AS Integer`)
                // is rejected — a `Named("FUNC(…)")` blob would match coarsely and let
                // it through.
                super::req("handler", "The function invoked when this route matches. The type is exact — a function of any other signature is a compile-time mismatch, not a runtime error.", &[], super::handler_type()),
            ],
            return_type: ParameterType::named(super::ROUTE_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__http_route"),
        }],
    });
}
