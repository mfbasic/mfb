# bug-866: `http` route `:name` binds an empty segment — `/t/:id` matches `/t//` with `id = ""`

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Coordinator note: repro measured by the review agent only; the coordinator did not re-run it.
Regression Test: tests/rt-behavior/http/func_http_route_valid (add empty-segment cases)

A `:name` route segment is specified to match "one non-empty segment", but the matcher
binds any segment, including the empty one a doubled `/` produces. `/t/:id` matches
`/t//` with `params["id"] = ""`, and `/m/:a/lit/:b` matches `/m//lit/b` with `a = ""`. A
handler that trusts the route to guarantee a non-empty id (for example to look up a record)
gets `""`.

Correct behavior: an empty path segment does not satisfy `:name` (no match, so the next
route is tried, or 404). `:name?` absent-vs-empty follows the same rule.

References:

- src/docs/spec/stdlib/05_http.md §Routing table: "`:name` | one non-empty segment | `params["name"]`"

## Failing Reproduction

Server `/tmp/hardening/B04/srv` (route `http::route("/t/:id", echo)`, `echo` prints `req.params`).
Raw requests from `/tmp/hardening/B04/py/rawc.py emptyseg emptyseg3`:

```
GET /t// HTTP/1.1          -> 200, body "...params=(id=)..."
GET /m//lit/b HTTP/1.1     -> 200, body "...params=(a=)(b=b)..."
```

Minimal: any `http::server` loop with `http::route("/t/:id", h)` and
`printf 'GET /t// HTTP/1.1\r\n\r\n' | nc 127.0.0.1 <port>`.

- Observed: `HTTP/1.1 200 OK` with `params=(id=)`.
- Expected: `HTTP/1.1 404 Not Found` (no other route matches).

Contrast: `/t/42/` → `id=42` (trailing slash normalized, as specified); `/m/a/lit/` → 404;
`/o` vs `/o/:x?` → match with no `x`; `/o/v/w` → 404. All correct.

## Root Cause

`src/codegen/builtins/http/helper_match_path.rs:__http_matchPath`, in the `:name` branch:
`IF ui < un THEN params = collections::set(params, …, collections::get(uSegs, ui))` with no
`<> ""` test. `__http_segments` splits on `/`, and `__http_normalizePath` strips only one
trailing slash, so `//` produces an empty segment. Require a non-empty segment for `:name`
(and for `:name?` to bind).

## Goal

- `/t//` and `/m//lit/b` → 404 against the routes above.

### Non-goals (must NOT change)

- `*` may still capture `""` (`/static` vs `/static/*` → `params["*"] = ""`).

## Blast Radius

- `__http_matchPath` only.
