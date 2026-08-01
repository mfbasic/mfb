# bug-413: Schannel `tls::connect` writes `pwszServerName` at the wrong struct offset → hostname verification is silently skipped (TLS MITM on Windows)

Last updated: 2026-07-28
Effort: small (<1h) code change; HIGH-priority security fix
Severity: HIGH
Class: Security (TLS certificate hostname-verification bypass → MITM)

Status: Open
Regression Test: tests/ — a **negative** Windows TLS test: `tls::connect` to a
server presenting a CA-valid certificate issued for a *different* hostname must
fail (ErrTls), not succeed. (A positive same-hostname test passes either way, which
is why this went unnoticed.)

`emit_verify_hostname` (`src/target/shared/code/tls/schannel_io.rs`) performs the
one certificate check Schannel does NOT do automatically:
`CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL, …)` with an
`SSL_EXTRA_CERT_CHAIN_POLICY_PARA`. On Win64 that struct is:

```
struct SSL_EXTRA_CERT_CHAIN_POLICY_PARA {
    DWORD  cbSize;         // offset 0
    DWORD  dwAuthType;     // offset 4
    DWORD  fdwChecks;      // offset 8
    // 4 bytes padding      // offset 12
    LPWSTR pwszServerName;  // offset 16  (8-byte pointer)
};                          // sizeof == 24
```

The code sets `cbSize = 24` (correctly declaring this 4-field x64 layout) and
zeroes `SSLPARA..+0x60`, but stores the server-name pointer at **`SSLPARA + 8`**:

```rust
abi::store_u32("%v9", "%v8", SSLPARA),        // cbSize = 24
...
abi::store_u32("%v9", "%v8", SSLPARA + 4),    // dwAuthType = 1 (SERVER)
abi::load_u64("%v9", abi::stack_pointer(), snamew_off),
abi::store_u64("%v9", "%v8", SSLPARA + 8),    // <-- writes fdwChecks@8 (+pad@12), NOT pwszServerName@16
```

So `pwszServerName@16` stays **NULL** (from the zeroing), and
`CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL)` with a NULL server name
performs **no hostname/CN/SAN match at all** — it only validates chain trust. Net
effect: a Windows `tls::connect`/`connectText` accepts **any chain-trusted
certificate regardless of the hostname it was issued for**. An attacker holding any
valid CA-issued certificate for any domain can MITM a connection to a different
domain. (Worse, the low 32 bits of the name pointer land in `fdwChecks@8`, which can
even set check-skip flags such as `SECURITY_FLAG_IGNORE_CERT_CN_INVALID = 0x1000`.)

Chain trust (`SCH_CRED_AUTO_CRED_VALIDATION`) still blocks untrusted roots, but the
hostname binding — the entire purpose of `emit_verify_hostname` — is defeated. The
module doc even calls this "the check the plan flags as easy to omit and never
notice."

References:

- `src/target/shared/code/tls/schannel_io.rs:155` (the `store_u64 … SSLPARA + 8`
  that should be `SSLPARA + 16`); `cbSize = 24` at :150 confirms the 4-field x64
  layout. Caller: `lower_tls_connect` (`schannel_impl.rs:446`) runs it on every
  Windows `tls::connect`/`connectText`. Found during goal-07.

## Failing Reproduction

Windows/Schannel-only; not reproducible on the macOS host. Static ABI proof: the
store offset `+8` (`fdwChecks`) contradicts the struct's own declared `cbSize = 24`,
under which `pwszServerName` is at offset 16; the zeroing loop leaves offset 16 NULL.

- Observed: `pwszServerName == NULL` → `CertVerifyCertificateChainPolicy` skips the
  server-name check → any CA-trusted cert is accepted for any hostname.
- Expected: `pwszServerName` points at the wide server-name string, so a cert whose
  CN/SAN does not match the connected hostname is rejected.

A negative test (connect to `example.com` presenting a valid cert for
`attacker.com`) would currently succeed; after the fix it must fail.

## Root Cause

`store_u64(name, SSLPARA + 8)` writes the server-name pointer into `fdwChecks`
(offset 8) instead of `pwszServerName` (offset 16); the real field stays NULL.

## Goal

- `emit_verify_hostname` stores the wide server-name pointer at `SSLPARA + 16`, so
  `CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL)` performs the hostname
  match; a wrong-hostname certificate is rejected on Windows exactly as on
  Linux/macOS.

### Non-goals (must NOT change)

- The chain-trust validation (already correct). The wide-string encoding of the
  server name (already prepared at `snamew_off`).

## Blast Radius

- `src/target/shared/code/tls/schannel_io.rs:155` — change `SSLPARA + 8` to
  `SSLPARA + 16` (leave `fdwChecks@8` as the zeroed 0). This is the only site.
- Verify no other consumer relies on the current (wrong) `fdwChecks` value being
  non-zero.
