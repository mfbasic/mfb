# bug-1042: `net::toUrl` does no IDNA (UTS #46) host processing — a non-ASCII or `xn--` host is stored as written

Last updated: 2026-10-10
Effort: large (3h–1d)
Severity: LOW
Class: Security (parser differential) / Correctness

Status: Open
Regression Test: `tools/oracles/url/run.sh <mfb> parse` (+ `wpt`) with the groups `parse/lenient/host-idna-invalid`, `parse/strict/host-idna-mapped` and `parse/host-idna-mapped` back in class a, reporting zero entries

## Cause

- Rule: WHATWG URL host parsing runs UTS #46 "domain to ASCII" (CheckHyphens=false, CheckBidi=true, CheckJoiners=true, UseSTD3ASCIIRules=false, Transitional=false, VerifyDnsLength=false) on the percent-decoded host of a special URL, before the forbidden-code-point check and the IPv4 test. `mfb spec stdlib url` §Host validation currently says "Not implemented: IDNA" (the owner's 2026-10-10 interim ruling); the Goal of this doc removes that.
- Where: `src/codegen/builtins/net/helper_host.rs:__net_host`, between the percent-decode and the forbidden-character loop.
- Fix: one `domain to ASCII` step there (map + NFC via `strings::normalizeNFC`, Punycode decode/validate of `xn--` labels, Punycode encode of non-ASCII labels); the IPv4 test then sees the mapped text, so `１２７.0.0.1` and `０Ｘｃ０．０２５０．０１` canonicalize through the existing `__net_ipv4Host`. The IDNA mapping table is generated from a vendored `IdnaMappingTable-16.0.0.txt` by a generator under `tools/unicode-tables/` run by `scripts/check-generated.sh`. Deletes the spec's "Not implemented: IDNA" note and the `xn--` exception in §Host validation.

WHATWG runs UTS #46 `domain to ASCII` on an http(s) host: it maps (fullwidth to ASCII,
case folding), refuses what the tables forbid, and encodes non-ASCII labels as `xn--`
Punycode. `net::toUrl` does none of it. This is what bug-979 left. **Owner decision
(2026-10-10): a documented difference for now**; this bug keeps the implementation on the
books.

**Correct behavior (if implemented):** `toUrl` agrees with Node's WHATWG `URL` on every
host in the three oracle groups: the invalid ones raise `ErrInvalidFormat`, the mapped ones
store what WHATWG stores.

## Failing Reproduction

Measured 2026-10-10 on batch 20's staging compiler (`/tmp/bs20-target/release/mfb`),
`MFB_URL_WPT=~/.cargo/registry/src/*/url-2.5.8/tests/urltestdata.json tools/oracles/url/run.sh <mfb> parse wpt`:

| Group (class b today) | parse | wpt | Example |
| --- | --- | --- | --- |
| `parse/lenient/host-idna-invalid` | 18 | 19 | `xn--`, `a.b.c.xn--pokxncvks`, U+00AD alone, U+FFFD, `%C2%AD`: MFBASIC accepts, WHATWG refuses |
| `parse/strict/host-idna-mapped` | 1 | 0 | `http://１２７.0.0.1/`: MFBASIC refuses ("invalid IPv4 address"), WHATWG reads `127.0.0.1` |
| `parse/host-idna-mapped` | 0 | 1 | `０Ｘｃ０．０２５０．０１`: MFBASIC keeps the name, WHATWG reads `192.168.0.1` |

## What is known

`src/codegen/builtins/net/helper_host.rs:__net_host` validates the percent-decoded host
byte by byte and hands it to `__net_ipv4Host`; a name is returned as written. A UTS #46
step belongs between the decode and the forbidden-character check. It needs Punycode and
the Unicode mapping and validity tables, kept in step with the Unicode version the
`unicode` package ships.

The risk today: a check written against `Url.host` sees the text as written, while a
resolver or a peer that does apply IDNA may read a different name. The numeric-IPv4 bypass
bug-979 was filed for is closed either way: a host that ends in a number is canonicalized
or refused.

## Goal

- The three groups are class a in `tools/oracles/url/node/compare.mjs` and report zero.
- `mfb spec stdlib url` drops "Not implemented: IDNA".

### Non-goals

- Schemes other than http/https.
