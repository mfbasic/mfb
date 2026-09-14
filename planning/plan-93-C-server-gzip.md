# plan-93-C: Server-side gzip response compression

Last updated: 2026-08-09
Effort: medium (1h–2h)
Depends on: plan-93-A (the `compress::` package must exist and round-trip)

The `http::` server serializes every response body verbatim and forces a
`Content-Length` from `len(resp.body)` (`__http_serializeHead`,
`src/builtins/http_package.mfb:1166-1180`); it never compresses. This sub-plan
makes `handleRequest` gzip a response body when the request's `Accept-Encoding`
allows it and the payload is worth compressing, setting `Content-Encoding: gzip`
and a correct `Content-Length`. Behavioral outcome: a client that sends
`Accept-Encoding: gzip` receives a `Content-Encoding: gzip` body that decodes back
to exactly what the handler returned.

References:

- `src/builtins/http_package.mfb:__http_serializeHead` (lines 1166-1180) — forces
  `Content-Length: len(resp.body)` and `Connection: close`; the hook point.
- `src/builtins/http_package.mfb:__http_handleRequest` (spec `05_http.md:236-357`)
  — the parse/match/dispatch/emit core; negotiation reads `req.headers`.
- `src/docs/spec/stdlib/05_http.md:328-357` — the constructor/emit contract
  (`Content-Length`/reason/`Connection: close` are always server-supplied).
- plan-93-A — `compress::gzipEncode`.

## Prerequisites

See plan-93-A's Prerequisites (shared feature gate). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-93-A complete (compress round-trips) | `ls planning/completed/plan-93-A-*` AND compress fixture green | NOT MET |

If plan-93-A is not complete, this plan cannot start, full stop.

## 1. Goal

- When the request's `Accept-Encoding` contains `gzip` (q≠0), the server may
  gzip-compress the response body, add `Content-Encoding: gzip`, and set
  `Content-Length` to the compressed length.
- Compression is applied only when: the body is above a size threshold, is not
  already `Content-Encoding`-tagged by the handler, and the status permits a body
  (not 204/304/1xx). Otherwise the body is sent uncompressed, unchanged.
- The emitted body decodes (via `compress::gzipDecode` or any standard client) back
  to the exact bytes the handler returned.
- `Vary: Accept-Encoding` is added when the response was negotiated on
  `Accept-Encoding`, so caches don't serve a gzip body to a non-gzip client.

### Non-goals (explicit constraints)

- **No brotli.**
- **No opt-out of the framing invariants:** `Content-Length` and `Connection:
  close` remain server-forced; a handler-set `Content-Length` stays ignored
  (existing rule). Compression recomputes `Content-Length`, it does not surrender
  server control of it.
- **Do not double-encode:** if the handler already set `Content-Encoding`, leave
  the body alone.
- No streaming/chunked compression — the server buffers the full body already
  (value `Response`), so one-shot gzip is correct and simplest.
- No change to the `Response` record shape or to `respondFile`/`respondPath`
  signatures.

## 2. Current State

### Measured populations

| What | Count | Command |
|---|---|---|
| content-encoding handling in the package | 0 | `grep -ci 'content-encoding' src/builtins/http_package.mfb → 0` |
| places Content-Length is forced on emit | 1 | `grep -n 'Content-Length' src/builtins/http_package.mfb` → server emit at `:1179` |

### Verified properties

- **The server computes `Content-Length` from the final body bytes.**
  `__http_serializeHead` writes `"Content-Length: " & toString(len(resp.body))`
  (`http_package.mfb:1179`) after copying handler headers except
  content-length/connection. So compressing `resp.body` *before* serialize makes
  the forced length correct automatically. **VERIFIED** by reading lines 1166-1180.
- **Negotiation input is available.** `req.headers` is a lowercased last-wins map
  (`05_http.md:270-298`), so `accept-encoding` is readable in `handleRequest`
  before emit. **VERIFIED** via spec Request record.
- **Body-permitting status is already known.** The client decode path special-cases
  204/304; the server must apply the same "no body" statuses to skip compression.
  **VERIFIED** via `05_http.md:120-133`.

## 3. Design Overview

A single negotiation step inserted between "handler produced a `Response`" and
"serialize + write" in `handleRequest`:

