# plan-93-B: Client-side gzip/deflate response decoding

Last updated: 2026-08-09
Effort: medium (1h–2h)
Depends on: plan-93-A (the `compress::` package must exist and round-trip)

The `http::` client sends no `Accept-Encoding` and never decodes a compressed
body today, so a server that responds with `Content-Encoding: gzip` hands the
program raw compressed bytes. This sub-plan makes the client advertise gzip/deflate
support and transparently decode the response body, so `resp.body` is always the
decoded payload — the behavioral outcome: `toString(http::read(url).body)` returns
readable text when the server gzips the response.

References:

- `src/docs/spec/stdlib/05_http.md:120-160` — current Body Decoding (204/304 →
  empty; chunked de-chunk; size cap) that this extends.
- `src/builtins/http_package.mfb:__http_decodeBody`, `__http_dechunk` — the decode
  path this hooks into after de-chunking.
- `src/builtins/http_package.mfb:__http_buildRequest` (auto-headers, ~line 157-208)
  — where `Accept-Encoding` gets defaulted and made overridable.
- plan-93-A — the `compress::gzipDecode` / `compress::inflate` primitives.

## Prerequisites

See plan-93-A's Prerequisites (the shared gate for the whole feature). This letter
additionally requires:

| Must be true | Command | Status |
|---|---|---|
| plan-93-A complete (compress package round-trips) | `ls planning/plan-93-A-*` in `planning/completed/` AND a `compress::gzipDecode` round-trip fixture is green | NOT MET |

If plan-93-A is not complete, this plan cannot start, full stop — there is no
fallback decoder path and no dual-mode design.

## 1. Goal

- The client sends `Accept-Encoding: gzip, deflate` by default, overridable by a
  caller header of the same name (case-insensitive), matching how `Accept`/
  `User-Agent` are handled today.
- After framing (de-chunk), the client decodes the body per `Content-Encoding`:
  `gzip` → `compress::gzipDecode`, `deflate` → `compress::inflate`; `identity`/
  absent → unchanged. Multiple stacked encodings are applied right-to-left.
- `resp.body` holds the **decoded** bytes; the response still reports what was on
  the wire (see Non-goals on which headers are rewritten).
- A **decoded-size cap** bounds decompression so a small compressed body cannot
  expand without limit (zip-bomb guard); exceeding it fails with a documented code.

### Non-goals (explicit constraints)

- **No brotli.**
- **Order of operations is fixed:** transfer-encoding (chunked) is the outer frame
  and is removed first; content-encoding is decoded on the de-chunked bytes. Do not
  reorder.
- **204/304 stay empty** regardless of any `content-encoding` header (existing rule
  wins).
- Keep `resp.body AS List OF Byte`; do not change the `Response` record shape.
- No auto-decompression when the caller **explicitly** overrode `Accept-Encoding`
  to something the client can't decode — in that case return the raw body and
  leave `content-encoding` intact (caller opted out).

## 2. Current State

### Measured populations

| What | Count | Command |
|---|---|---|
| accept-encoding handling in the package | 0 | `grep -ci 'accept-encoding' src/builtins/http_package.mfb → 0` |
| content-encoding handling in the package | 0 | `grep -ci 'content-encoding' src/builtins/http_package.mfb → 0` |

### Verified properties

- **Body decode happens in one place after framing.** `__http_decodeBody`
  (spec `05_http.md:120-133`) returns `""` for 204/304, de-chunks when
  `transfer-encoding` contains `chunked`, else returns the raw section. Content-
  encoding decoding slots in immediately after this returns the framed bytes.
  **VERIFIED** by reading the spec's Body Decoding section; confirm against the
  function body before editing.
- **Auto-headers are override-aware.** `__http_buildRequest` emits Host/User-Agent/
  Accept as overridable defaults and forces Connection/Content-Length
  (`05_http.md:69-89`). `Accept-Encoding` joins the overridable set with the same
  `__http_isExtraHeader`/`__http_headerValue` mechanism. **VERIFIED** via spec;
  confirm the extra-header reserved list before editing.
- **The 64 MiB cap is on the raw (compressed) stream.** `__HTTP_MAX_RESPONSE`
  (`05_http.md:152-160`). The decoded size is currently unbounded because nothing
  decodes — this sub-plan introduces the decoded cap. **VERIFIED** via spec.

## 3. Design Overview

Two edits to `http_package.mfb`, both small:

1. **Request:** add `Accept-Encoding: gzip, deflate` to the overridable auto-header
   set in `__http_buildRequest` (reuse the `Accept` machinery). Add its lowercased
   name to whatever reserved/extra-header bookkeeping treats the auto set.
