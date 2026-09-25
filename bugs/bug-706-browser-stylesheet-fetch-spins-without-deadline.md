# bug-706: `fetchStyles` busy-spins forever on a stalled stylesheet, and `resolveLocation` misresolves query/fragment/`../` hrefs

Last updated: 2026-09-25
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: `tools/browser-fetch-probe/` — a standalone MFBASIC driver (added in Phase 1) that
imports the built `fetch` package directly and (a) drives `fetchStyles`/`fetch` against a
"black hole" stylesheet server with a wall-clock budget, and (b) calls `fetch::resolveHref`
with the four inputs below and asserts the resolved strings.

`examples/browser/fetch/src/lib.mfb` has two independent bugs in its network layer, both
found while auditing the `fetch` package for correctness. Together they mean the browser
example can hang indefinitely pegging a CPU core on an ordinary slow/stalled site, and can
silently mis-navigate or fail to navigate a same-origin link whose href contains a query
string, a fragment, or a `../` segment.

**Sub-issue 1 — unbounded busy-spin (`fetchStyles`).** `fetchStyles`'s drive loop polls
`http::ready`/`http::pump` in a tight `DO WHILE` with no sleep and no elapsed-time check. Per
`mfb man http startRead`, "the per-read deadline is a matter for the drive loop" — `http::`
does not enforce one itself. Because `fetchStyles` never checks one, a single unresponsive
stylesheet server (accepts the TCP connection, then never writes and never closes) makes the
loop spin at full speed forever: the page never finishes loading and the worker thread never
yields. **Correct behavior:** the drive loop tracks elapsed time (or a caller-supplied budget)
per stream and stops waiting on it — treating it the same as a stream whose connection failed
(skipped, matching a page that renders without that sheet) — once a deadline elapses, and,
independently of any deadline, must not busy-spin (some backoff/yield between unproductive
polls) while waiting.

**Sub-issue 2 — `resolveLocation` misresolves three href shapes.** `resolveLocation`
(reached from the app through the exported `fetch::resolveHref`) tests
`strings::contains(target, "://")` against the *whole* href to decide "is this absolute", and
falls through to "resolve against the *directory* of the current path" for anything else —
with no query/fragment special case and no dot-segment normalization. Three inputs come out
wrong:

1. A same-origin relative href that merely contains `"://"` inside its query string (e.g. a
   `?next=http://...` redirect parameter) is treated as already-absolute and returned
   completely unresolved — not even prefixed with a scheme — producing a string
   `net::toUrl` rejects outright.
2. A query-only href (`"?new=2"`) or fragment-only href (`"#section"`) resolves against the
   *directory* of the current path, silently dropping the current page's filename (and, for
   the query case, its existing query) instead of keeping the current document and only
   swapping in the new query/fragment.
3. A `"../"`-relative href is concatenated onto the current directory without ever
   normalizing the `..` segment out, leaving a literal `..` in the returned URL.

**Correct behavior:** `resolveLocation`/`resolveHref` follow RFC 3986 §5.3 merge + remove
dot-segments: only a href with a *leading* scheme (`scheme:` before any `/`, `?`, or `#`) is
absolute; a query-only or fragment-only href resolves against the base's full path (keeping
its last segment); and any `.`/`..` segment in the merged path is removed before the result is
returned.

References:

- `mfb man http startRead` / `mfb man http ready` / `mfb man http pump` — the non-blocking
  drive contract; "startRead applies the 30-second connect deadline; the per-read deadline is
  a matter for the drive loop."
- `examples/browser/README.md` "CSS" section — describes `fetchStyles`'s cooperative
  multi-stream drive loop, silent on any deadline.
- RFC 3986 §5.3 (Component Recomposition) / §5.2.4 (Remove Dot Segments) — the reference
  algorithm `resolveLocation` is meant to approximate.

## Failing Reproduction

### Sub-issue 1 — unbounded spin

Built `dom` and `fetch` into a scratch copy under `/tmp/browser-repro/` (README's install
order, dom first) and a standalone driver `/tmp/browser-repro/probe` that imports the built
`fetch` package directly:

```basic
IMPORT fetch
IMPORT thread
IMPORT io

SUB main()
  io::print("starting fetch")
  RES t AS Thread OF String TO fetch::LoadResult = thread::start(fetch::fetch, "http://127.0.0.1:8900/index.html")
  LET r AS fetch::LoadResult = thread::waitFor(t)
  io::print("done ok=" & toString(r.ok) & " title=" & r.title)
END SUB
```

