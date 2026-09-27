# tools/browser-fetch-probe

Regression check for bug-706: `examples/browser/fetch`'s `fetchStyles` drive
loop and `resolveLocation`/`resolveHref`.

    tools/browser-fetch-probe/run.sh [path-to-mfb]

Defaults `path-to-mfb` to `<repo-root>/target/release/mfb`. Builds
`examples/browser/{dom,fetch}` plus this tool's own `src/main.mfb` driver into
a scratch directory under `mktemp -d` (nothing here or under `examples/` is
modified), then checks two things against the freshly built `fetch` package,
imported directly — no TUI involved:

1. **`resolveLocation`/`resolveHref`** — five href shapes (a root-relative
   href whose query happens to contain `"://"`, a plain-relative href with the
   same quirk, a query-only href, a fragment-only href, and a `"../"`-relative
   href), each compared against its RFC 3986 §5.3 merge + remove-dot-segments
   value (the same values `urllib.parse.urljoin` computes for the same
   base+href pair — used here as the reference implementation). Fails with
   the mismatched label(s) if any output differs.

2. **`fetchStyles`'s drive-loop deadline** — starts two throwaway local
   servers (`page/blackhole.py`, a raw-socket server that accepts a
   connection and then never writes or closes it; and a plain
   `python3 -m http.server` serving a one-page HTML document whose single
   `<link rel="stylesheet">` points at the black hole) and fetches that page
   through `fetch::fetch` on its own OS thread, exactly like the browser app
   does. Asserts the whole fetch completes within 45 wall-clock seconds
   (`fetchStyles`'s own 30s per-batch deadline plus slack for process
   startup/scheduling noise) with `ok=TRUE`, `filesLoaded=1`, `filesTotal=2`
   (the stalled sheet skipped, matching how a sheet that fails to connect is
   already skipped), and that the run burned well under a second of CPU time
   despite waiting up to 30 wall-clock seconds — the busy-spin bug burns CPU
   time proportional to wall-clock wait, since neither `http::ready` nor
   `http::pump` ever blocks.

Confirmed RED against the pre-fix `examples/browser/fetch/src/lib.mfb` (all
five `resolveHref` cases but the pass-through absolute case mismatch); GREEN
against the fixed version.

`src/main.mfb` is a standalone driver with two modes (`resolve`, `spin
<page-url>`); `run.sh` is the only thing that invokes it and owns starting/
stopping the two Python servers the spin check needs.
