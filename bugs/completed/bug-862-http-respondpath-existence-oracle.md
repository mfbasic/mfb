# bug-862: `http::respondPath` answers 404 vs 403 for paths outside `root` — a file-existence oracle for the whole filesystem

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Security

Status: Resolved (2026-10-05)
Regression Test: tests/rt-behavior/http/func_http_respondPath_valid (extended: escape to a missing file is 403)

## Resolution

Fixed in `__http_respondPath` (`func_respond_path.rs`): a new lexical
`..`-escape check (`__http_pathEscapesRoot`) runs before `fs::fileExists`,
independent of what exists on disk, so a traversal attempt is 403 whether or
not its target exists. The existing `fs::isWithin` (realpath-based,
symlink-aware) check still runs after `fs::fileExists`, unchanged — it must,
since `isWithin` requires every path component including the file to exist,
and reordering it naively would have turned every in-root-missing-file 404
into a 403 (the Root Cause section's own caveat).

RED: `bug-862 RED: pin the respondPath existence oracle` (commit
`2a9523de7`) — both new cases failed `expected 403, got 404` against the
unfixed `fileExists`-first order.
GREEN: `bug-862: check confinement before existence in http::respondPath`
(commit `929d56a53`) — all 6 cases pass.


`respondPath` checks `fs::fileExists(candidate)` *before* it checks
`fs::isWithin(root, candidate)`. A traversal request to a path outside `root` therefore gets
`403 Forbidden` when the target exists and `404 Not Found` when it does not. A remote client
can probe for any file on the host (`/static/../../../../etc/hosts` gives 403,
`/static/../../../../etc/zzznope` gives 404) without reading it. The spec says every
escape is 403 and the confinement check happens first.

Correct behavior: a candidate that is not within `root` is `403` whether or not it exists.
`404` only for a missing file inside `root`.

References:

- src/docs/spec/stdlib/05_http.md §Constructors: "`respondPath` is path-traversal-safe:
  it canonicalizes the requested path and confines it to `root` (via `fs::isWithin`)
  *before* opening — any escape (`..`, absolute, symlink-out) yields `403`, a missing file
  yields `404`"

## Failing Reproduction

Server: `/tmp/hardening/B04/srv` (route `/static/*` → `http::respondPath(req, "./public")`,
run from `/tmp/hardening/B04/srv`, with `secret.txt` beside `public/`). Driven by raw sockets
from `/tmp/hardening/B04/py/rawc.py` / `rawc2.py trav`:

```
GET /static/../secret.txt HTTP/1.1            -> HTTP/1.1 403 Forbidden
GET /static/../nonexistent.txt HTTP/1.1       -> HTTP/1.1 404 Not Found
GET /static/../../../../../../../../etc/hosts HTTP/1.1     -> HTTP/1.1 403 Forbidden
GET /static/../../../../../../../../etc/zzznope HTTP/1.1   -> HTTP/1.1 404 Not Found
GET /static/%2e%2e/secret.txt HTTP/1.1        -> HTTP/1.1 403 Forbidden
```

Minimal server:

```
IMPORT http
IMPORT tcp
IMPORT collections

FUNC files(req AS http::Request) AS http::Response
  RETURN http::respondPath(req, "./public")
END FUNC

FUNC main() AS Nothing
  RES s AS tcp::Listener = http::server(18090, "127.0.0.1")
  MUT rs AS List OF http::Route = []
  rs = collections::append(rs, http::route("/static/*", files))
  DO
    http::handleRequest(s, rs)
  LOOP UNTIL FALSE
END FUNC
```

```
printf 'GET /static/../../../../../../../../etc/hosts HTTP/1.1\r\n\r\n' | nc 127.0.0.1 18090 | head -1
printf 'GET /static/../../../../../../../../etc/zzznope HTTP/1.1\r\n\r\n' | nc 127.0.0.1 18090 | head -1
```

- Observed: `HTTP/1.1 403 Forbidden` then `HTTP/1.1 404 Not Found`.
- Expected: `HTTP/1.1 403 Forbidden` for both.

Contrast: in-root files are served with the right content type (`/static/` → index.html,
`/static/sub/data.csv` → `text/csv`), and an in-root missing file (`/static/nope.html`) is
404. Both are correct.

## Root Cause

`src/codegen/builtins/http/func_respond_path.rs:__http_respondPath` runs, in this order:
`IF fs::fileExists(candidate) = FALSE THEN RETURN __http_status(404, …)`, then
`within = fs::isWithin(root, candidate)` → 403. The existence test runs on an
unconfined path. Swap the order: confinement first. `fs::isWithin` may need the path to
exist to canonicalize it (it is TRAPped to FALSE today). If so, confine lexically after
`..` resolution, or treat an `isWithin` failure as 403 before the existence probe.

## Goal

- Every candidate outside `root` → 403, whether or not it exists; the second probe above prints `403`.
- In-root missing → 404; in-root existing → 200 with the right content type.

### Non-goals (must NOT change)

- Symlink-out still 403; content-type table unchanged.

## Blast Radius

- `__http_respondPath`: fixed by this bug. `http::respondFile` takes an already-open handle and is unaffected.

## Golden impact

`__http_respondPath`'s body gained a new early lexical-escape check, and a new private
helper `__http_pathEscapesRoot` was added to the `http` package's assembled source.
Every fixture that `IMPORT http`s will show a changed `.ast`/`.ir`/`.ncode` for the same
reason as bug-981/bug-868 below — the helper section and one existing function body
changed text. Expect diffs under `tests/byte-identity/http/`, `tests/syntax/http/**`, and
any `rt-behavior`/`rt-error` `http` fixture with a golden. No other package's assembled
source changed for this bug.
