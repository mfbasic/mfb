# bug-864: `tls::listen` / `http::serverSSL` fail with ErrTlsFailed for an EC (P-256) PKCS#8 key on macOS

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (owner-confirmed: add EC + Ed25519 PKCS#8 support, unwrap on macOS)

Status: Open
Coordinator note: repro measured by the review agent only; the coordinator did not re-run it.
Regression Test: tests/rt-behavior/tls/tls-listen-ec-key-rt (to add, macOS)

## Cause (coordinator, bug-staging-24)

- Rule: EC (P-256/P-384) and Ed25519 PKCS#8 keys work as TLS server keys on every target (owner decision 2026-10-05: add support; macOS unwraps PKCS#8 `PrivateKeyInfo` to the format `SecItemImport` accepts; Linux and Windows get the same treatment wherever their loader needs it; documented in `mfb man tls listen` / `http::serverSSL`).
- Where: `src/codegen/builtins/tls/gen_macos/server.rs:emit_import_pem_item` calls `SecItemImport` with a NULL format (rejects EC/Ed25519 PKCS#8, measured); Windows imports `PKCS_RSA_PRIVATE_KEY` into `PROV_RSA_AES` only.
- Fix: one PKCS#8 key-loading step per platform before import: decode `PrivateKeyInfo`, read the algorithm OID, and hand the platform the key in the form it imports (macOS: SEC1 `ECPrivateKey` for id-ecPublicKey, Ed25519 as the platform requires; Windows: the CNG/NCrypt key blob for EC); RSA unchanged.

**Owner decision (2026-10-05): add support.** EC (P-256/P-384) and Ed25519 PKCS#8 keys must
work as TLS server keys. On macOS the listener unwraps PKCS#8 `PrivateKeyInfo` to the
older format `SecItemImport` accepts (SEC1 `ECPrivateKey` for id-ecPublicKey) before
import. The unwrapping is documented in `mfb man tls listen` (and `http::serverSSL`).
Linux (OpenSSL) and Windows (Schannel) get the same treatment wherever their loader
needs it. Windows today is RSA-only (`PKCS_RSA_PRIVATE_KEY` into `PROV_RSA_AES`), so it
needs an EC (and Ed25519) import path, not just unwrapping.

Open risk for the implementer (not yet measured): Ed25519 has no pre-PKCS#8 "old
format" to unwrap to, and Apple's TLS stack may not accept an Ed25519 server identity
at all. If it cannot be supported on a target, that is a blocker to report back to the
owner, not a silent `ErrTlsFailed`.

**Owner rule (2026-10-05): parity is the requirement.** Whatever key formats are
supported must be supported on **every** platform (macOS, Linux, Windows). A key type
that works on one target and fails on another is the bug, whichever target is the odd
one out. So if Ed25519 (or any EC curve) cannot be made to work on one platform, it is
documented and rejected as unsupported on all of them, with one consistent error. It
is never left as a per-platform difference. The man page and
`mfb spec stdlib transports` list exactly the supported set, and a regression fixture
proves each supported format on every target.

On macOS AArch64, an `openssl req -newkey ec` P-256 self-signed certificate with its
unencrypted PKCS#8 key (`-----BEGIN PRIVATE KEY-----`) makes `http::serverSSL`
(a pass-through to `tls::listen`) raise `ErrTlsFailed` (77070008) before any client
connects. The same command with `-newkey rsa:2048` works. The spec and `mfb man tls
listen` say a PKCS#8 key is accepted "on every platform" and name no key-algorithm
restriction. ECDSA server certificates are the modern default (Let's Encrypt issues them),
so this blocks a common deployment.

Correct behavior is an owner decision (class UNCLEAR). Either EC keys load and serve, or
the spec and man state RSA-only and the error says so. Today the code rejects the key
while the docs promise PKCS#8 support.

References:

- src/docs/spec/stdlib/17_transports.md: "**The key may be PKCS#8 or PKCS#1**, on every
  platform, unencrypted. One PEM therefore serves every target."
- src/docs/spec/stdlib/05_http.md §Lifecycle: `serverSSL` "works on both Linux and macOS".
- Found by batch B04 (http topic) — transports is batch B06's topic; cross-check there.

## Failing Reproduction

```
mkdir -p /tmp/hardening/B04/tlsec && cd /tmp/hardening/B04/tlsec
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout key.pem -out cert.pem -days 2 -subj "/CN=127.0.0.1" -addext "subjectAltName=IP:127.0.0.1"
head -1 key.pem     # -----BEGIN PRIVATE KEY-----
```

`/tmp/hardening/B04/tlssrv2/src/main.mfb` (the `tlssrv` program with the paths pointed at `tlsec/`):

```
RES s AS tls::Listener = http::serverSSL(18443, "/tmp/hardening/B04/tlsec/cert.pem", "/tmp/hardening/B04/tlsec/key.pem", "127.0.0.1")
```

```
./tlssrv2/build/tlssrv2.out &  ; sleep 2 ; pgrep -fl tlssrv2.out || echo dead-before-client
```

- Observed (no client ever connected):
  ```
  dead-before-client
  Error: 7-707-0008
  TLS handshake, certificate validation, SNI validation, or protocol operation failed.
  ```
- Expected: the listener binds and `curl -sk https://127.0.0.1:18443/` gets a 200 (works
  with the RSA pair from the same `openssl` invocation, `-newkey rsa:2048`).

| Environment | key | Result |
| --- | --- | --- |
| macOS 15 (Darwin 24.6) aarch64, /tmp/hardening/mfb | RSA-2048 PKCS#8 | works ✓ |
| macOS 15 (Darwin 24.6) aarch64, /tmp/hardening/mfb | EC P-256 PKCS#8 | fails ✗ |
| Linux (OpenSSL backend) | EC P-256 | not run |

The EC pair itself is valid: `openssl s_server -accept 18444 -cert cert.pem -key key.pem -www` serves it, and `curl -sk https://127.0.0.1:18444/` returns `200`.

## Root Cause

**Measured by the coordinator (2026-10-05).** The TLS listeners do not use the `crypto`
package. They hand the PEM to the OS TLS stack. On macOS that is Security.framework's
`SecItemImport`, called by `src/codegen/builtins/tls/gen_macos/server.rs:emit_import_pem_item`
with a NULL format and item type. A standalone C probe
(`/tmp/hardening/coord/sec/t.c`) that makes the same call shows `SecItemImport` itself
cannot read EC or Ed25519 PKCS#8:

| key file | `SecItemImport` (NULL format) | (`kSecFormatOpenSSL` + `kSecItemTypePrivateKey`) |
| --- | --- | --- |
| EC P-256, SEC1 `BEGIN EC PRIVATE KEY` | 0 (ok) | 0 (ok) |
| EC P-256, PKCS#8 `BEGIN PRIVATE KEY` | -25257 `errSecUnknownFormat` | -25257 |
| RSA, PKCS#8 | 0 (ok) | -25257 |
| Ed25519, PKCS#8 | -25257 | -25257 |

So the macOS failure is an Apple import limitation the listener doesn't work around,
not a missing key type in `crypto`. `crypto` already builds EC keys on macOS via
`SecKeyCreateWithData` (`src/codegen/builtins/crypto/func_sign.rs`, curve-specific
PKCS#8 templates in `gen_cert.rs:PKCS8_TEMPLATES`). One possible fix is to unwrap the
PKCS#8 `PrivateKeyInfo` for the id-ecPublicKey OID to its inner SEC1 `ECPrivateKey`
before import. Windows (`gen_schannel_server.rs`) is RSA-only by construction
(`PKCS_RSA_PRIVATE_KEY` into `PROV_RSA_AES`). Ed25519 needs an owner decision on whether
it is supported at all as a TLS server key.

### Earlier hypothesis (B04 agent)

Not localized. Hypothesis: the macOS listener (`src/codegen/builtins/tls/gen_macos/server.rs`)
imports the key with `SecItemImport` and builds the identity with `SecIdentityCreate`. The
PKCS#8-unwrapping path that bug-461-style work added is probably RSA-only (it strips to
PKCS#1 / imports as `kSecAttrKeyTypeRSA`), so an EC PKCS#8 key fails import. To confirm,
trace the import's OSStatus for the EC file.

## Goal

- An EC P-256 PKCS#8 key either serves (preferred), or is rejected with a documented, specific error, and the spec/man say so.

### Non-goals (must NOT change)

- RSA PKCS#1/PKCS#8 loading.

## Blast Radius

- `tls::listen` (macOS backend) and so `http::serverSSL`. Schannel/OpenSSL backends: not run.

## Addendum (batch B06): key-algorithm matrix


Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (owner-confirmed: add EC + Ed25519 PKCS#8 support, unwrap on macOS)

Status: Open
Regression Test: tests/rt-behavior/tls/tls-listen-ec-key-rt (as bug-864), plus a key-encoding matrix fixture

This confirms bug-864 and widens it. The macOS failure is not "EC keys fail". It is
"**PKCS#8 works only for RSA**": an EC key in SEC1 form (`-----BEGIN EC PRIVATE
KEY-----`) loads and serves, while the identical key re-wrapped as PKCS#8 fails at
listen with `ErrTlsFailed`. On Windows, code inspection shows the Schannel listener
decodes the key with `PKCS_RSA_PRIVATE_KEY` into a `PROV_RSA_AES` container, so no
EC key in any encoding can work there. Taken together, no EC PEM exists that works on
both macOS and Windows. That contradicts the spec's promise that "One PEM therefore
serves every target".

Correct behavior is an owner decision (UNCLEAR, as in bug-864). Option (a): every
backend accepts RSA and EC (P-256/P-384) in both PKCS#8 and the traditional
encoding. Option (b): the spec and man state the supported matrix (e.g. "RSA only",
or "EC as SEC1 on macOS/Linux, unsupported on Windows"), and an unsupported key
raises an error that names the key, not the generic handshake message.

References:

- src/docs/spec/stdlib/17_transports.md §TLS specifics: "**The key may be PKCS#8 or PKCS#1**, on every platform, unencrypted. One PEM therefore serves every target."
- `mfb man tls listen` (src/codegen/builtins/tls/func_listen.rs DESC): "The key may be in either PEM encoding, on every platform: PKCS#8 … or the traditional PKCS#1".
- bugs/bug-864-https-listen-rejects-ec-pkcs8-key-macos.md (B04's original).

#### Failing Reproduction

```
cd /tmp/hardening/B06/certs
X="-addext subjectAltName=IP:127.0.0.1,DNS:localhost -addext extendedKeyUsage=serverAuth"
openssl req -x509 -newkey rsa:2048 -nodes -keyout rsa8.key -out rsa.crt -days 30 -subj /CN=localhost $X
openssl rsa -in rsa8.key -traditional -out rsa1.key
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout ec8.key -out ec.crt -days 30 -subj /CN=localhost $X
openssl ec -in ec8.key -out ec1.key                       # SEC1: BEGIN EC PRIVATE KEY
openssl pkcs8 -topk8 -nocrypt -in ec1.key -out ec8b.key   # same key back to PKCS#8
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-384 -nodes -keyout ec384.key -out ec384.crt -days 30 -subj /CN=localhost $X
openssl ec -in ec384.key -out ec384s.key
openssl req -x509 -newkey ed25519 -nodes -keyout ed.key -out ed.crt -days 30 -subj /CN=localhost $X
```

Server `/tmp/hardening/B06/tlssrv` (`tls::listen("127.0.0.1", port, cert, key)` → `tls::accept(l, 4000)` → read → write "pong"); driven by `/tmp/hardening/B06/tlsprobe.sh <crt> <key> <port>` with `openssl s_client -connect 127.0.0.1:<port> -servername localhost -quiet` as the client.

| cert / key | key PEM header | Result (macOS 15 aarch64) |
| --- | --- | --- |
| rsa.crt / rsa8.key | BEGIN PRIVATE KEY | served, client got `pong` ✓ |
| rsa.crt / rsa1.key | BEGIN RSA PRIVATE KEY | served ✓ |
| ec.crt / ec1.key | BEGIN EC PRIVATE KEY | served ✓ |
| ec384.crt / ec384s.key | BEGIN EC PRIVATE KEY | served ✓ |
| ec.crt / ec8.key | BEGIN PRIVATE KEY | `listen ERR:77070008` ✗ |
| ec.crt / ec8b.key (ec1.key → PKCS#8) | BEGIN PRIVATE KEY | `listen ERR:77070008` ✗ |
| ec384.crt / ec384.key | BEGIN PRIVATE KEY | `listen ERR:77070008` ✗ |
| ed.crt / ed.key (Ed25519) | BEGIN PRIVATE KEY | `listen ERR:77070008` ✗ |

- Observed: `listen ERR:77070008 TLS handshake, certificate validation, SNI validation, or protocol operation failed.` (no client ever connected).
- Expected: per the spec, PKCS#8 is accepted "on every platform".

On the client side, `tls::connect(..., allowSelfSigned := TRUE)` to an `openssl s_server`
presenting `ec.crt` or `ec384.crt` succeeds on macOS. Ed25519 fails (`ERR:77070008`),
which is a Network.framework limit (`/tmp/hardening/B06/clprobe.sh`).

A mismatched pair across algorithms (`rsa.crt` + `ec1.key`, `ec.crt` + `rsa8.key`) is
accepted at listen and fails at the first `accept` (`accept ERR:77070008`). That
matches the spec's "mismatch surfaces at the first accept" claim.

#### Root Cause

- macOS: `src/codegen/builtins/tls/gen_macos/server.rs:emit_import_pem_item` calls `SecItemImport(data, NULL, NULL, NULL, 0, NULL, NULL, &items)` with no input format or item type. Security.framework's PEM import recognizes `RSA PRIVATE KEY`, `EC PRIVATE KEY` and PKCS#8 `PRIVATE KEY` for RSA, but fails an EC/Ed25519 PKCS#8 blob. Hypothesis to confirm: log the `OSStatus` for `ec8.key` (likely `errSecUnknownFormat` -25257). Possible fix: detect `BEGIN PRIVATE KEY`, unwrap the PKCS#8 `PrivateKeyInfo` and, for the id-ecPublicKey OID, import the inner ECPrivateKey with `SecKeyCreateWithData`/`kSecAttrKeyTypeECSECPrimeRandom` (or re-wrap as SEC1).
- Windows (code only, not run): `src/codegen/builtins/tls/gen_schannel_server.rs` (key load in `lower_tls_listen`) decodes `PKCS_PRIVATE_KEY_INFO` then `PKCS_RSA_PRIVATE_KEY=43` and imports into `PROV_RSA_AES` (`CryptAcquireContextW(..., PROV_RSA_AES, ...)`). An EC key fails the RSA decode in both encodings.
- Linux (OpenSSL): not run.

#### Goal

- The spec/man key-encoding claim matches a measured matrix on every backend, and each unsupported combination raises a specific, documented error.

##### Non-goals (must NOT change)

- RSA PKCS#1/PKCS#8 loading on all three backends (bug-461).

#### Blast Radius

- `tls::listen` on macOS and Windows; `http::serverSSL` via pass-through.
- `tls::connect` client trust of EC server certificates — unaffected on macOS (measured).