Two local servers: a plain `python3 -m http.server 8900` serving one page,

```html
<html><head><title>Spin Test</title>
<link rel="stylesheet" href="http://127.0.0.1:8901/blackhole.css">
</head><body><h1>hi</h1></body></html>
```

and a raw-socket "black hole" on 8901 that `accept()`s and then writes nothing and never
closes:

```python
import socket
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", 8901)); s.listen(5)
while True:
    conn, addr = s.accept()  # never write, never close
```

Running `probe.out` and polling it:

```
$ /usr/bin/time -l ./build/probe.out &
$ ps -o pid,etime,time,pcpu,stat -p <pid>
  PID ELAPSED      TIME  %CPU STAT
44898   01:07   0:02.50   3.1 RN
```

- Observed: the process is still `STAT=R` (runnable, not blocked in a syscall) 67 seconds in,
  `starting fetch` is the only line ever printed, and its accumulated CPU time keeps climbing
  the whole time — proof it is actively looping, not sleeping. (The machine this was measured
  on carried a load average around 147 from other concurrent sessions, which dilutes the
  reported `%CPU`; on an idle core the same loop pegs the core near 100%, as the loop body —
  `http::ready`/`http::pump` on a stream that's never ready and never `http::done` — does no
  syscall that blocks and nothing that sleeps between iterations.) Left running past 30s with
  no deadline of any kind; killed manually.
