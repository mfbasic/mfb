# bug-303: HTTP client reads responses as text (not bytes), corrupting/failing multibyte-UTF-8 and chunked bodies

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: tests/ (new) — a chunked UTF-8 response and a >64 KiB UTF-8 response are received intact

The HTTP client accumulates a decoded `String` via `net::readText(sock, 65536)` /
`tls::readText`. Two failure modes result: (1) `readText` raises `ErrEncoding` when
a multi-byte UTF-8 sequence is split across a 64 KiB read boundary, and the loop's
inline TRAP only recovers `ErrConnectionClosed` — so any response with a multibyte
character on a read boundary fails the whole request. (2) `__http_dechunk` reads a
hex chunk *byte*-length but slices `raw` with `strings::mid`, which indexes by
*Unicode scalar*; any non-ASCII byte desynchronizes the offsets, corrupting the body
or raising "malformed/truncated chunk". The plan-03-http design explicitly prescribed
byte accumulation (`net::read`/`tls::read` → `List OF Byte`) so framing is byte-exact
and decoding happens once — the implementation diverged from its own design.

The single correct behavior a fix produces: the client accumulates raw bytes, does
all Content-Length / chunked framing on bytes, and decodes to text exactly once at
the end — so any UTF-8 response, chunked or large, is received intact.

References:

- `planning/old-plans/plan-03-http.md:270-278` (byte-accumulation design).
- `net::readText` man page (a receive may split a multi-byte sequence; invalid UTF-8
  raises ErrEncoding).
- Found during goal-06 review of `src/builtins/http_package.mfb`.

## Failing Reproduction

- A chunked UTF-8 response (JSON API with `Transfer-Encoding: chunked` containing
  `é`/emoji): `__http_dechunk` mis-slices (byte length vs scalar index) → corrupt or
  "malformed chunk".
- A >64 KiB UTF-8 response with a multibyte char on a 65536-byte boundary:
  `ErrEncoding` (77020004) fails the request.

- Observed: request fails or body corrupts on non-ASCII responses.
- Expected: body received intact.

(Reasoned from code + man page; not network-exercised in the review.)

## Root Cause

`src/builtins/http_package.mfb:326` and `:352` (`__http_exchangeTcp` /
`__http_exchangeTls`) accumulate decoded text via `net::readText`/`tls::readText`;
`__http_dechunk` (`:211`) and `__http_slice` (`:48`) then mix byte-length chunk
headers with scalar-indexed `strings::mid`.

## Goal

- Accumulate `List OF Byte` via `net::read`/`tls::read`; do all framing/de-chunking
  on bytes (the server side already uses `__http_dechunkBytes`); decode once at the
  end.

### Non-goals (must NOT change)

- The public `http::` response API shape.
- ASCII-only response behavior (must remain correct).

## Blast Radius

- `__http_exchangeTcp`/`__http_exchangeTls`, `__http_dechunk`, `__http_slice` —
  fixed here.
- The server-side byte path (`__http_dechunkBytes`) — already correct, reuse it.

## Fix Design

Switch the read loops to byte reads and route Content-Length/chunk accounting through
byte-indexed helpers, decoding to `String` only after the full body is assembled.
Rejected alternative: widening the inline TRAP to retry on `ErrEncoding` — does not
fix the chunk byte-vs-scalar desync and still risks splitting a code point.

## Phases

### Phase 1 — failing test
- [ ] Tests for a chunked UTF-8 body and a >64 KiB UTF-8 body (fail today).
### Phase 2 — the fix
- [ ] Byte accumulation + byte-exact framing + single decode.
### Phase 3 — validation
- [ ] Full suite green; ASCII and non-ASCII, chunked and Content-Length, all intact.

## Validation Plan

- Regression: the chunked/large UTF-8 tests (a local loopback server fixture if the
  harness supports it).
- Doc sync: none (aligns with plan-03 design).

## Summary

The client decodes text mid-stream and mixes byte and scalar indexing, breaking
non-ASCII and chunked responses; moving to byte accumulation per the original design
fixes it. Real risk is reworking the framing helpers to be byte-exact.
