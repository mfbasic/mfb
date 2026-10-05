# bug-871: `crypto::hmac(crypto::Hash.SHA1, key, "text")` reports CRYPTO_SHA1_INSECURE — the String-data overload misses the sound-use exemption

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/crypto (the SHA-1 warning-scope fixture: add the String-data hmac case)

The spec exempts a direct `Hash.SHA1` selector of `hmac`/`hkdf`/`pbkdf2` from the
`CRYPTO_SHA1_INSECURE` warning. That holds for `hmac(SHA1, key, bytes)`, `pbkdf2` and
`hkdf`, but `hmac(SHA1, key, "a string")`, the String-data overload (RFC 6238 TOTP code
often passes text), still warns.

Correct behavior: no warning for a direct `crypto::Hash.SHA1` in the selector position of
either `hmac` overload.

References:

- src/docs/spec/stdlib/10_crypto.md §Algorithm set: "`Hash.SHA1` written directly as the
  hash selector of `hmac`, `hkdf` or `pbkdf2` reports nothing … Every other occurrence
  reports, `hash` included."

## Failing Reproduction

`/tmp/hardening/B04/sha1w/src/main.mfb`:

```
IMPORT io
IMPORT crypto
IMPORT strings

FUNC main() AS Nothing
  LET k AS List OF Byte = strings::toBytes("k")
  LET a AS List OF Byte = crypto::hmac(crypto::Hash.SHA1, k, k)
  LET b AS List OF Byte = crypto::hmac(crypto::Hash.SHA1, k, "text")
  io::print(toString(len(a) + len(b)))
END FUNC
```

```
/tmp/hardening/mfb build /tmp/hardening/B04/sha1w 2>&1 | grep warn
```

- Observed: `sha1w/src/main.mfb:8 warn[2-203-0136 CRYPTO_SHA1_INSECURE]: SHA-1 is not collision-resistant; use it only for legacy interoperability` (line 8 = the String overload; line 7 is silent).
- Expected: no warning.

Contrast (an earlier version of the same file): `pbkdf2(Hash.SHA1, …)` and
`hkdf(Hash.SHA1, …)` are silent. `crypto::hash(Hash.SHA1, …)`, `LET h AS crypto::Hash =
crypto::Hash.SHA1`, and `RETURN crypto::Hash.SHA1` all warn, as specified.

## Root Cause

`src/ir/verify/values.rs:hash_selector_use_is_sound` matches the call target's member
name against `"hmac" | "hkdf" | "pbkdf2"`. The String overload of `crypto::hmac` is a
`Body::Rewrite("__crypto_hmacText")`
(`src/codegen/builtins/crypto/func_hmac.rs:register`), so at verify time the target's
member is `hmacText` and the exemption does not apply. Add `hmacText` (or match on the
public member rather than the rewrite target).

## Goal

- Line 8 of the repro builds with no warning; `crypto::hash(Hash.SHA1, …)` still warns.

### Non-goals (must NOT change)

- Nested selectors (`hmac(pick(SHA1), …)`) still warn, per bug-517.

## Blast Radius

- Any other rewrite-target overload of the three members: `pbkdf2`/`hkdf` have single overloads (verified silent).
