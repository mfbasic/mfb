# bug-863: `crypto::open(cipher, key, nonce, sealed, aad)` — the documented Sealed+aad overload fails to compile with an unlocated TYPE_CALL_ARGUMENT_MISMATCH

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/crypto/crypto-open-sealed-aad-rt (to add)

`crypto::open` overload 2 is `open(cipher AS SymmetricCipher, key AS List OF Byte, nonce AS
List OF Byte, sealed AS crypto::Sealed, [aad AS List OF Byte])`. Calling it **with** the
optional `aad` (the only way to open a box sealed with associated data through the
`Sealed` record) does not compile. The compiler pads the call to six arguments, then
rejects it, and the diagnostic has no file/line.

Correct behavior: the call compiles and returns the plaintext (or raises
`ErrAuthenticationFailed` on a wrong `aad`), exactly like the explicit
`open(cipher, key, nonce, box.ciphertext, box.tag, aad)` form.

References:

- src/docs/spec/stdlib/10_crypto.md §AEAD: "`aad` defaults to empty"
- `mfb man crypto open`, Overloads: "2. `crypto::open(cipher AS crypto::SymmetricCipher,
  key AS List OF Byte, nonce AS List OF Byte, sealed AS crypto::Sealed, [aad AS List OF Byte])`"

## Failing Reproduction

`/tmp/hardening/B04/openaad/src/main.mfb`:

```
IMPORT io
IMPORT crypto
IMPORT strings

FUNC main() AS Nothing
  LET k AS List OF Byte = crypto::randomBytes(32)
  LET n AS List OF Byte = crypto::randomBytes(12)
  LET aad AS List OF Byte = strings::toBytes("hdr")
  LET box AS crypto::Sealed = crypto::seal(crypto::SymmetricCipher.AES256GCM, k, n, "hi", aad)
  LET clear AS List OF Byte = crypto::open(crypto::SymmetricCipher.AES256GCM, k, n, box, aad)
  io::print(toString(clear))
END FUNC
```

```
/tmp/hardening/mfb build /tmp/hardening/B04/openaad
```

- Observed (exit 1, no source location):
  ```
  error: TYPE_CALL_ARGUMENT_MISMATCH: Call to `crypto.open` has argument type(s) (crypto.SymmetricCipher, List OF Byte, List OF Byte, crypto.Sealed, List OF Byte, List OF Byte), expected crypto::SymmetricCipher, List OF Byte, List OF Byte, (List OF Byte, List OF Byte or crypto::Sealed)[, List OF Byte].
  ```
- Expected: builds; prints `hi`.

Contrast (all work, from `/tmp/hardening/B04/crx`): `open(cipher, k, n, sealed)` (4 args);
`open(cipher, k, n, ct, tag)` and `open(cipher, k, n, ct, tag, aad)`. Every AES-GCM and
ChaCha20-Poly1305 seal/open vector matched python `cryptography` 49.0.0.

## Root Cause

`src/codegen/registry/mod.rs:default_argument_padding` chooses which overload's trailing
`Fill` defaults to append by **first-argument type only**
(`param.ty.name() == argument.name()`) among overloads with
`params.len() >= provided`. Both `crypto::open` overloads start with `SymmetricCipher` and
both have ≥ 5 params. For the 5-argument Sealed+aad call it picks overload 1 (ciphertext,
tag, aad: 6 params) and appends its `aad` default, giving 6 arguments with a `Sealed` in
position 4. No overload accepts that. The function's own comment names this exact member
("`crypto::open`'s AEAD form has an exact-arity sibling at 5 arguments whose `aad` must
still be padded"), so it handles the ct/tag form and breaks the Sealed form. The selection
needs all provided argument types (or the overload the type checker actually selected), not
just the first. The missing location is a second defect: this padding-induced mismatch is
reported without the call's span.

## Goal

- The repro builds and prints `hi`; a wrong `aad` raises 77050016.
- The 5-arg `open(cipher, k, n, ct, tag)` call is still padded with empty `aad`.
- The diagnostic for any padding mismatch carries file:line.

### Non-goals (must NOT change)

- `tls::connect` Address/host padding (plan-110-D) keeps working.

## Blast Radius

- `default_argument_padding` serves every registry member with optional trailing params and ≥ 2 overloads sharing a first-parameter type. Grep descriptors for `DefaultValue::Fill` on multi-overload members; `crypto::seal` (List OF Byte vs String data, both 5 params) also shares a first type but its overloads have equal shape, so they are unaffected (verified: `seal(..., aad)` with String data compiles).