- Expected: `fetchStyles` gives up on the stalled sheet after a bounded wait (matching how it
  already treats a sheet that fails to connect — "simply skipped, matching a page that renders
  without it") and `fetch::fetch` returns a `LoadResult` within a bounded time, and the drive
  loop does not spin the CPU while waiting either way.

Contrast case that works correctly today: a stylesheet URL that fails to *connect* at all
(bad host) is caught by the `TRAP`/`CONTINUE FOR` around `http::startRead` in the same
function and correctly skipped — the bug is specific to a connection that succeeds and then
stalls mid-exchange, which the loop has no way to give up on.

| Environment | config | Result |
| --- | --- | --- |
| macOS aarch64 (dev machine) | `probe.out` vs. local black-hole server on 8901 | spins ✗ (never completes, `STAT=R`, CPU time still climbing at 67s) |

### Sub-issue 2 — `resolveLocation` misresolution

Same standalone driver, calling the exported `fetch::resolveHref` directly (it is a thin
`TRAP`-wrapped call onto private `resolveLocation`, so it is callable without the TUI):

```basic
IMPORT fetch
IMPORT io

SUB main()
  LET base AS String = "http://example.com/a/b/page.html?old=1"
  io::print("a) " & fetch::resolveHref(base, "/redirect?next=http://evil.example/x"))
  io::print("a2) " & fetch::resolveHref(base, "search?next=http://evil.example/x"))
  io::print("b1) " & fetch::resolveHref(base, "?new=2"))
  io::print("b2) " & fetch::resolveHref(base, "#section"))
  io::print("c) " & fetch::resolveHref(base, "../up.html"))
END SUB
```

Observed output:

```
a) /redirect?next=http://evil.example/x
a2) search?next=http://evil.example/x
b1) http://example.com:80/a/b/?new=2
b2) http://example.com:80/a/b/#section
c) http://example.com:80/a/b/../up.html
```

- Observed (a2 — the clean same-origin-relative case, no leading `/`): `resolveHref` returns
  the href completely untouched — `search?next=http://evil.example/x` — because
  `strings::contains(target, "://")` matches inside the query string. Feeding that string to
  `net::toUrl` (as the app's link-follow path in `examples/browser/app/src/main.mfb:900,1023`
  eventually does) fails outright:
  ```
  toUrl FAILED: unsupported URL scheme
  ```
  (confirmed by calling `net::toUrl("search?next=http://evil.example/x")` directly in the
  same driver). A same-origin link with such a query string cannot be followed at all.
- Observed (b1, query-only): `http://example.com:80/a/b/?new=2` — drops `page.html`, replacing
  the whole file with the bare directory plus the new query.
- Observed (b2, fragment-only): `http://example.com:80/a/b/#section` — same defect, drops
  `page.html` (and the existing `?old=1` query) for the fragment case.
- Observed (c, `../`): `http://example.com:80/a/b/../up.html` — the literal `..` segment
  survives into the returned URL unnormalized.
- Expected (RFC 3986 §5.3 merge, `remove_dot_segments`):
  - a2: `http://example.com/search?next=http://evil.example/x` (resolved against the base,
    absolute and toUrl-parseable, since the href has no leading scheme)
  - b1: `http://example.com/a/b/page.html?new=2`
  - b2: `http://example.com/a/b/page.html?old=1#section`
  - c: `http://example.com/a/up.html`

Contrast cases that work correctly today: `a)` (`/redirect?next=http://evil.example/x`) is a
root-relative href that also happens to contain `"://"` in its query — it hits the same
`"://"`-anywhere branch and is *also* returned unresolved (`/redirect?next=...`, missing
scheme+host), so it is not actually a "works today" case; it is the same defect as a2, just
with a leading `/`. A href with no `"://"`, no leading `/` or `?`/`#`, and no `..` (e.g.
`"other.html"`) resolves correctly today via the existing directory-substitution path — that
is the one shape the function was written for.

| Environment | config | Result |
| --- | --- | --- |
| macOS aarch64 (dev machine) | `fetch::resolveHref` direct call, 5 inputs above | 4 of 5 wrong ✗ (all but the plain-filename case) |

## Root Cause

**Sub-issue 1** — `examples/browser/fetch/src/lib.mfb:fetchStyles`, lines 152–161:

```basic
MUT pending AS Boolean = TRUE
DO WHILE pending
  pending = FALSE
  FOR EACH s IN streams
    IF http::done(s) = FALSE THEN
      IF http::ready(s) THEN http::pump(s)
      pending = TRUE
    END IF
  NEXT
LOOP
```

`http::ready` is documented as "a pure readiness probe with a zero timeout — it never blocks";
`http::pump` "is internally gated on readiness... when no bytes are available it returns
immediately having done nothing." Neither call ever blocks or sleeps, and per `mfb man http
startRead`, "the per-read deadline is a matter for the drive loop" — the API deliberately does
not impose one. This loop sets `pending = TRUE` for any not-yet-`http::done` stream regardless
of whether that iteration made progress, and re-enters the outer `DO WHILE` immediately with no
sleep and no check of how long any given stream has been outstanding. A stream whose peer
never sends bytes and never closes is never `http::done`, so `pending` never goes false, and
the loop consumes the calling (worker) thread at 100% CPU indefinitely with no way out.

**Sub-issue 2** — `examples/browser/fetch/src/lib.mfb:resolveLocation`, lines 64–81:

```basic
FUNC resolveLocation(base AS net::Url, loc AS String) AS String
  LET target AS String = strings::trim(loc)
  IF strings::contains(target, "://") THEN RETURN target
  LET authority AS String = base.scheme & "://" & base.host & ":" & toString(base.port)
  IF strings::startsWith(target, "//") THEN
    RETURN base.scheme & ":" & target
  ELSEIF strings::startsWith(target, "/") THEN
    RETURN authority & target
  END IF
  MUT dir AS String = base.path
  LET slash AS Integer = lastSlash(dir)
  IF slash >= 0 THEN
    dir = strings::mid(dir, 0, slash + 1)
  ELSE
    dir = "/"
  END IF
  RETURN authority & dir & target
END FUNC
```

- Line 66: `strings::contains(target, "://")` scans the *entire* href for the substring
  anywhere, not just for a leading `scheme:` before the first `/`/`?`/`#`. A relative href
  whose query string happens to contain `"://"` (a very common shape — an OAuth `redirect_uri`
  or "next page" query parameter) matches this and is returned completely as typed on line 66
  (`RETURN target`), never even reaching the `authority &` prefixing that every other branch
  applies — so it is not merely "wrongly treated as absolute," it comes out with no scheme or
  host at all, and downstream `net::toUrl` (see `resolveHref`'s callers,
  `examples/browser/app/src/main.mfb:900,1023`) rejects it as an unsupported/missing scheme.
- Lines 73–80: once neither `"://"` nor a leading `/` matches, the function always resolves
  against `dir` — `base.path` with its last `/`-segment stripped, i.e. the *directory*, never
  the full path. There is no check for `target` starting with `?` or `#`. A query-only or
  fragment-only href therefore loses the current document's filename (`page.html`) and, for
  the query case, its existing query — the function has no representation of "same document,
  different query/fragment."
- Lines 73–80 also never scan `target` (or the concatenated `dir & target`) for `.`/`..`
  segments — there is no dot-segment removal step at all, so a `../` in the href passes
  straight through into the returned string.

`lastSlash` (lines 51–60) is a straightforward linear scan and is not itself at fault; the bug
is entirely in what `resolveLocation` does with the pieces it computes.

## Goal

- `fetchStyles`'s drive loop enforces a bounded overall wait per stream (or per the whole
  batch) and gives up on a stream that exceeds it exactly the way it already gives up on a
  stream that fails to connect — the sheet is skipped, `loaded`/`texts` do not count it, and
  the page still renders. The loop does not busy-spin the CPU between unproductive polls
  either.
