# bug-980: the HTTP chunked decoder accepts framing llhttp (and RFC 9112) rejects — no CRLF check after data, whitespace around the size, bad extensions, missing final CRLF, malformed trailers, bytes after the message

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Security (request smuggling / desync surface) — owner-confirmed

Status: Open
Regression Test: `tools/oracles/url/run.sh <mfb> chunk-client chunk-server` with zero class-a/c divergences; plus fixtures for each repro

**Owner decision (2026-10-05): match Node's (llhttp's) strictness for all of these, on both the
client response path and the server request path.** Each malformed framing is
`ErrInvalidFormat` (77050003), as `mfb spec stdlib http` already promises for "malformed framing".
This settles bug-873 item 9.

## Measured (oracle, 49 cases per direction, `/tmp/hardening/mfb-165`)

| group | repro (body) | MFBASIC | Node |
| --- | --- | --- | --- |
| A3 CRLF after data | `4\r\nWikiXX5\r\npedia\r\n0\r\n\r\n` | `Wikipedia` | `HPE_STRICT` "Expected LF after chunk data" (both directions) |
| A4 whitespace around size | `  4\r\n…`, `4 \r\n…`, `4\t\r\n…` | `Wiki` | `HPE_INVALID_CHUNK_SIZE` |
| C3 extension shape | `4;\r\n…`, `4 ;x\r\n…` | accepted | `HPE_STRICT` / `HPE_INVALID_CHUNK_SIZE` |
| C4 no final CRLF | `4\r\nWiki\r\n0\r\n` then EOF | `Wiki` | client `aborted`, server `HPE_INVALID_EOF_STATE` |
| C5 malformed trailer | `…0\r\nnot a header\r\n\r\n` | `Wiki` | `HPE_INVALID_HEADER_TOKEN` |
| C6 bytes after message | `…0\r\n\r\nGARBAGE` (`Connection: close`) | `Wiki` | `HPE_CLOSED_CONNECTION` |

## Root Cause

`__http_dechunkBytes` (`src/codegen/builtins/http/`, shared by the client and
`__http_parseRequest`) `strings::trim`s the size line, skips the two bytes after each chunk's
data without checking them, never parses trailer lines, and does not require the trailer
section's final CRLF. The client's frame completion ignores bytes after the terminal chunk.

## Goal

- Both oracle chunk modes report zero class-a/c divergences.
- `mfb spec stdlib http` §Chunked transfer decoding states the RFC 9112 grammar the code
  enforces (`chunk-size [ chunk-ext ] CRLF`, data `CRLF`, `last-chunk`, trailer section, final
  `CRLF`) and lists each rejection.
- The server path closes the one connection on a framing error without killing the accept loop
  (bug-859's rule).

### Non-goals

- Trailer *semantics* (trailers are still discarded once validated).
