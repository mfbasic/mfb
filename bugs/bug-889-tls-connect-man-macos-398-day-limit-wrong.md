# bug-889: `mfb man tls connect` says macOS refuses certificates valid ≥ ~398 days even with `allowSelfSigned`; the measured limit is 825 days

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (documentation — man surface)

Status: Open
Coordinator note: repro measured by the review agent only; the coordinator did not re-run it.
Regression Test: none (prose); optionally a macOS rt fixture with an 800-day self-signed cert + allowSelfSigned

`mfb man tls connect` (src/codegen/builtins/tls/func_connect.rs DESC): "One platform
note: macOS additionally requires a server certificate to carry a serverAuth extended
key usage and a validity window under about 398 days. A certificate outside those
limits is refused there even with allowSelfSigned set." The serverAuth half is
correct. The 398-day half is wrong for the `allowSelfSigned` case the note describes.
Apple's 398-day rule applies only to certificates that chain to a public root. For a
self-signed or private-CA chain the limit is the older 825 days, and that is what
`tls::connect` shows. A developer who reads the man page will shorten certs for no
reason, or will misdiagnose a failure that has a different cause.

Correct behavior: the man note says "a validity period of at most 825 days" for
self-signed/private-CA chains, with 398 days applying to publicly trusted chains.

## Failing Reproduction

```
cd /tmp/hardening/B06/certs
for d in 399 800 825 826 3650; do openssl req -x509 -newkey rsa:2048 -nodes -keyout d$d.key -out d$d.crt -days $d \
  -subj /CN=localhost -addext subjectAltName=IP:127.0.0.1,DNS:localhost -addext extendedKeyUsage=serverAuth; done
# server: openssl s_server -accept <port> -cert d$d.crt -key d$d.key -www -naccept 1
# client: /tmp/hardening/B06/tlscli  -> tls::connect("127.0.0.1", port, 3000, "localhost", TRUE)
/tmp/hardening/B06/clprobe.sh d825.crt d825.key 19552 localhost 1 3000
```

| validity (days) | EKU | `tls::connect(..., allowSelfSigned := TRUE)` on macOS 15 |
| --- | --- | --- |
| 399 | serverAuth | connects ✓ |
| 800 | serverAuth | connects ✓ |
| 825 | serverAuth | connects ✓ |
| 826 | serverAuth | `ERR:77070008` |
| 3650 | serverAuth | `ERR:77070008` |
| 30 | none | `ERR:77070008` (EKU half of the note is right) |
| 30 | clientAuth only | `ERR:77070008` |

- Observed: 399–825 days accepted.
- Documented: anything ≥ ~398 days refused.

## Root Cause

Prose in `src/codegen/builtins/tls/func_connect.rs` (DESC, "One platform note") describes the
public-CA rule instead of the rule that applies to the chains `allowSelfSigned` admits.

## Goal

- The man note states the 825-day limit for self-signed/private-CA chains (measured above).

## Blast Radius

- `mfb man tls connect` only; spec 17_transports.md does not repeat the number.