- `resolveLocation`/`resolveHref` resolve a href with a leading `scheme:` as absolute; resolve
  a href with no leading scheme against `base` per RFC 3986 §5.3, correctly handling `//`,
  `/`, `?`-only, `#`-only, and a plain relative path; and remove `.`/`..` segments from the
  merged path before returning.

### Non-goals (must NOT change)

- `fetchStyles`'s concurrent multi-stream drive design itself (one pass per round, only
  pumping ready streams) — only its lack of a deadline/backoff.
- The plain relative-path case (`"other.html"` against `.../a/b/page.html`) and the
  already-correct `//`/`/`-prefixed absolute-ish cases, which resolve correctly today and must
  keep doing so.
- `sheetLoaded`'s Content-Length/200 accounting, and `http::`'s own contract (its 30-second
  connect deadline, its documented no-per-read-deadline design) — this bug is entirely in the
  example's caller-side drive loop and URL resolution, not in `http::`.
- Do not "fix" this by editing the man pages to claim `http::` enforces a read deadline itself
  — it deliberately does not (`mfb man http startRead`); the fix belongs in `fetchStyles`.

## Blast Radius

- `examples/browser/fetch/src/lib.mfb:fetchStyles` — fixed by this bug (sub-issue 1).
- `examples/browser/fetch/src/lib.mfb:resolveLocation` / `EXPORT FUNC resolveHref` — fixed by
  this bug (sub-issue 2).
- `examples/browser/fetch/src/lib.mfb:fetch` (the `EXPORT ISOLATED FUNC` entry point) — calls
  `resolveLocation` directly at line 227 to follow a redirect `Location:` header. Same
  misresolution risk if a server redirects to a query-only/fragment-only/`../`-relative
  `Location`, though that is unusual for a `Location` header (which is normally a full or
  root-relative URL) — in scope for the same fix since it is the same function, not a separate
  repro.
- `examples/browser/app/src/main.mfb:900` and `:1023` (`fetch::resolveHref` calls for link
  navigation) — fixed by this bug; these are the only two call sites reaching `resolveHref`.
- `grep -rln "startRead\|http::ready\|http::pump\|http::done" examples/` returns only
  `examples/browser/fetch/src/lib.mfb` — no sibling non-blocking drive loop elsewhere in
  `examples/` shares this hazard.
- `examples/network-client/src/main.mfb:126,169,229`, `examples/network-server/src/main.mfb:163,255,319`,
  `examples/network-server/src/wire.mfb:117,197`, `examples/ai_chat/src/main.mfb:152,172,1021,1081`
  — all poll with an explicit blocking timeout (`tcp::poll(sock, ms)` /
  `tls::poll(sock, ms)` / `udp::poll(sock, ms)` / `process::poll(p, 0)` guarded by an iteration
  cap at `ai_chat/src/main.mfb:1041`), which blocks for up to `ms` per call rather than
  busy-spinning with a zero-timeout probe — unaffected, different (already-bounded) pattern.
- `examples/wind/src/fetch.mfb:93` — builds a fixed `https://` URL by string concatenation,
  no href resolution logic — unaffected.
- No other href/URL-relative-resolution logic exists elsewhere in `examples/` (`dom` package
  has none; `grep -rln '://' examples/` returns only `wind/src/fetch.mfb`,
  `browser/app/src/main.mfb`, and `browser/fetch/src/lib.mfb`, the latter two already covered
  above).

## Fix Design

