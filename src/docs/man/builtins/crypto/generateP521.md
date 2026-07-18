# generateP521

Generate a random NIST P-521 ECDSA key pair (FIPS 186).

## Synopsis

```
crypto::generateP521() AS crypto::KeyPair
```

## Package

crypto

## Imports

```
IMPORT crypto
```

`crypto` is a built-in package, so no manifest dependency is required.
[[src/builtins/crypto.rs:augmented_project]]

## Description

`crypto::generateP521` creates a fresh ECDSA key pair over the NIST P-521 curve
(FIPS 186) for use with `crypto::p521Sign` and `crypto::p521Verify`. It takes no
arguments and returns a `crypto::KeyPair` record with two fields:
[[src/builtins/crypto_package.mfb:__crypto_generateP521]]

- `privateKey` — 199 bytes, the wire form `0x04 || X || Y || K`: the SEC1
  uncompressed public point (`0x04` tag, 66-byte `X`, 66-byte `Y`) followed by
  the 66-byte big-endian private scalar `K`. It is self-contained and is what
  `crypto::p521Sign` consumes. [[src/builtins/crypto_package.mfb:__crypto_generateP521]]
- `publicKey` — 133 bytes, the wire form `0x04 || X || Y`: the leading SEC1
  uncompressed public point, sliced from the private bytes.
  [[src/builtins/crypto_package.mfb:__crypto_bytePrefix]]

The key is produced by a native raw keygen helper that binds the platform key
API — `SecKey` on macOS, `EVP_PKEY`/`EC_KEY` on Linux (OpenSSL) — while the
public/private wire encodings above are identical across macOS and Linux and are
interoperable: a key produced on one platform is accepted on the other and by
OpenSSL/pyca. [[src/target/shared/code/crypto_ec.rs:ec_call]]

The secret scalar is drawn from the platform CSPRNG, so the result is random and
non-reproducible: every call yields a different key pair. There is no seeded or
deterministic form; to persist a key, store the returned bytes yourself.

**Secret safety.** The `privateKey` field embeds the secret scalar `K`. Anyone
who holds it can forge signatures. Never log a `KeyPair`, and treat `typeName` /
`toString` / diagnostics as non-security boundaries. The `publicKey` is safe to
share; distribute it to verifiers.

To display or store a key, stringify its bytes with `encoding::hexEncode`
(lowercase hex) or `encoding::base64Encode`.

## Parameters

None.

## Return value

| Type | Description |
| --- | --- |
| `crypto::KeyPair` | A record whose `privateKey` is the 199-byte `0x04 \|\| X \|\| Y \|\| K` form and whose `publicKey` is the 133-byte `0x04 \|\| X \|\| Y` SEC1 uncompressed point. [[src/builtins/crypto_package.mfb:KeyPair]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050000` | `ErrUnknown` | The platform key API fails to load or key generation fails (e.g. `SecKeyCreateRandomKey` / `EC_KEY_generate_key` returns an error). [[src/target/shared/code/crypto_ec/macos.rs:generate]] [[src/target/shared/code/crypto_ec/openssl.rs:generate]] |
| `77010001` | `ErrOutOfMemory` | An arena allocation for the key bytes (or for the byte list built while slicing out the public key) fails. [[src/target/shared/code/crypto_ec/macos.rs:generate]] [[src/target/shared/code/crypto_ec/openssl.rs:generate]] [[src/builtins/crypto_package.mfb:__crypto_bytePrefix]] |

## Examples

Generate a key pair and print the public key as hex:

```
IMPORT crypto
IMPORT encoding

LET kp AS crypto::KeyPair = crypto::generateP521()
PRINT encoding::hexEncode(kp.publicKey)
```

Sign a message with the freshly generated key:

```
IMPORT crypto

LET kp AS crypto::KeyPair = crypto::generateP521()
LET sig AS List OF Byte = crypto::p521Sign(kp.privateKey, message)
```

## See also

- `mfb man crypto p521Sign`
- `mfb man crypto p521Verify`
- `mfb man crypto generateP256`
- `mfb man crypto generateP384`
- `mfb man encoding hexEncode`