```
maybeCompress(req, resp):
  IF status has no body (204/304/1xx)          -> resp unchanged
  IF resp already has content-encoding          -> resp unchanged
  IF len(resp.body) < MIN_GZIP                   -> resp unchanged
  IF NOT acceptsGzip(req.headers["accept-encoding"]) -> resp unchanged
  ELSE:
    body' = compress::gzipEncode(resp.body)
    resp' = WITH resp { body := body',
                        headers += content-encoding: gzip,
                        headers += vary: accept-encoding }
    RETURN resp'
```

Because `Content-Length` is recomputed from `len(resp.body)` at serialize time,
compressing the body is all that's needed for correct framing — no manual length
math. `acceptsGzip` parses the `Accept-Encoding` list, honoring `gzip;q=0` as a
refusal.

**Correctness risk (schedule last):** the `q=0` refusal parsing and the "already
encoded / no-body status" guards — a wrong guard produces a body a client can't
decode. Pin each with a fixture.

Gate is **runtime behavior** (client decodes the emitted body to the original),
never byte-identity.

## Compatibility / Format Impact

- Responses to gzip-capable clients gain `Content-Encoding: gzip` + `Vary:
  Accept-Encoding` and a compressed body/`Content-Length`. Non-gzip clients and
  small/no-body responses are byte-for-byte unchanged.
- No API/signature changes; purely internal negotiation in `handleRequest`.

## Phases

### Phase 1 — Accept-Encoding negotiation helper

- [ ] Add `__http_acceptsGzip(headerValue AS String) AS Boolean` parsing a comma
      list with optional `;q=` weights; `gzip` present and not `q=0` → TRUE (also
      honor `*` unless `gzip;q=0`).
- [ ] Tests: unit fixtures for `gzip`, `gzip, deflate`, `gzip;q=0`, `*`, empty,
      and absent → expected boolean.

Acceptance: the helper returns the correct decision for the tabulated inputs.
Commit: —

### Phase 2 — Compress on emit (behind the round-trip fixture)

- [ ] Insert `maybeCompress(req, resp)` before serialize in `__http_handleRequest`,
      guarded by no-body-status, already-encoded, min-size, and `acceptsGzip`.
- [ ] Set `content-encoding: gzip` and `vary: accept-encoding` via the header map;
      let `__http_serializeHead` recompute `Content-Length` from the compressed body.
- [ ] Tests: rt-behavior fixtures — (a) gzip client gets a `Content-Encoding: gzip`
      body that `compress::gzipDecode` restores to the handler's bytes and whose
      `Content-Length` equals the compressed length; (b) non-gzip client gets the
      original uncompressed body; (c) a handler that pre-set `content-encoding` is
      left untouched; (d) a 204 / small body is not compressed.

Acceptance: gzip client round-trips; non-gzip client and excluded cases are
unchanged; `Content-Length` matches the emitted body in every case.
Commit: —

## Validation Plan

- Tests: fixtures under the http server test home; drive `handleRequest` against a
  crafted `Request` (or loopback client) and assert the raw emitted bytes /
  decoded body.
- Coverage check: confirm the negotiation branch is exercised (green run with the
  gzip-client fixture present).
- Runtime proof: run `http::server`, curl it with and without `--compressed`, and
  confirm `Content-Encoding: gzip` + a matching decoded body only when requested.
- Doc sync: extend `05_http.md` Server section with the negotiation rules and the
  `Vary`/`Content-Encoding` behavior; update the emit-contract paragraph
  (`:328-357`) to note server-side compression.
- Acceptance: full `cargo test` + acceptance harness on Linux + macOS.

## Open Decisions

- **Minimum-size threshold** — recommend a small constant (e.g. 256 bytes) below
  which compression is skipped (gzip overhead exceeds savings). Tune with a fixture.
- **Content-type allowlist** — recommend compressing regardless of type initially
  (simplest, correct); a future refinement could skip already-compressed types
  (images/zip). Documented as a non-blocking follow-up, not scope here.

## Corrections

<Filled in during execution.>

## Summary

Low risk: compress the body before the existing serialize step and the forced
`Content-Length` follows for free. The only traps are the negotiation guards
(`q=0`, no-body statuses, already-encoded), each pinned by a fixture. Depends on
plan-93-A; independent of the client work in plan-93-B.