**Sub-issue 1.** Track a start time (or per-stream deadline) when `fetchStyles` begins driving
the batch (`net::now`/`time::` equivalent — whichever the codebase's timing primitive is).
Each round, drop a stream whose elapsed time exceeds the budget from `streams` (or mark it
failed) the same way a connect failure already causes it to be left out via
`CONTINUE FOR`, so it is skipped by the later `http::finish` pass rather than driven forever.
Independently, avoid a hot spin on rounds where no stream was ready — a short sleep between
unproductive rounds — since even a bounded wait should not peg a CPU core for its duration.
Rejected alternative: make `http::` itself enforce a read deadline — rejected because the man
page states this is deliberately the caller's responsibility, and doing so would change
`http::`'s public contract for every caller, not just this example.

**Sub-issue 2.** Reimplement `resolveLocation` to test for a *leading* scheme (a run of
scheme characters followed by `:` before any `/`, `?`, `#` — not `strings::contains` anywhere
in the string), add explicit `?`/`#`-leading branches that resolve against `base`'s full path
(not its directory), and run a dot-segment removal pass over the final merged path per RFC
3986 §5.2.4 before returning. Rejected alternative: patch only the reported symptom (special-
case `"://"` to require it appear before the first `/`) without adding dot-segment removal or
the query/fragment branches — rejected because it would leave two of the three confirmed
defects unfixed under the same root cause (this function approximates URL resolution without
following the reference algorithm).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tools/browser-fetch-probe/` (a README'd standalone MFBASIC project + the two Python
      local servers it needs, or an equivalent script) that reproduces both sub-issues on
      demand and fails (non-zero exit / explicit assertion mismatch) against current behavior:
      a bounded-time run of `fetch::fetch`/`fetchStyles` against a black-hole stylesheet server
      that must complete within a stated budget, and the five `resolveHref` inputs above
      compared against their RFC-3986-correct expected strings.
- [ ] Confirm the blast-radius audit above (already completed in this document) stays accurate
      once the tool exists — re-run its greps.

Acceptance: the new test(s) fail for the documented reason (spin exceeds budget; 4 of 5
`resolveHref` cases mismatch); the audit list above is complete with a verdict per site.
Commit: —

### Phase 2 — the fix

- [ ] Add a deadline (and a between-round sleep) to `fetchStyles`'s drive loop in
      `examples/browser/fetch/src/lib.mfb`, skipping an expired stream the same way a
      connect failure is skipped today.
- [ ] Rewrite `resolveLocation` in the same file: leading-scheme detection, `?`/`#`-leading
      branches against the base's full path, and dot-segment removal on the merged path.

Acceptance: the Phase 1 tool reports the spin bounded and all 5 `resolveHref` cases correct;
the contrast cases (connect-failure skip; plain relative-path resolution) still behave as
documented; nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Re-run the Phase 1 tool end-to-end against fresh local servers; confirm the bounded-wait
      number and all 5 resolution strings.
- [ ] Manually smoke-test the full `browser` app (per its README's build steps) against a page
      whose stylesheet stalls, and against a page with a query-only/fragment-only/`../` link,
      confirming the UI no longer hangs and the link navigates correctly.
- [ ] Run the project's full test/CI suite (this is an `examples/` change with no compiler
      surface touched, so no golden/spec regeneration is expected).

Acceptance: full suite green; the Phase 1 tool passes; the app smoke test confirms both fixes
end-to-end.
Commit: —

## Validation Plan

- Regression test(s): `tools/browser-fetch-probe/` (Phase 1), covering both sub-issues.
- Runtime proof: the two repros in this document (black-hole-server spin; the 5-input
  `resolveHref` table), re-run after the fix.
- Doc sync: `examples/browser/README.md`'s "CSS" section should note the deadline once added;
  none of the compiler's `mfb spec`/`mfb man` surfaces are affected (this bug is entirely in
  example code, not in `http::` or `net::`).
- Full suite: the project's standard `cargo test` (no compiler code changes expected, but the
  example is exercised by whatever example-build check the suite already runs, if any).

## Open Decisions

- Exact deadline/backoff mechanism and budget for `fetchStyles` (e.g. a fixed N-second cap
  matching `http::startRead`'s own 30-second connect deadline, vs. a caller-configurable
  parameter) — recommend matching the existing 30-second connect deadline for consistency,
  vs. a shorter budget since a stylesheet fetch happens after the page's own successful
  connect. (§Fix Design, sub-issue 1)
- Whether a dot-segment-removal helper belongs in `fetch` locally or should be promoted to
  `net::` as a reusable utility, since any future example doing URL resolution would want the
  same algorithm — recommend keeping it local to `fetch` for this bug (no second consumer
  exists yet per the blast-radius grep) vs. promoting it now. (§Fix Design, sub-issue 2)

## Summary

Both defects sit entirely in `examples/browser/fetch/src/lib.mfb`'s example-side logic, not in
the `http::`/`net::` builtins, which behave exactly as documented. The engineering risk is
concentrated in `resolveLocation`'s rewrite (three interacting cases plus the one currently-
correct case that must not regress) and in choosing a deadline value for `fetchStyles` that
does not make an ordinarily-slow-but-live site time out spuriously; the busy-spin fix itself
(a deadline check plus a short sleep) is mechanically simple once the budget is decided.
