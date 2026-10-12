# bug-979: `net::toUrl` diverges from WHATWG host parsing — no host validation, numeric IPv4 kept as written, `\` not an authority terminator

Last updated: 2026-10-10
Effort: large (3h–1d)
Severity: HIGH
Class: Security (parser differential) / Correctness — owner-confirmed

Status: Fixed (batch 20, 2026-10-10). The three owner decisions and the port message are implemented. IDNA is not: by owner decision (2026-10-10, "c for now") it is a documented difference, and implementing it is bug-1042.
Regression Test: `tests/rt-behavior/net/bug979_toUrl_host_canonical`, `tests/rt-error/net/bug979_toUrl_host_invalid`; `tools/oracles/url/run.sh <mfb> parse` (+ `wpt` with `MFB_URL_WPT`) must report zero class-a/c divergences in the `parse/host-*`, `parse/split-differs/backslash` and `strict|lenient/backslash-authority` groups (zero in both modes after the IDNA groups became class b, see "IDNA")

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

As found: confirmed. All of `toUrl` is injected MFBASIC source (`Body::mfb` plus
`RegistryHelper::always` helpers), so there is no per-architecture code. The port message
came from `__net_parsePort` mapping every `toInt(text, 10)` failure to "non-digit in port",
including the overflow of an all-digit text.

## Fix

- `__net_authorityEnd` also ends the authority at `\`; `__net_toUrl` rewrites every `\` in
  the path to `/` (query and fragment keep theirs).
- `__net_toUrl` passes the extracted host to the new `__net_host(text, bracketed)`:
  - bracketed text must satisfy `__net_isIpv6` (the WHATWG IPv6 parser as a validator); it
    is stored as written;
  - any other host is percent-decoded (`__net_percentDecodeImpl`), refused on a forbidden
    domain byte (`<= 0x20`, `0x7F`, `#%/:<>?@[\]^|`), then given to `__net_ipv4Host`
    (WHATWG ends-in-a-number check + IPv4 parser over `__net_ipv4Number`), which returns
    the canonical dotted-decimal address, fails, or reports "not an address"; a name is
    stored as written.
- `__net_parsePort` checks every byte is an ASCII digit first ("non-digit in port"); the
  only remaining `toInt` failure is overflow, reported as "port out of range".
- New helpers: `__net_byteAt`, `__net_ipv4Number`, `__net_ipv4Host`, `__net_isIpv6`,
  `__net_host` (one `helper_*.rs` each, registered after the existing helpers).
- `src/codegen/builtins/tests/mfb_body_declared_errors.rs`: one `UNREACHABLE_QUALIFIED_CALL`
  row, `("__net_toUrl", "strings::replace", "ErrInvalidArgument")` — the needle is the
  literal `"\\"`, and replace raises that code only for an empty needle.
- Spec `mfb spec stdlib url` (Host validation, Conformance table, port, messages) and the
  `mfb man net toUrl` description updated.
- Oracle `tools/oracles/url/node/compare.mjs`: new class-b group
  `parse/strict/host-tab-newline` (MFBASIC now refuses a host holding tab/LF/CR, which
  WHATWG strips first — "no input cleanup"); the four groups the owner decided
  (`host-ipv4-noncanonical`, `split-differs/backslash`, `strict|lenient/backslash-authority`)
  move from class c to class a.

## Measured after the fix

`tools/oracles/url/run.sh target/release/mfb parse wpt` with the pinned WPT file:

