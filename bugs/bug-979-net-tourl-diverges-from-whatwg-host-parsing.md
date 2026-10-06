# bug-979: `net::toUrl` diverges from WHATWG host parsing — no host validation, numeric IPv4 kept as written, `\` not an authority terminator

Last updated: 2026-10-05
Effort: large (3h–1d)
Severity: HIGH
Class: Security (parser differential) / Correctness — owner-confirmed

Status: Open
Regression Test: `tools/oracles/url/run.sh <mfb> parse` (+ `wpt` with `MFB_URL_WPT`) must report zero class-a/c divergences in the `parse/host-*`, `parse/split-differs/backslash` and `strict|lenient/backslash-authority` groups; plus rt-behavior/rt-error fixtures for the minimal repros below

**Owner decisions (2026-10-05):** URL handling is checked against Node's WHATWG `URL`
(oracle `tools/oracles/url/`, built during the hardening review). For http(s) URLs `toUrl`
must agree with it on:
1. **Host validation** (bug-873 item 5): a host that is not a valid reg-name, IPv4, or
   bracketed IPv6 is `ErrInvalidFormat` (77050003) at parse time.
2. **Numeric IPv4 is canonicalized like WHATWG:** `http://0x7f.1/`, `http://2130706433/`,
   `http://127.1/` and `http://0177.0.0.1/` all give host `127.0.0.1`. `127.0.0.010` gives
   `127.0.0.8`. The connection uses the canonical address.
3. **`\` is treated as `/`** in http(s) authority and path, as in WHATWG.
   `http://good.com\@evil.com/` has host `good.com`, path `/@evil.com/`.

## Measured (oracle run, `/tmp/hardening/mfb-165`, HEAD 846aa55fd)

`tools/oracles/url/run.sh /tmp/hardening/mfb-165` → 3036 cases without WPT (3782 with the
pinned WPT `urltestdata.json`). Full report: the oracle's `URL-ORACLE.md` findings
(`/tmp/hardening/findings/URL-ORACLE.md`).

- **A1 — no host validation (345 cases: parse 170 + wpt 175).** `__net_toUrl` accepts control
  bytes, `%zz`, invalid IPv4, any text inside `[]`, invalid IDNA, and keeps CR/LF in the host:
  `net::toUrl("http://h\r\n/")` gives host `h\r\n`, and `"http://exa mple.com/"` gives `exa mple.com`.
  `http://[v1.fe]/` renders as `http://v1.fe/` (re-bracket heuristic keys on `:`).
- **C1 — numeric IPv4 kept as written (79 parse + 12 wpt).** This matters. `net::lookup` on
  macOS reads `0177.0.0.1` as 177.0.0.1, `017700000001` as 127.0.0.1, and `127.0.0.010` as
  127.0.0.10. `tcp::connect("0x7f.1")` reached a 127.0.0.1 listener. An allow-list written
  against the canonical host is bypassable.
- **C2 — backslash (12 cases).** `http://good.com\@evil.com/` is user `good.com\`, host
  `evil.com` (`__net_authorityEnd` ends only at `/?#`).
- Minor: `toUrl("http://example.com:18446744073709551696/")` reports "non-digit in port" for an
  all-digit overflow (`__net_parsePort`). Fix the message to "port out of range".

## Root Cause

`src/codegen/builtins/net/func_to_url.rs:__net_toUrl` splits on `://`, then
`helper_authority_end.rs:__net_authorityEnd` (terminators `/?#` only), then takes the
host text verbatim. No host grammar, IPv4 number parser or IDNA step exists.

## Goal

- The `parse` (and `wpt`) oracle modes show no class-a or class-c divergence in the host and
  backslash groups. The class-b groups stay declared (http/https only, `://` required, no
  tab/newline stripping, no path normalization) and are documented in
  `mfb spec stdlib url`.
- bug-872 (IPv6 literal unusable): its validation half is covered here. Its connect half
  stays in bug-872.

### Non-goals

- Supporting schemes other than http/https.
- Forbidden wrong fix: rewording the spec to drop "garbage host fails".

## Blast Radius

- `http::*`, `tls::connect`, `tcp::connect` callers that take a `Url` host. The canonical
  host is what they connect to.
- bug-868 (`%+A` signed escape, oracle group A2): same oracle, separate fix.
