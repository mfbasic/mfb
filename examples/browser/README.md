# browser — a terminal web viewer

A tiny full-screen terminal "web browser" built on the `term::` TUI surface and
the `http::` client. It fetches a page, parses the HTML into a small DOM, and
shows the readable text — with an address bar, the page title, a padlock
secure-indicator, an animated loading spinner, and a **Raw Mode** that displays
the parsed DOM as an indented tree.

It is split into three packages plus the app so each concern is its own unit and
the loading spinner keeps spinning while a page loads *and* parses:

| Project     | Kind       | What it is                                                                    |
| ----------- | ---------- | ----------------------------------------------------------------------------- |
| `dom/`      | package    | The DOM: `Node` (`ElementNode`/`TextNode`/`HeaderNode`/`StyleNode`) + an HTML **and** CSS parser. |
| `fetch/`    | package    | The network worker: blocking `http::` fetch, redirects, `dom::parse`, **and loading external stylesheets**. |
| `display/`  | package    | Renders a `dom::Node`: `render(node, width)` → text, `tree(node, width)` → an indented DOM tree. |
| `app/`      | executable | The TUI: layout, input, scrolling, the three modes.                          |

`fetch` and `display` both import `dom` (they name its `Node` type in their APIs);
the app imports all three. All are referenced by **relative local paths**, not a
package repository:

```json
"packages": [
  { "name": "dom",     "version": "=0.1.0", "source": "file:packages/dom.mfp" },
  { "name": "fetch",   "version": "=0.1.0", "source": "file:packages/fetch.mfp" },
  { "name": "display", "version": "=0.1.0", "source": "file:packages/display.mfp" }
]
```

## Why a worker thread — and why the DOM crosses it

`http::read` is **blocking**: it freezes the calling thread until the whole
exchange finishes. To keep the UI free to animate the spinner, the app runs the
fetch on a separate OS thread with `thread::start(fetch::fetch, url)` and polls
`thread::isRunning` while redrawing. `thread::start` requires an **exported
`ISOLATED` function from an imported package**, which is why the fetch lives in
`fetch`.

The worker parses the page into a DOM on that thread and returns the **document
`Node`** — a recursive tree — which `thread::waitFor` **deep-copies** out of the
worker's arena into the app's. (Transferring a recursive value across threads was
`bug-391`; it is fixed.) So both the network *and* the parse stay off the UI
thread, and the app just renders the tree.

## Building

Each package must be built to a `.mfp` and installed where the next one's
manifest expects it (`packages/<name>.mfp`). From the repository root:

```sh
# 1. dom (no deps)
mfb build examples/browser/dom

# 2. fetch and display both depend on dom
for p in fetch display; do
  mkdir -p examples/browser/$p/packages
  cp examples/browser/dom/dom.mfp examples/browser/$p/packages/dom.mfp
  mfb build examples/browser/$p
done

# 3. the app depends on all three
mkdir -p examples/browser/app/packages
cp examples/browser/dom/dom.mfp examples/browser/fetch/fetch.mfp \
   examples/browser/display/display.mfp examples/browser/app/packages/
mfb build examples/browser/app
./examples/browser/app/build/browser.out
```

The installed `.mfp` files (and each package's build artifact) are git-ignored,
so re-run the relevant steps whenever you change a package's source.

## Using it

Run it from an interactive terminal. The top bar shows the page **title**
(right-aligned) in Display Mode, or `HREF: <url>` while editing an address. The
far-left indicator is a spinner while loading, 🔒 for an https page, 🔓 otherwise.

- **G** — enter Address Mode; type a URL, **Backspace** deletes, **Enter** loads
  it, **Esc** cancels. `https://` is assumed when you omit the scheme.
- **R** — (Display Mode) switch to **Raw Mode**: the parsed DOM as an indented tree.
- **D** — (Raw Mode) switch back to Display Mode.
- **Up / Down** — scroll.
- **Q** — quit (also aborts an in-flight load).

Redirects (301/302/303/307/308) are followed automatically, up to 10 hops.

## The DOM parser

`dom::parse` is a forgiving, iterative tag stripper (not a strict XML parser): it
drops `<script>`/`<style>`/comments (recording their src/href/inline markers in a
`HeaderNode`), captures `<title>`, builds an `ElementNode`/`TextNode` tree, decodes
character references (`encoding::htmlUnescape`), and handles unclosed tags. The
document is an `ElementNode` tagged `#document` whose first child is the header.

Everything is **iterative** (an explicit stack, never a recursive function over
`Node`) — a recursive function over an *imported* union does not lower to native
code across a package boundary, so `dom`/`display` walk trees with a work-stack.
`dom::parse` also caps its node count: extracting nodes churns short-lived arena
allocations, whose free list is quadratic (a known open arena issue), so a
multi-megabyte page is rendered as a truncated preview rather than hanging.

## CSS

The `fetch` worker also loads stylesheets: it reads each `<link rel="stylesheet">`
over http (resolving relative hrefs against the page URL) and captures each inline
`<style>`. Every stylesheet body is kept on the header (`HeaderNode.css`) and
parsed by `dom::parseCss` into **`StyleNode`s** — one per rule, each with the
selector and a `props: Map OF String TO String` (a comma selector list becomes one
rule per selector). The rules hang off `HeaderNode.rules` and show in Raw Mode; the
footer's `Files: n/m` counts the HTML document plus each stylesheet fetched.

`StyleNode` is a `Node` **variant** (not a standalone record) on purpose: a
consumer package (`display`) can read a union variant's fields, but the fields of a
record reached only *transitively* through an imported type are opaque to its
codegen — so all `StyleNode` iteration lives in `dom`, which returns plain strings
(`dom::styleLines`).