| Group | parse before → after | wpt before → after |
|---|---|---|
| `parse/host-ipv4-noncanonical` | 79 → 0 | 12 → 1 |
| `parse/host-keeps-tab-newline` | 9 → 0 | 4 → 0 |
| `parse/lenient/host-forbidden-code-point` | 88 → 0 | 46 → 5 |
| `parse/lenient/host-ipv4-invalid` | 17 → 0 | 36 → 0 |
| `parse/lenient/host-ipv6-invalid` | 26 → 0 | 20 → 0 |
| `parse/lenient/host-percent-invalid` | 12 → 0 | 55 → 0 |
| `parse/lenient/host-idna-invalid` | 18 → 18 | 14 → 14 |
| `parse/split-differs/backslash` | 7 → 0 | 1 → 0 |
| `parse/strict/backslash-authority` | 3 → 0 | 0 → 0 |
| `parse/lenient/backslash-authority` | 1 → 0 | 0 → 0 |
| `parse/strict/other` (unclassified) | 0 → 1 | 0 → 0 |
| `parse/strict/host-tab-newline` (class b, new) | 0 → 9 | 0 → 4 |
| agree | 1039 → 1280 | 255 → 421 |

Class a/c/unclassified entries: parse 260 → 19, wpt 188 → 20.

## IDNA (owner decision, 2026-10-10: a documented difference for now)

After the fix, every remaining class-a/unclassified oracle entry needed UTS #46 (Punycode
decoding plus the Unicode mapping and validity tables), which none of the three decisions
settled: 19 entries in `parse` mode and 20 in `wpt`. Asked whether to implement UTS #46,
refuse every non-ASCII host, or declare the difference, the owner chose to declare it, for
now. Implementing UTS #46 is bug-1042.

- The oracle's `parse/lenient/host-idna-invalid` group is class b. A host whose
  percent-decoding holds a non-ASCII character is sorted into it (the five `wpt` cases such
  as `%C2%AD` were in `host-forbidden-code-point`).
- Two new class-b groups hold the hosts UTS #46 maps to an IPv4 address:
  `parse/strict/host-idna-mapped` (`http://１２７.0.0.1/`: its last part is a number and the
  others are not IPv4 numbers, so MFBASIC refuses it; WHATWG reads `127.0.0.1`) and
  `parse/host-idna-mapped` (fullwidth `０Ｘｃ０．０２５０．０１`: MFBASIC keeps it as a name,
  WHATWG reads `192.168.0.1`).
- `mfb spec stdlib url` states it under "Not implemented: IDNA" and in the Conformance table.

Measured by the coordinator on staging (`/tmp/bs20-target/release/mfb`):
`MFB_URL_WPT=<pinned urltestdata.json> tools/oracles/url/run.sh <mfb> parse wpt` exits 0;
`parse`: 1955 cases, 1280 agree, 0 class a/c/unclassified entries; `wpt`: 746 cases, 421
agree, 0 class a/c/unclassified entries.

## Goldens expected to change (not regenerated here)

`scripts/artifact-gate.sh target/release/mfb all` → `2326 golden(s) checked, 79 diff(s)`:

- 54 `.ir` goldens — exactly the fixtures whose IR embeds the `net` package
  (`grep -rl '#net_toUrl' tests`). In each: `#net_authorityEnd`, `#net_parsePort` and
  `#net_toUrl` change; `#net_byteAt`, `#net_ipv4Number`, `#net_ipv4Host`, `#net_isIpv6`
  and `#net_host` appear; the other `net` functions after `#net_authorityEnd` change only
  in `"line"` / error-location line values; nothing outside the `net` functions changes
  (checked on `rt-behavior/tcp/func_tcp_close_valid`; its `.ast` is identical).
- 25 `.ncodesum` goldens — the five covers that embed `net` (`byte-identity/http`, `net`,
  `tcp`, `tls`, `udp`) on all five targets. Checked on `byte-identity/tcp` macos-aarch64
  against a base compiler: the five new `_mfb_ifn_net_5F*` functions appear, the three
  edited ones change, four later `net` functions change only by a uniform source-line
  constant (+14 or +302), and string data objects are renumbered (six strings added, `-`
  removed).

`scripts/test-accept.sh` over those 54 fixtures: 54 mismatches, all `.ir`; no `build.log`
(program output) changes.

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
