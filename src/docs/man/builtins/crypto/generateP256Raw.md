# generateP256Raw

Generate a random NIST P-256 private key as raw bytes (FIPS 186).

## Synopsis

```
crypto::generateP256Raw() AS List OF Byte
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

`crypto::generateP256Raw` creates a fresh ECDSA private key over the NIST P-256
curve (FIPS 186) and returns it as a flat `List OF Byte`, with no surrounding
record. It takes no arguments. [[src/builtins/crypto.rs:GENERATE_P256_RAW]]

The returned bytes are 97 bytes in the wire form `0x04 || X || Y || K`: the SEC1
uncompressed public point (`0x04` tag, 32-byte `X`, 32-byte `Y`) followed by the
32-byte big-endian private scalar `K`. This is the same self-contained private
form that `crypto::p256Sign` consumes and that `crypto::generateP256` stores in
its `privateKey` field. [[src/builtins/crypto_package.mfb:__crypto_generateP256]]

`crypto::generateP256Raw` is the low-level native helper underlying
`crypto::generateP256`: the higher-level wrapper calls this function and then
slices the leading 65 bytes (`0x04 || X || Y`) into the `publicKey` field of a
`crypto::KeyPair`. Prefer `crypto::generateP256` when you want a structured key
pair; use `crypto::generateP256Raw` when you only need the raw private bytes.
[[src/builtins/crypto_package.mfb:__crypto_bytePrefix]]

The key is produced by binding the platform key API — `SecKey` on macOS,
`EVP_PKEY`/`EC_KEY` on Linux (OpenSSL) — while the wire encoding is identical
across macOS and Linux and is interoperable: a key produced on one platform is
accepted on the other and by OpenSSL/pyca. [[src/target/shared/code/crypto_ec.rs:ec_call]]

The secret scalar is drawn from the platform CSPRNG, so the result is random and
non-reproducible: every call yields a different key. There is no seeded or
deterministic form; to persist a key, store the returned bytes yourself.

**Secret safety.** The returned bytes embed the secret scalar `K`. Anyone who
holds them can forge signatures. Never log them, and treat `typeName` /
`toString` / diagnostics as non-security boundaries. To display or store a key,
stringify its bytes with `encoding::hexEncode` (lowercase hex) or
`encoding::base64Encode`.

## Parameters

None.

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The 97-byte `0x04 \|\| X \|\| Y \|\| K` private form: the SEC1 uncompressed public point followed by the 32-byte big-endian private scalar. [[src/builtins/crypto.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050000` | `ErrUnknown` | The platform key API fails to load or key generation fails (e.g. `SecKeyCreateRandomKey` / `EC_KEY_generate_key` returns an error). [[src/target/shared/code/crypto_ec/macos.rs:generate]] [[src/target/shared/code/crypto_ec/openssl.rs:generate]] |
| `77010001` | `ErrOutOfMemory` | An arena allocation for the key bytes fails. [[src/target/shared/code/crypto_ec.rs:emit_fail]] |

## Examples

Generate a raw P-256 private key and print it as hex:

```
IMPORT crypto
IMPORT encoding

LET priv AS List OF Byte = crypto::generateP256Raw()
PRINT encoding::hexEncode(priv)
```

Sign a message directly with the raw private bytes:

```
IMPORT crypto

LET priv AS List OF Byte = crypto::generateP256Raw()
LET sig AS List OF Byte = crypto::p256Sign(priv, message)
```

## See also

- `mfb man crypto generateP256`
- `mfb man crypto p256Sign`
- `mfb man crypto p256Verify`
- `mfb man crypto generateP384`
- `mfb man encoding hexEncode`
