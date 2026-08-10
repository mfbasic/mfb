<!-- Bug document. See .claude/skills/write-bug/template.md -->

# bug-439: HTTP chunked response completion is detected by a naive `0\r\n\r\n` substring search, so a body that contains those bytes stops the read early and de-chunking overruns ("truncated chunk data")

Last updated: 2026-08-09
Effort: small fix (rewrite one detection branch); moderate test (loopback chunked server)
Severity: HIGH — makes `http::read` fail on real chunked sites whose body happens to contain the bytes `0\r\n\r\n`
Class: Correctness (HTTP client transfer-encoding framing)

Status: Open (discovered loading facebook.com through the `examples/browser` client, which calls `http::read`)
Regression Test: none yet — see Acceptance

## Symptom

Loading a large real page over `http::read` (e.g. `https://www.facebook.com/`)
fails with:

```
Error loading https://www.facebook.com/

truncated chunk data
```

`truncated chunk data` is raised by `__http_dechunkBytes`
(`src/builtins/http_package.mfb:760`) when a chunk's declared size runs past the
end of the received bytes:

```mfbasic
LET dataEnd AS Integer = dataStart + size
IF dataEnd > total THEN
  FAIL error(errorCode::ErrInvalidFormat, "truncated chunk data")
END IF
```

De-chunking is not the bug — it is the victim. It is handed a body that the
transport read loop declared *complete* when it was not.

## Root Cause

`__http_frameComplete` (`src/builtins/http_package.mfb:718`) decides when a
response has fully arrived. For a `Transfer-Encoding: chunked` body (framing
length `-1`) it uses a **naive byte-substring search** for the terminating
zero-length chunk:

```mfbasic
IF framing = -1 THEN
  RETURN __http_indexOfBytes(raw, strings::toBytes("0\r\n\r\n"), bodyStart) >= 0
END IF
```

The literal bytes `0\r\n\r\n` are *not* a reliable end-of-body marker: they can
occur **inside chunk data** (a `0` immediately followed by two CRLFs is common in
minified HTML/JS and in binary/compressed payloads). When that byte sequence
appears in a read *before* the real terminator arrives, `__http_frameComplete`
returns `TRUE` prematurely, the transport read loop stops
(`__http_done` at `:472`, and the blocking / streaming loops at `:1294` / `:1335`),
and `__http_dechunkBytes` then walks the real chunk framing, reaches a chunk whose
`dataStart + size` exceeds the truncated `raw`, and fails with
`truncated chunk data`.

The read loop reads in 65536-byte blocks, so the trigger is a chunked response
larger than one read whose content contains `0\r\n\r\n` within an early block —
exactly the shape of a big site like facebook.com. (A small response that fits in
one read is not affected: the whole body — including the real terminator — is
present, and `__http_dechunkBytes` parses the true framing correctly regardless of
the stray substring.)

The correct terminator is a **zero-length chunk at a chunk boundary**, which can
only be found by parsing the chunk framing, not by substring search — the same
walk `__http_dechunkBytes` already performs.

## Failing Reproduction

Any chunked response > 64 KiB whose body contains the bytes `0\r\n\r\n` before the
real terminator. facebook.com reproduces it live. A self-contained reproduction
needs a loopback server that streams such a body; sketch:

1. Start an `http::server` handler that responds with `Transfer-Encoding: chunked`
   and writes, in order:
   - a first chunk (say ~70 KiB) whose data contains the bytes `0\r\n\r\n`
     somewhere in the middle,
   - then more chunks,
   - then the real `0\r\n\r\n` terminator.
2. `http::read` that URL on loopback.
3. Observe `truncated chunk data` instead of the full body.

(If a direct unit hook is preferred, `__http_frameComplete` can be exercised on a
hand-built `List OF Byte`: a chunked body whose *first* chunk's data contains
`0\r\n\r\n` while the buffer is deliberately cut short of the real terminator must
return `FALSE`, but today returns `TRUE`.)

## Proposed Fix

Replace the substring check with a real chunk-framing walk that returns whether the
terminating zero-length chunk has been received at a valid boundary — mirroring
`__http_dechunkBytes` but reporting completeness instead of failing on a short
buffer. It must return `FALSE` (keep reading) when a size line or a chunk's data
(plus its trailing CRLF) has not fully arrived, and `TRUE` only on reaching a
`size = 0` chunk.

```mfbasic
' Whether a chunked body starting at bodyStart is fully present: the terminating
' zero-length chunk has been received at a real chunk boundary. Returns FALSE while
' any size line or chunk (data + trailing CRLF) is still incomplete, so the read
' loop keeps going instead of stopping on a `0\r\n\r\n` that merely appears inside
' chunk data.
FUNC __http_chunkedComplete(raw AS List OF Byte, bodyStart AS Integer) AS Boolean
  MUT cursor AS Integer = bodyStart
  LET total AS Integer = len(raw)
  LET crlf AS List OF Byte = strings::toBytes("\r\n")
  MUT scanning AS Boolean = TRUE
  MUT complete AS Boolean = FALSE
  WHILE scanning
    LET lineEnd AS Integer = __http_indexOfBytes(raw, crlf, cursor)
    IF lineEnd < 0 THEN
      scanning = FALSE
    ELSE
      MUT sizeText AS String = __http_bytesToText(__http_byteSlice(raw, cursor, lineEnd))
      LET semi AS Integer = __http_indexOf(sizeText, ";", 0)
      IF semi >= 0 THEN
        sizeText = __http_slice(sizeText, 0, semi)
      END IF
      LET size AS Integer = __http_hexToInt(strings::trim(sizeText))
      IF size = 0 THEN
        complete = TRUE
        scanning = FALSE
      ELSE
        LET dataEnd AS Integer = lineEnd + 2 + size
        ' need the chunk data AND its trailing CRLF before advancing
        IF dataEnd + 2 > total THEN
          scanning = FALSE
        ELSE
          cursor = dataEnd + 2
        END IF
      END IF
    END IF
  END WHILE
  RETURN complete
END FUNC
```

and in `__http_frameComplete`:

```mfbasic
IF framing = -1 THEN
  RETURN __http_chunkedComplete(raw, bodyStart)
END IF
```

This matches `__http_dechunkBytes`'s leniency (it stops at `size = 0` and ignores
trailers), so the two stay consistent: once `__http_chunkedComplete` says done,
`__http_dechunkBytes` decodes without overrunning.

Note: editing the embedded `src/builtins/http_package.mfb` requires touching its
includer `src/builtins/http.rs` so cargo re-runs `include_str!`, then
`cargo build --release --bin mfb`.

## Acceptance

- Add an rt-behavior test with a loopback chunked server (per Failing Reproduction)
  asserting `http::read` returns the full body, and that a body containing
  `0\r\n\r\n` in an early chunk no longer fails with `truncated chunk data`.
- Full `cargo test` green (this touches the shared HTTP client read path used by
  both blocking `http::read` and streaming `http::startRead` / `pump` / `finish`).

## Notes / Scope

- Only the chunked completion check is wrong; Content-Length framing
  (`have >= framing`) is fine.
- Loading facebook.com fully will still not *render* well (it serves
  `Content-Encoding`-compressed bodies and requires JS/cookies), but this bug is
  the reason it errors out instead of loading at all, and it affects any chunked
  site whose body contains the `0\r\n\r\n` byte sequence.
- Found via `examples/browser` (which calls `http::read` on a worker thread); the
  bug is entirely in the `http` builtin, not the example.
