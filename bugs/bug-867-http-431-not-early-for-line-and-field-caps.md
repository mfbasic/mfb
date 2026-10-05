# bug-867: the http server's 8 KiB-line and 100-field caps are enforced only after the head is complete — an unterminated over-cap head gets 408 after 10 s, not an immediate 431

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Coordinator note: repro measured by the review agent only; the coordinator did not re-run it.
Regression Test: tests/rt-behavior/http (server loopback: over-long line with no terminator → 431 promptly)

The spec says the 64 KiB head cap, the 100-field cap and the 8 KiB line cap are each
answered `431` "as soon as the excess is seen, without waiting for the rest of the head".
Only the 64 KiB cap is enforced early. A client that sends a 9000-byte header line, or 101
header lines, without the terminating blank line holds the single-threaded accept loop for
the full 10 s idle timeout and then gets `408 Request Timeout`, not an immediate `431`.
The DoS window is bounded by the idle timeout, but the observable answer and timing contradict
the spec.

Correct behavior: as soon as the bytes read contain a complete header line over 8 KiB
(or an unterminated tail over 8 KiB), or more than 100 complete header lines, answer 431
and close.

References:

- src/docs/spec/stdlib/05_http.md §Lifecycle: "The request head may not exceed 64 KiB,
  100 header fields, or an 8 KiB line (`431 Request Header Fields Too Large`, answered as
  soon as the excess is seen, without waiting for the rest of the head)"

## Failing Reproduction

Server `/tmp/hardening/B04/srv` on 127.0.0.1:18090; client `/tmp/hardening/B04/py/rawc2.py`:

```
python3 rawc2.py longline_early   # "GET /echo HTTP/1.1\r\nX-L: " + 9000*"a" + "\r\n", then wait
python3 rawc2.py manyhdr_early    # request line + 101 "X-n: v\r\n" lines, no blank line, then wait
python3 rawc2.py bighead_early    # request line + 70000 bytes, no terminator
```

- Observed:
  ```
  === longline-no-terminator (10.5s) b'HTTP/1.1 408 Request Timeout\r\n...'
  === 101hdrs-no-terminator (10.5s) b'HTTP/1.1 408 Request Timeout\r\n...'
  === 70KB-no-terminator (0.5s) b'HTTP/1.1 431 Request Header Fields Too Large\r\n...'
  ```
- Expected: the first two answered `431` within ~0.5 s, like the third.

Contrast: with the terminator present, both caps answer 431 (`rawc.py longline`,
`manyhdrs` → 431 in 0.5 s); exactly 100 fields → 200.

## Root Cause

`src/codegen/builtins/http/helper_frame_advance.rs:__http_frameAdvance`: while
`headEnd < 0` it only tests `total > __HTTP_MAX_HEAD`. The field-count and line-length
caps are checked in `__http_requestHeaderMap`, which runs only once `\r\n\r\n` is found.
Track complete lines (count and longest) and the current unterminated line's length in
`__http_FrameState` during the pre-terminator scan.

## Goal

- The two repro cases get 431 without waiting for the idle timeout.

### Non-goals (must NOT change)

- 64 KiB head cap, 413/408 behavior, strict header parsing.

## Blast Radius

- `__http_frameAdvance` (shared by the TCP and TLS read loops): fixed for both.