2. **Response:** after `__http_decodeBody` returns the framed bytes, read the
   lowercased `content-encoding` header, split on `,`, and fold decoders
   **right-to-left** (`gzip`→`compress::gzipDecode`, `deflate`→`compress::inflate`,
   `identity`/`""`→identity). Enforce a decoded-size cap after each stage.

**Correctness risk (schedule last, behind a fixture):** the chunked+gzip
interaction (must de-chunk first) and the decoded-size cap. A gzip response that is
*also* chunked is the fixture that proves the ordering.

**Header semantics:** on successful decode, set `content-encoding` to `identity`
(or remove it) and leave `content-length` as-received — document the choice in the
spec so programs don't re-decode. This is the one observable-contract decision;
recommend **remove `content-encoding` on full decode** (body now matches absence of
the header). See Open Decisions.

Gate is **runtime behavior** (decoded body equals expected plaintext), never
byte-identity.

## Compatibility / Format Impact

- Requests now carry `Accept-Encoding` by default (observable on the wire); callers
  can override or clear it.
- `resp.body` for a compressed response changes from raw-compressed to decoded
  bytes — this is the intended behavior change; document it in the http spec.
- `Response` record shape unchanged.

## Phases

### Phase 1 — Advertise Accept-Encoding

- [ ] Add `Accept-Encoding: gzip, deflate` as an overridable default in
      `__http_buildRequest`; include it in the auto/reserved header bookkeeping so a
      caller header of the same name replaces it and isn't double-emitted.
- [ ] Tests: an rt-behavior fixture asserting the default request contains exactly
      one `Accept-Encoding: gzip, deflate`, and a second asserting a caller override
      replaces it. (Prefer the existing request-building test harness.)

Acceptance: default requests advertise gzip/deflate; caller override wins; no
duplicate header.
Commit: —

### Phase 2 — Decode Content-Encoding (behind the ordering fixture)

- [ ] After framing in the decode path, add content-encoding decoding: split the
      lowercased `content-encoding` on `,`, trim, fold decoders right-to-left via
      `compress::gzipDecode`/`compress::inflate`; `identity`/absent = passthrough.
- [ ] Add a decoded-size cap (constant, e.g. mirror/relate to `__HTTP_MAX_RESPONSE`)
      checked after each decode stage; exceed → documented error code.
- [ ] On full successful decode, drop the `content-encoding` header (chosen in
      Open Decisions) so the body and headers agree.
- [ ] Do not decode when the caller overrode `Accept-Encoding` to omit gzip/deflate
      and the server still sent an unknown encoding: return raw body, keep header.
- [ ] Tests: fixtures for (a) gzip body, (b) deflate body, (c) **chunked + gzip**
      together (proves de-chunk-then-decode order), (d) decoded-size-cap overflow →
      error, (e) `identity`/absent passthrough unchanged.

Acceptance: `toString(http::read(url).body)` returns the plaintext for gzip,
deflate, and chunked+gzip responses; the cap fixture errors; identity is byte-for-
byte unchanged.
Commit: —

## Validation Plan

- Tests: the fixtures above under `tests/rt-behavior/http/` (or the package's
  existing http fixture home); reuse a loopback/mock server if the harness has one,
  else feed canned raw responses to the parse/decode path directly.
- Coverage check: confirm the new decode branch is exercised (a green run with the
  chunked+gzip fixture present).
- Runtime proof: a program that GETs a known gzip endpoint (or a local
  `http::server` from plan-93-D that gzips) and prints the decoded body.
- Doc sync: extend `src/docs/spec/stdlib/05_http.md` Body Decoding + the auto-header
  table (`Accept-Encoding` row); update any affected man pages (`read`/`write`).
- Acceptance: full `cargo test` + acceptance harness on Linux + macOS.

## Open Decisions

- **What to do with `content-encoding` after decode** — recommend **remove it**
  (headers then describe the delivered body). Alternative: rewrite to `identity`.
  Either way, `content-length` is left as-received and documented as "wire length,
  not `len(body)` after decode". (§3)
- **Windows / no-decoder degradation** — per plan-93-A, if the `compress` package
  is unavailable on a target, keep the raw body and leave `content-encoding`
  intact rather than failing. (plan-93-A Open Decisions)

## Corrections

<Filled in during execution.>

## Summary

Small, well-contained: one request-header default and one response-decode fold. The
only real trap is ordering (de-chunk before content-decode) and the zip-bomb cap,
both pinned by fixtures. Depends entirely on plan-93-A; no fallback decoder here.
