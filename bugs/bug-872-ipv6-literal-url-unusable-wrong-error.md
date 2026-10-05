# bug-872: an IPv6-literal URL parses but cannot be used — `tcp::connect("::1")` raises ErrAddressNotFound (spec: ErrAddressInvalid), and http's Host header would drop the brackets

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/rt-error/tcp (IPv6 literal → ErrAddressInvalid) + tests/rt-behavior/http (Host header for `[::1]`)

`net::toUrl("http://[::1]:18083/v6")` parses as specified (`host = "::1"`, brackets stripped,
renders back as `http://[::1]:18083/v6`). The transports layer is IPv4-only by design, and
the spec says an IPv6 literal is *rejected with `ErrAddressInvalid` rather than silently
mis-parsed*. It actually raises `ErrAddressNotFound` (77070002, "could not be resolved").
That is a different code with a different meaning ("DNS said no" versus "this address form is
unsupported"), so a program that branches on the documented code misbehaves.

Latent second defect, unreachable while connect fails:
`src/codegen/builtins/http/helper_host_header.rs:__http_hostHeader` writes `url.host` raw,
so an IPv6 URL would send `Host: ::1:18083`. RFC 9110 §7.2 / RFC 3986 §3.2.2 require
`[::1]:18083`.

Correct behavior: `tcp::connect` (and `tls::connect`, `udp`) with an IPv6 literal raises
`ErrAddressInvalid` (77070001). `http::read` of an IPv6 URL raises the same. When IPv6 is
added, `Host` re-brackets a host containing `:` (as `toString(Url)` already does).

References:

- src/docs/spec/stdlib/17_transports.md: "Addresses are IPv4. `net::lookup` returns only
  IPv4 results, the emitters use `AF_INET`, and an IPv6 literal is rejected with
  `ErrAddressInvalid` rather than silently mis-parsed."
- src/docs/spec/stdlib/06_url.md §IPv6 host handling (brackets stripped from `host`)
- Transports is batch B06's topic; found here via the http/url topics.

## Failing Reproduction

`/tmp/hardening/B04/v6/src/main.mfb`:

```
IMPORT io
IMPORT tcp

FUNC main() AS Nothing
  RES s AS tcp::Socket = tcp::connect("::1", 18083, 2000) TRAP(e)
    io::print("tcp::connect(::1) FAIL " & toString(e.code) & " " & e.message)
    RETURN
  END TRAP
END FUNC
```

- Observed: `tcp::connect(::1) FAIL 77070002 Network host name or address could not be resolved.`
  (The same code for `"[::1]"` and `"0:0:0:0:0:0:0:1"`. `http::read(net::toUrl("http://[::1]:18083/v6"))`
  → `FAIL 77070002`, with a Python IPv6 listener on `::1` that never saw a connection.)
- Expected: `FAIL 77070001` (ErrAddressInvalid).

## Root Cause

Hypothesis: the connect path hands the literal to the IPv4-only resolver
(`net::lookup`, `src/codegen/builtins/net/func_lookup.rs`), which finds no A record and
reports not-found, instead of first classifying the text as an IPv6 literal (contains `:`)
and rejecting it. Host-header half: `__http_hostHeader` above.

## Goal

- An IPv6 literal to any transport connect/bind → 77070001.
- `__http_hostHeader` brackets a host containing `:`.

### Non-goals (must NOT change)

- No IPv6 support is being added here.

## Blast Radius

- `tcp::connect`, `tls::connect`, `udp` send/bind, `net::lookup` on a literal; `http` client via `tcp`/`tls`.

## Addendum (batch B06)

- `tls::connect("::1")` raises `ErrTlsFailed` (77070008) on macOS — the TLS transport has the same IPv6-literal gap (measured by the review agent, `/tmp/hardening/B06/ip6`).
