# bug-447: Windows `crypto::randomBytes` raises ErrUnknown (BCryptGenRandom path)

Last updated: 2026-08-18
Effort: medium (2h–4h)
Severity: HIGH
Class: Correctness (a core primitive fails on one platform)

Status: Open
Regression Test: tests/rt-behavior/crypto/crypto-randomint-wide-range-rt (needs a
windows-x86_64 `.run` golden — today the crypto runtime fixtures carry `.run`
goldens for linux/macos only, so this is invisible to the harness)

`crypto::randomBytes(n)` raises `ErrUnknown` ("Unclassified standard-package
failure.", code 77050000) on Windows 11 x86_64 (test host `2230`). macOS/Linux
(getentropy) return the bytes. Because `randomBytes` is the entropy source for
`crypto::randomInt`, the ed25519 keygen, and any user CSPRNG use, the whole
random surface is dead on Windows.

## Reproduction

```mfb
IMPORT crypto
IMPORT io
SUB main()
  LET r AS List OF Byte = crypto::randomBytes(32)   ' Windows: raises ErrUnknown
  io::print(toString(len(r)))                        ' macOS/Linux: 32
END SUB
```

## Mechanism

`lower_crypto_random_bytes_helper` (src/codegen/builtins/crypto/native/random.rs)
calls `BCryptGenRandom(NULL, buf+off, chunk, BCRYPT_USE_SYSTEM_PREFERRED_RNG)`
via `platform.emit_external_call` and then treats a non-zero NTSTATUS as failure
(`branch_ne(entropy_fail)` → `ErrUnknown`). On Windows the call returns non-zero
(or the status check reads the wrong register), so it always takes the fail path.
This code is functionally identical to `main` (this branch only renamed the trait
method `emit_libc_call` → `emit_external_call`), so the defect is pre-existing and
was simply never runtime-verified on Windows.

## Hypotheses ruled out

- **Missing Win64 shadow space** around the `BCryptGenRandom` call (the 32-byte
  caller-reserved home area). Adding `subtract_stack(0x20)`/`add_stack(0x20)` did
  NOT fix it.
- **`hAlgorithm` in the wrong register.** The intent is `hAlgorithm = NULL` in arg0
  (`rcx` = `c_arg(0)`); the code writes `return_register()`. On SysV/AArch64 those
  coincide, on Win64 they may not — but forcing `c_arg(0) = 0` did NOT fix it, so
  either `return_register()` already resolves to the arg0 register on Win64 (as the
  working CNG `generateP256` path implies) or the failure is elsewhere.

## Likely remaining suspect

The Win64 result-register convention after `emit_external_call`
(`win_x86_64::code::emit_external_call` → `emit_linux_c_call("windows-x86_64")`,
which — unlike the linux-x86_64 path — does NOT stage `rax` into the aligned MFB
result bank). If `return_register()` does not alias `rax` here, the `!= 0`
NTSTATUS check reads a stale/garbage register and spuriously fails. Next step: an
`.ncode`/objdump trace of the Windows `_mfb_rt_crypto_randomBytes_windows-x86_64`
helper around the `BCryptGenRandom` call to see which register the compare reads.

This is one of a set of Windows-crypto runtime defects surfaced while implementing
`crypto::generate` (see also bug-446, CNG ECDSA verify); the common thread is that
the Windows crypto backend has no runtime golden coverage.
