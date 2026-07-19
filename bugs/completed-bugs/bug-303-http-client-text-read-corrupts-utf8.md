# bug-303: HTTP client reads responses as text (not bytes), corrupting/failing multibyte-UTF-8 and chunked bodies

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Fixed
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

## Resolution

The client now accumulates bytes, frames on bytes, and decodes exactly once — the
design plan-03-http specified and the implementation had drifted from. Notably, the
byte primitives it needed (`__http_indexOfBytes`, `__http_byteSlice`,
`__http_bytesToText`, `__http_dechunkBytes`) **already existed**: the *server* half
of the same file was written byte-correctly. Only the client path was text-based, so
this is mostly a matter of routing it through machinery already present and proven.

- `__http_exchangeTcp` / `__http_exchangeTls` read via `net::read` / `tls::read` into
  `List OF Byte`, matching the server's accept loop verbatim.
- `__http_parseResponse` takes bytes, finds `CRLFCRLF` by byte offset, and decodes
  **only the head** (ASCII per RFC 9110). The body stays bytes into
  `Response.body` — which was already `List OF Byte`, so the old code was decoding
  and then re-encoding, doing the work twice *and* losing data in between.
- `__http_decodeBody` takes and returns bytes, dispatching to `__http_dechunkBytes`.

### Both failure modes reproduced and proven fixed, byte-exactly

Not "it compiles" — a real server on a real socket, with the payload hashed on both
ends:

1. **Chunked multibyte UTF-8.** A server emitting 1880 bytes of
   `héllo wörld — ünïcode ✓ 日本語 🎉` in 97-byte chunks deliberately chosen to split
   multibyte sequences. Before: `trapped: 77020004` (ErrEncoding). After:
   `bytes=1880`, `sha=1d6ed2a5b3705cfa` — an exact match for the server's own SHA.
2. **A multibyte character straddling the 64 KiB read boundary.** 65535 filler bytes,
   then `é` spanning offsets 65535–65536, then more. After: `bytes=66537`,
   `sha=b00f4e1137db52b3`, again matching the server exactly.

Both were bisected: stashing `http_package.mfb` alone restores the failure.

### The broken helper was deleted, not left unused

The fix orphaned the String-based `__http_dechunk` — the one that read a hex chunk
*byte* length and sliced with `strings::mid`, which indexes by Unicode scalar. It is
deleted rather than left in place, with a note at the site, so a future caller cannot
pick the broken version back up. `__http_dechunkBytes` is the only chunk decoder now.

One `.ir` golden moved (it captures the lowered stdlib source). Runtime behaviour is
unchanged and was verified rather than assumed: `func_http_response_valid`'s
`build.log`, which embeds the program's complete output, is byte-identical.

Full `cargo test` green; artifact gate 0 diffs; acceptance 1006/1006.
