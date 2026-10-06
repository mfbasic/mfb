# bug-868: `net::percentDecode` / `net::parseQuery` accept a signed escape — `"%+A"` decodes to LF (0x0A)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Security

Status: Resolved (2026-10-05)
Regression Test: tests/rt-behavior/net/func_net_percentDecode_valid, func_net_parseQuery_valid (extended)

## Resolution

Fixed in `__net_percentDecodeImpl` (`helper_percent_decode_impl.rs`): both
characters after a `%` are now validated as hex digits (new
`__net_isHexDigitChar` helper) before `toInt(.., 16)` parses them, matching
`encoding::percentDecode`'s existing `__encoding_hexValue`-based validation.
`net::parseQuery`'s `__net_decodeQueryComponent` already traps any `FAIL`
from this helper and falls back to the raw text, so it needed no change —
once the signed escape fails, it is automatically kept raw.

RED: `bug-868 RED: pin the signed percent-escape rejection` (commit
`67429ec57`) — `percentDecode("%+5")` did not raise, and
`parseQuery("k=%+5")` decoded to byte `0x05` instead of keeping `"%+5"` raw.
GREEN: `bug-868: reject a signed percent-escape in net::percentDecode`
(commit `ba8744b5a`).


`net::percentDecode` parses the two characters after `%` with `toInt(text, 16)`, which
accepts a leading `+`. So `"%+5"` decodes to byte 0x05 and `"%+A"` to 0x0A (line feed)
instead of failing. The `http` server percent-decodes every request path with this
function (`Request.path`) and decodes query values through the same core. A filter or WAF
that looks for `%0A`/`%0D` is bypassed by `%+A`/`%+D`, and the decoded path can carry a
control byte the client never wrote as a valid escape. The sibling
`encoding::percentDecode` validates each hex digit and rejects `%+5` (verified).

Correct behavior: `%` must be followed by exactly two hex digits. `%+5` fails
`ErrInvalidFormat` (77050003) in `percentDecode`. In `parseQuery`'s tolerant mode the
component is kept raw (`"%+5"`).

References:

- src/docs/spec/stdlib/06_url.md §Component Decoding: "`net::percentDecode` decodes `%XX`
  escapes … A truncated/invalid escape or non-UTF-8 result fails `ErrInvalidFormat`"
- RFC 3986 §2.1 (`pct-encoded = "%" HEXDIG HEXDIG`)

## Failing Reproduction

`/tmp/hardening/B04/repro/small/src/main.mfb` (`pd` prints the decoded bytes or FAIL code):

```
  pd("%+5")
  pd("%+A")
  pd("%-1")
  pd("%05")
  LET q AS Map OF String TO String = net::parseQuery("k=%+5")
  io::print("parseQuery k=%+5 -> bytes " & encoding::hexEncode(strings::toBytes(collections::get(q, "k"))))
```

- Observed:
  ```
  net::percentDecode(%+5) = bytes 05
  net::percentDecode(%+A) = bytes 0a
  net::percentDecode(%-1) FAIL 77050003
  net::percentDecode(%05) = bytes 05
  parseQuery k=%+5 -> bytes 05
  ```
- Expected: `%+5` and `%+A` → `FAIL 77050003`; `parseQuery` → raw `"%+5"` (bytes `252b35`).

`%-1` only fails because `toByte(-1)` raises. `python3 urllib.parse.unquote('%+5')` leaves
it literal (`'%+5'`).

## Root Cause

`src/codegen/builtins/net/helper_percent_decode_impl.rs:__net_percentDecodeImpl`:
`toByte(toInt(strings::mid(s, i + 1, 2), 16))`. `toInt` with radix 16 accepts a sign (the
same reason `__net_parsePort` and `__http_hexToInt` reject `+`/`-` before calling it).
Validate both characters as hex digits, as `encoding`'s `__encoding_percentDecodeBytes`
does with `__encoding_hexValue`.

## Goal

- `%+5`, `%+A`, `%-1`, `% 5` all → 77050003 from `net::percentDecode`; `parseQuery` keeps such components raw.

### Non-goals (must NOT change)

- `+` outside an escape stays literal in `percentDecode` and becomes a space in `parseQuery`.

## Blast Radius

- `net::percentDecode`, `net::parseQuery` (via `__net_decodeQueryComponent`), and so `http` server `Request.path`/`Request.query`: fixed by this bug.

## Golden impact

`__net_percentDecodeImpl`'s body gained a hex-digit validation check, and a new private
helper `__net_isHexDigitChar` was added to the `net` package's assembled source. Every
fixture that `IMPORT net`s — directly, or transitively through `http`/`tls`/`tcp`/`udp`,
all of which pull in `net` — will show a changed `.ast`/`.ir`/`.ncode`. Expect diffs
under `tests/byte-identity/net/` (and, transitively, `tcp`/`udp`/`tls`/`http` byte-identity
fixtures, since they import `net`), `tests/syntax/net/**`, and any `rt-behavior`/`rt-error`
`net` fixture with a golden.
