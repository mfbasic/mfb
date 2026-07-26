# browser — a terminal web viewer

A tiny full-screen terminal "web browser" built on the `term::` TUI surface and
the `http::` client. It fetches a page, parses the HTML down to the title and
readable text, and shows it in a scrollable content area, with an address bar, a
padlock secure-indicator, and an animated loading spinner.

This example is split into two projects so the loading spinner can keep spinning
while a page loads and parses:

| Project    | Kind         | What it is                                                                 |
| ---------- | ------------ | -------------------------------------------------------------------------- |
| `app/`     | executable   | The TUI: layout, input handling, scrolling, drawing.                       |
| `backend/` | package      | The network worker: the blocking `http::` fetch, redirect following, and HTML parsing. |

## Why two projects?

`http::read` is **blocking** — it freezes the calling thread until the whole
exchange finishes. To keep the UI thread free to animate the spinner, the app
runs the fetch on a separate OS thread with `thread::start(backend::fetch, url)`
and polls `thread::isRunning` while redrawing.

`thread::start` requires its worker to be an **exported `ISOLATED` function from
an imported package** — a worker defined in the caller's own package is rejected
at compile time. So the fetch lives in the `backend` package, which the app
imports. `backend` is referenced by a **relative local path**, not published to a
package repository:

```json
"packages": [
  { "name": "backend", "version": "=0.1.0", "source": "file:packages/backend.mfp" }
]
```

## Building

The app imports a compiled package, so build the `backend` package first and
install its `.mfp` into the app's `packages/` directory, then build the app.
From the repository root:

```sh
# 1. Build the backend package -> examples/browser/backend/backend.mfp
mfb build examples/browser/backend

# 2. Install it where the app's manifest expects it
mkdir -p examples/browser/app/packages
cp examples/browser/backend/backend.mfp examples/browser/app/packages/backend.mfp

# 3. Build and run the app
mfb build examples/browser/app
./examples/browser/app/build/browser.out
```

`examples/**/packages/` is git-ignored, so the installed `backend.mfp` is a local
build artifact — re-run steps 1–2 whenever you change the backend source.

## Using it

Run it from an interactive terminal.

- **G** — enter Address Mode; type a URL, **Backspace** deletes, **Enter** loads
  it, **Esc** cancels. `https://` is assumed when you omit the scheme.
- **Up / Down** — scroll the loaded page.
- **Q** — quit (also aborts an in-flight load).

The top-bar indicator is a spinner while loading, 🔒 once a page has loaded over
https, and 🔓 otherwise. Redirects (301/302/303/307/308) are followed
automatically, up to 10 hops.

## Parsing

`backend::parseHtml` is a forgiving single-pass tag stripper, not an XML/DOM
parser: it drops `<script>`/`<style>`/comments, breaks lines on block-level
tags, decodes character references (`encoding::htmlUnescape`), and collapses
whitespace — so real, often-malformed HTML turns into readable text. The
`<title>` is captured separately and shown as a heading. A non-HTML body (JSON,
plain text) is shown verbatim.

Parsing runs on the worker thread, so only the small cleaned result crosses back
to the UI. It also caps how many text segments it collects (`maxParts`): the
runtime arena's free list is quadratic under the short-lived-String churn that
extracting segments creates (a known open issue — see the benchmark's arena
regression gate), so an unbounded parse of a multi-megabyte page would hang.
Past the cap the page is rendered as a truncated preview.
