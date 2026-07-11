# bug-96 — audit source collector omits tls/http/crypto from capability, fallibility, and resource tables

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G4).
**Severity:** MED — `mfb audit` under-reports what a program can do; a
security-vetting tool that misses network use has a disclosure gap.
**Class:** correctness (audit misreport).

## Finding

`src/audit/collect/source.rs:503-571` — three tables lag the builtin surface:

1. **`builtin_capability`** maps only fs/io/thread/net/os/math/datetime. A
   program whose networking goes exclusively through `tls::connect` /
   `http::read` discloses **no** "network" capability, and
   `crypto::randomBytes` discloses no "randomness". (Audit runs on the
   pre-monomorph AST — audit/mod.rs:116-124 — so the http→net source
   rewriting never happens before collection.)
2. **`is_fallible_call`** hardcodes `fs|io|json|net|thread`. `tls::*`,
   `http::*`, `crypto::aes256GcmOpen`, `datetime::parse` etc. are TRAP-able
   but the enclosing function is reported non-fallible with no fallible call
   sites.
3. **`resource_producer`** knows File/Thread/Socket/Listener but not
   `tls.connect`/`tls.accept` (TlsSocket), `tls.listen`/`http.serverSSL`
   (TlsListener), or `net.bindUdp` (UdpSocket) — those resources vanish from
   the Resources section and from AUDIT-RESOURCE-CLOSE-MAY-FAIL findings.

## Trigger

```
IMPORT tls
LET s = tls::connect("host", 443)
```

`mfb audit` prints no Permissions entry, no AUDIT-PERM-NETWORK finding, no
Resources row, and marks the function non-fallible — an operator vetting the
program sees "no network use".

## Fix sketch

Extend the three tables: tls/http → network capability; crypto random/keygen →
randomness; add the tls/http/crypto/datetime fallible sets; add
TlsSocket/TlsListener/UdpSocket producers. Consider deriving these tables from
the builtin registry rather than hand-maintaining them, so new packages can't
silently fall out of audit coverage again.
