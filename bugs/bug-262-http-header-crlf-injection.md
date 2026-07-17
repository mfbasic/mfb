# bug-262: HTTP client concatenates header names/values and URL path/query/host verbatim → CR/LF request splitting

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Security

Status: Open
Regression Test: (none yet)

`http::read`/`http::write` build the raw request by string-concatenating header
names and values (and the request-target derived from the URL path/query/host)
with no rejection of control characters. A program that forwards
attacker-influenced data into the `headers` map — or builds a `Url` from an
attacker-controlled `href` — lets the attacker embed `\r\n` and inject extra
headers or a whole second request line, i.e. HTTP request splitting/smuggling
against the upstream. The single correct behavior a fix produces: any header
name/value (and the derived request-target) containing a byte `< 0x20`
(specifically CR or LF) is rejected before the request is framed.

References:

- `planning/audit-2-fs-net-thread.md` (OS-09).
- `src/builtins/http_package.mfb:131` — `request = request & entry.key & ": " &
  entry.value & crlf` (verbatim concat).
- `src/builtins/http_package.mfb:80-89` — `__http_requestTarget` from
  `url.path`/`url.query`; `:72-77` — `Host:` from `url.host`.
- `src/builtins/http_package.mfb:62-70` — `__http_normalizeMethod` rejects
  space/empty in the *method* only; header names/values and the URL parts are not
  checked.

## Failing Reproduction

```basic
MUT h AS Map OF String TO String = {}
h = collections::set(h, "X-Tag", "a\r\nX-Admin: true\r\nEvil: 1")
LET r AS http::Response = http::read(net::toUrl("http://upstream/api"), h)
```

- Observed: the socket write contains
  `X-Tag: a\r\nX-Admin: true\r\nEvil: 1\r\n` — two injected headers.
- Expected: the header value is rejected (or the request fails) before framing.

Contrast: the request **method** is already validated for space/empty
(`__http_normalizeMethod`); only names/values and the request-target lack the
check.

## Root Cause

`__http_buildRequest` concatenates header names, header values, and the
request-target (`url.path`/`url.query`) and `Host` (`url.host`) into the raw
request bytes with no CR/LF/control-char filtering
(`http_package.mfb:131,72-89`). Only `\r\n` line terminators and response parsing
use CRLF in the file; nothing rejects a CRLF *inside* a value.

## Goal

- `__http_buildRequest` FAILs (`FAIL error(...)`) when any header name, header
  value, or the derived request-target contains a byte `< 0x20` (at minimum CR
  `0x0D` / LF `0x0A`). Pure companion-source logic; no surface change.

### Non-goals (must NOT change)

- Which hosts may be contacted (SSRF — OS-10 → bug-268).
- Normalization beyond control-char rejection (no header folding/canonicalization).
- The public `http::read`/`http::write`/`Url` API.

## Fix Design

Add a small `__http_hasControlBytes(s)` helper in `http_package.mfb` and call it
on each header name, each header value, and the request-target before they are
concatenated in `__http_buildRequest`; `FAIL error("invalid header/URL: control
character")` on a hit. Rejecting (rather than stripping) is chosen so a caller
cannot silently smuggle a truncated header.
