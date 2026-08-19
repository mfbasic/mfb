# bug-446: Windows CNG ECDSA `p*Verify` always returns FALSE for a valid signature

Last updated: 2026-08-18
Effort: medium (2h–4h)
Severity: HIGH
Class: Correctness (valid signature rejected on one platform)

Status: Open
Regression Test: tests/rt-behavior/crypto/crypto-ec-valid (needs a windows-x86_64
`.run` golden — today it has ncodesum goldens only for linux/macos, so this bug
is invisible to the harness)

`crypto::p256Verify` / `p384Verify` / `p521Verify` return **FALSE for a signature
that is provably valid** on the Windows (CNG/BCrypt) backend. macOS (SecKey) and
Linux (OpenSSL) verify the same inputs TRUE. This means the entire ECDSA verify
surface is broken on Windows — it was never runtime-verified there (the
`crypto-ec-valid` acceptance fixture carries `.run`/ncodesum goldens for
linux-aarch64/riscv64/x86_64 and macos-aarch64, but **no windows-x86_64 golden**).

## Reproduction (isolated, verify-only — no generate/sign involved)

A fixed OpenSSL-produced P-256 public key + DER signature over "The quick brown
fox" (SHA-256). Verified TRUE on macOS/Linux; FALSE on Windows 11 x86_64 (test
host `2230`).

```mfb
IMPORT crypto
IMPORT encoding
IMPORT io
SUB main()
  LET msg AS List OF Byte = encoding::utf8Encode("The quick brown fox")
  LET pk AS List OF Byte = encoding::hexDecode("0427468253cdc25e0981777ba33c64558e1b7e22c4296611d2eea105f657a59e84c6e356f96f185b6f0342161aa92fa920433e28577c2433542b1c4c3d1bc1d58e")
  LET sig AS List OF Byte = encoding::hexDecode("304402200eb0ca5a0f492af525dfc4f5a6547cd83f1c9e478c749f72f46bbb8c6bf3452b02200764fdc09e9a1be5d134f5578ba96c2b966e3a7b13686615aca8d427e6ae6b2f")
  io::print(toString(crypto::p256Verify(pk, msg, sig)))  ' macOS/Linux: TRUE, Windows: FALSE
END SUB
```

## What is NOT the bug (ruled out)

- **generate and sign are correct on Windows.** A Windows-produced
  `crypto::generateP256()` key + `crypto::p256Sign()` signature verifies TRUE when
  the exact (public-key, signature) bytes are re-checked on macOS. So the CNG
  keygen and signing paths produce valid, standards-conformant output.
- **SHA-256 is correct on Windows** (`crypto::sha256("abc")` matches the KAT), so
  the digest fed to `BCryptVerifySignature` is not the problem in isolation.
- **DER decoding of the simplest form fails too.** The KAT above uses a short-form
  SEQUENCE with two 32-byte INTEGERs that need no leading-zero trim — the simplest
  possible path through `der_decode_int` — and still returns FALSE, so the DER
  long-form / pad-strip logic is not implicated.
- **It is not a missing call-site shadow-space frame** (that was a separate,
  genuine Win64 ABI bug in `bcrypt_call` — see below — whose fix made generate/sign
  produce valid output, but verify remained FALSE).

## Fixed as a prerequisite (already committed): `bcrypt_call` shadow space

`src/codegen/builtins/crypto/native/cng.rs::bcrypt_call` reserved the 32-byte
Win64 shadow (home) space only for calls with **more than 4** arguments; the ≤4-arg
path (`BCryptOpenAlgorithmProvider`, `BCryptGenerateKeyPair`,
`BCryptFinalizeKeyPair`, `BCryptDestroyKey`, `BCryptCloseAlgorithmProvider`)
emitted a bare `call` with no `subtract_stack`. Win64 requires **every** caller to
reserve ≥32 bytes below the outgoing args, or the callee clobbers the caller's
`[sp .. sp+0x20]` locals when it homes its register args. This corrupted the
generate/sign frames. Fixed by always reserving `(0x20 + stack*8)`-aligned space.
This is necessary but **not sufficient** — verify is still FALSE after it.

## Remaining suspects (verify-only codepath)

Since generate/sign/hash are proven good and DER decode of the simple form is
exercised, the defect is isolated to what `verify()` does that `sign()` does not:
`import_key(is_private=false)` building the `BCRYPT_ECCPUBLIC_BLOB`, or the
`BCryptVerifySignature` call itself, or a verify-specific register/stack
interaction. On inspection all of these look correct, so the next step is an
`.ncode`/objdump trace of a Windows `p256Verify` to localize (per
`register-slot-import-bugs-need-codegen-inspection`).

References: found while implementing the clean-room `crypto::generate`
(`func_generate.rs`); the Windows generate path is proven correct via
cross-platform verification despite this pre-existing verify defect.
