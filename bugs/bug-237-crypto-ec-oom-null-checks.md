# bug-237: crypto_ec sign/generate paths deref allocation results without a NULL check (OOM crash) + stale OpenSSL version claim

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: memory-safety / docs

Status: Open

A small cluster of OOM-only NULL-pointer derefs where an allocation-returning C
API result is used without a NULL check — one a proven asymmetry (verify checks
it, sign does not):

- OpenSSL (`src/target/shared/code/crypto_ec/openssl.rs`): `sign()` stores the
  `EVP_MD_CTX_new` result and feeds it to `EVP_DigestSignInit` with no NULL check
  (`:1105-1110`), and `generate()` feeds an unchecked `EVP_PKEY_new` result to
  `EVP_PKEY_assign` (`:630-635`). Both return NULL only on malloc failure and the
  callee dereferences immediately. `verify()` (`:1522-1529`) already added the
  exact `EVP_MD_CTX_new` NULL check (bug-136); sign/generate were missed. Fix:
  after each call, `cmp` against 0 and `branch_eq` to the existing error label.
- macOS (`src/target/shared/code/crypto_ec/macos.rs:513-523`): the
  `CFNumberCreate` result is stored into the attributes-dict value array and
  handed to `CFDictionaryCreate` (with `kCFTypeDictionaryValueCallBacks`) without
  a NULL check → the value-retain callback runs `CFRetain(NULL)` under memory
  pressure. Fix: NULL-check `NUM` before placing it in the dictionary.
- Docs (`src/target/shared/code/crypto_ec/openssl.rs:4-8`): the header says the
  one-shot `EVP_DigestSign`/`EVP_DigestVerify` are "present and non-deprecated on
  both OpenSSL 1.1 and 3.x", but those symbols arrived in 1.1.1, not 1.1.0 (fails
  closed via `load_fail` on a 1.1.0 build). Fix: narrow to "1.1.1 and 3.x".
