# bug-461: Windows `tls::listen` rejects a PKCS#1 private key

Last updated: 2026-08-30
Effort: small (< 1h)
Severity: MEDIUM
Class: Platform divergence (a valid key is refused; the diagnostic names nothing)

Status: Fixed (plan-110-F Phase 2)
Regression Test:
`codegen::builtins::tls::gen_schannel::schannel_tests::listen_accepts_a_pkcs1_key_not_only_pkcs8`,
plus execution proof on box 2230.

## Symptom

On Windows, `tls::listen` with a traditional PKCS#1 key raises:

```
Error: 7-707-0008
TLS handshake, certificate validation, SNI validation, or protocol operation failed.
```

The message names neither the key nor its encoding, and the same file works on macOS and Linux.

Measured on box 2230 (Windows 11, 10.0.26100.9168), same cert, same program, only the key encoding
differing:

| key PEM | macOS | Linux (OpenSSL) | Windows (Schannel) |
|---|---|---|---|
| `-----BEGIN PRIVATE KEY-----` (PKCS#8) | serves | serves | serves |
| `-----BEGIN RSA PRIVATE KEY-----` (PKCS#1) | serves | serves | **7-707-0008 at listen** |

PKCS#1 is what `openssl rsa -traditional` emits and what a great deal of existing server
configuration ships.

## Root cause

`gen_schannel_server.rs` loads the key as PEM → DER → PKCS#8 unwrap → RSA key:

```
CryptDecodeObjectEx(PKCS_PRIVATE_KEY_INFO = 44, der, ...)   -> WORK.PKINFO
CryptDecodeObjectEx(PKCS_RSA_PRIVATE_KEY  = 43, pkInfo->PrivateKey, ...) -> WORK.KBLOB
```

The first call was treated as mandatory: a `FALSE` return branched to the shared TLS failure exit.
A PKCS#1 DER is not a `PrivateKeyInfo`, so it never decodes there — even though it already **is**
the `RSAPrivateKey` the second call wants.

## Fix

Make the PKCS#8 unwrap a *try*. Both encodings stage the RSA DER into two frame slots and join at
one `PKCS_RSA_PRIVATE_KEY` decode:

* PKCS#8 → `pkInfo->PrivateKey` `{pbData@40, cbData@32}`
* PKCS#1 → the file's own `DERBUF`/`DERLEN`

A genuinely malformed key still fails, now at the second decode, on either path. `WORK.PKINFO`
stays NULL on the PKCS#1 path and nothing later reads it.

Verified on box 2230 after the fix: a PKCS#1 key listens, completes a real handshake with
`openssl s_client -CAfile`, and echoes the payload; a PKCS#8 key still does the same, so the fix
adds a path rather than swapping one.

`tls::listen`'s descriptor now documents both accepted encodings, so the portable choice does not
have to be discovered by experiment.
