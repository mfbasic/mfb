# plan-93-D: JS/static MIME types by file extension

Last updated: 2026-08-09
Effort: small (<1h)
Depends on: nothing (independent of the gzip and cookie letters)

`http::respondFile` / `http::respondPath` serve any file whose caller-supplied
content type is `""` as `application/octet-stream`
(`src/builtins/http_package.mfb:1216-1225`). A browser then refuses to execute a
served `.js` as a script and ignores a `.css` as a stylesheet. This sub-plan adds a
filename-extension → MIME table so static assets — JavaScript especially — are
served with the correct `Content-Type` by default. Behavioral outcome: serving
`app.js` yields `Content-Type: text/javascript` (or `application/javascript`), and
`style.css` yields `text/css`, without the caller passing a type.

References:

- `src/builtins/http_package.mfb:1216-1225` (`__http_respondFile`) — the default
  `application/octet-stream` this replaces with extension lookup.
- `src/builtins/http_package.mfb:__http_respondPath` (spec `05_http.md:339-357`) —
  the path-safe server helper that also gains extension inference.
- `src/docs/spec/stdlib/05_http.md:328-357` — the constructor/helper contract to
  update.

## Prerequisites

Uses the shared feature gate in plan-93-A only for "tree builds & tests green".
No dependency on any other letter; can land before or after them.

| Must be true | Command | Status |
|---|---|---|
| Tree builds & tests green | `cargo test` | UNMEASURED — run before starting |

## 1. Goal

- A single source-of-truth extension→MIME table covering at least: `.html`/`.htm`
  → `text/html`, `.js`/`.mjs` → JS type, `.css` → `text/css`, `.json` →
  `application/json`, `.svg` → `image/svg+xml`, `.png`/`.jpg`/`.jpeg`/`.gif`/`.webp`
  → the image types, `.txt` → `text/plain`, `.wasm` → `application/wasm`, `.ico`
  → `image/x-icon`, `.woff`/`.woff2` → font types, `.xml`, `.pdf`, `.map`.
- `respondFile(file)` (no explicit content type) and `respondPath(req, root)` infer
  the `Content-Type` from the file's extension via the table; an unknown extension
  falls back to `application/octet-stream` (today's behavior).
- An explicit `contentType` argument to `respondFile` still wins (override).
- Text types that are UTF-8 by convention are labeled without a charset by default
  (document the choice); `; charset=utf-8` on text types is an Open Decision.

### Non-goals (explicit constraints)

- **No content sniffing** — extension only, never reading file bytes to guess type.
  Deterministic and injection-safe.
- **No signature changes** to `respondFile`/`respondPath`; the explicit
  `contentType` parameter and its override precedence are preserved.
- Not a general MIME database — a curated table of common web asset types, not all
  of IANA. Unknown → `application/octet-stream`.

## 2. Current State

### Measured populations

| What | Count | Command |
|---|---|---|
| extension/MIME inference in the package | 0 | `grep -ciE 'guessMime\|mimeType\|byExtension\|\.js"\|\.css"' src/builtins/http_package.mfb → 0` |
| default content type in `respondFile` | `application/octet-stream` | read `http_package.mfb:1220-1221` |

### Verified properties

- **`respondFile` defaults an empty type to `application/octet-stream`.** Read
  `http_package.mfb:1216-1225`: `MUT ct = contentType; IF ct = "" THEN ct =
  "application/octet-stream"`. The extension lookup replaces this default branch,
  keeping the explicit-argument override. **VERIFIED** by reading the function.
- **`respondPath` opens under `root` after canonicalization** (`05_http.md:351-357`)
  — the resolved filename (and thus its extension) is known before/at open, so
  inference has an extension to read. **VERIFIED** via spec; confirm the resolved
  path variable name before editing.

## 3. Design Overview

One new helper plus two one-line call-site changes:

1. `__http_mimeForExt(name AS String) AS String` — lowercase the substring after
   the last `.`, look it up in a static table, return the MIME or `""` for unknown.
2. In `__http_respondFile`: when `contentType = ""`, set `ct =
   __http_mimeForExt(<file name>)`, then fall back to `application/octet-stream`
   when the lookup is `""`. (Requires the file's name/path; if `RES File` doesn't
   carry it, thread the name from the caller or add an internal overload — confirm
   what the open file exposes.)
3. In `__http_respondPath`: pass the resolved request path's basename through the
   same helper.

The table is a `Map` (or ordered `MATCH`) built once. This is pure MFBASIC string
work — no native code, no new intrinsic, fully in keeping with the http package.

**Risk:** minimal. The only subtlety is obtaining the filename inside
`respondFile` (a `RES File`). If the resource doesn't expose its path, `respondPath`
(which has the path) still gets correct inference, and `respondFile`'s inference is
threaded from the request path there; document any residual gap.

Gate is **runtime behavior** (served header equals expected type), never
byte-identity.

## Compatibility / Format Impact

- `respondFile`/`respondPath` now emit a specific `Content-Type` for known
  extensions instead of `application/octet-stream`. Callers passing an explicit
  type see no change; callers relying on the octet-stream default for a *known*
  extension get the more correct type (intended). Unknown extensions unchanged.

## Phases

### Phase 1 — MIME table + inference helper

- [ ] Add `__http_mimeForExt` and the curated extension→MIME table to
      `http_package.mfb`.
- [ ] Wire it into `__http_respondFile` (empty-type branch) and `__http_respondPath`
      (from the resolved basename); keep the explicit-argument override and the
      `application/octet-stream` fallback.
- [ ] Tests: rt-behavior fixtures asserting served `Content-Type` for `.js`,
      `.css`, `.html`, `.json`, `.svg`, `.png`, an unknown extension
      (→ octet-stream), and an explicit-`contentType` override (table ignored).

Acceptance: serving `app.js`/`style.css`/`index.html`/`data.json`/`logo.svg`
yields the correct `Content-Type`; unknown falls back to octet-stream; explicit
argument overrides.
Commit: —

## Validation Plan

- Tests: the fixtures above under the http server test home; assert the emitted
  `Content-Type` header from `respondFile`/`respondPath`.
- Coverage check: green run with the `.js` and unknown-extension fixtures present.
- Runtime proof: `respondPath` a small static dir (`index.html` + `app.js` +
  `style.css`) via `http::server`, curl each, confirm the `Content-Type` header.
- Doc sync: extend `05_http.md` §"Constructors, combinators, static helpers" to
  document extension inference and the table; note the override precedence; update
  the `respondFile`/`respondPath` man pages
  (`src/docs/man/builtins/http/{respondFile,respondPath}.md`).
- Acceptance: full `cargo test` + acceptance harness.

## Open Decisions

- **JS MIME string** — recommend `text/javascript` (the current WHATWG/IANA
  preferred type) over the legacy `application/javascript`. (§1)
- **`charset=utf-8` on text types** — recommend appending `; charset=utf-8` to
  `text/html`, `text/css`, `text/javascript`, `text/plain`, and `application/json`
  by default (browsers assume UTF-8 for these anyway; explicit is safer).
  Alternative: bare type, let the caller add charset. (§1)

## Corrections

<Filled in during execution.>

## Summary

The smallest, lowest-risk letter — a curated static table and two call-site edits,
pure MFBASIC, no native code, no dependency on the gzip primitive. The only open
question is whether `RES File` exposes its name inside `respondFile`; `respondPath`
always has the path.
