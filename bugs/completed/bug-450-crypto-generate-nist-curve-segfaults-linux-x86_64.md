# bug-450: crypto::generate for a NIST curve segfaults on linux-x86_64

Last updated: 2026-08-23
Effort: small (one-line-per-site register-bank fix; no ABI redesign)
Severity: HIGH (crash on a valid, supported platform; uncatchable — a TRAP cannot recover)
Class: Correctness (valid program crashes)

Status: FIXED
Regression Test: tests/codegen_crypto_ec_c_return_x86_64.rs (codegen-inspection: no
external-call result is read from the MFB-return bank on linux-x86_64) + the existing
rt-behavior crypto-ec-valid fixture (box-run on 2227 musl / 2228 glibc).

`crypto::generate(Certificate.P256)` (and the other NIST curves P384/P521, which
share the OpenSSL path) SIGSEGVs at runtime on **linux-x86_64 (musl)**. The crash
is uncatchable: wrapping the call in an inline `TRAP` does not help — the process
dies with a segfault, not a raised error. macOS-aarch64 and **windows-x86_64**
both run the identical call correctly (the full acceptance suite is 608/608 on
each), so this is specific to the linux-x86_64 OpenSSL EC-keygen path, not shared
x86_64 codegen.

The NIST-curve key generation dlopens OpenSSL `libcrypto` and calls its EC/EVP
keygen (`src/codegen/builtins/crypto/func_sign.rs` — `dlopen_libcrypto`,
`EVP_*`). The in-tree Ed25519 generator (`helper_generate_ed25519.rs`, no
libcrypto) works on the same box, and `crypto::randomBytes`/`crypto::randomInt`
(getrandom, no libcrypto) work too — so entropy and the general crypto surface
are fine; only the dlopen'd OpenSSL EC path crashes.

The box is **not** missing the library: `/usr/lib/libcrypto.so.3` is present
(Alpine `openssl`/`libcrypto3` installed). So this is not a graceful
"library-unavailable" path (which should raise `ErrNativeBindingUnavailable`
77030007) — libcrypto loads, and the subsequent call sequence faults. Likely
causes to investigate: the OpenSSL 3.x API/handle usage, a dlsym'd-symbol call
ABI mismatch under x86_64 SysV, or a musl-vs-glibc difference (only the musl box,
2227, was reachable; the glibc x86_64 box was down — re-test on glibc to split
musl-specific vs linux-x86_64-general).

## Failing Reproduction

```mfbasic
IMPORT io
IMPORT crypto
SUB main()
  LET rb AS List OF Byte = crypto::randomBytes(16)         ' ok
  io::print("randomBytes ok len=" & toString(len(rb)))
  LET ed AS crypto::KeyPair = crypto::generate(Certificate.Ed25519)  ' ok (in-tree)
  io::print("Ed25519 ok len=" & toString(len(ed.publicKey)))
  LET p AS crypto::KeyPair = crypto::generate(Certificate.P256) TRAP(e)
    io::print("P256 trapped " & toString(e.code))          ' never reached
    EXIT SUB
  END TRAP
  io::print("P256 ok")
END SUB
```

- Build: `mfb build --target linux-x86_64 <proj>` → `<proj>/build/<name>-musl.out`.
- Run on an Alpine musl x86_64 host with libcrypto installed:
  prints the randomBytes + Ed25519 lines, then **Segmentation fault (exit 139)**
  at the P256 generate. Expected: prints `P256 ok` (a 65-byte public key).

| Environment | arch | libc | Result |
| --- | --- | --- | --- |
| macOS | aarch64 | — | ok ✓ |
| Windows | x86_64 | — | ok ✓ (CNG path, not OpenSSL) |
| Linux (Alpine) | x86_64 | musl | segfault ✗ |
| Linux | x86_64 | glibc | not yet tested (box down) |

Found while cross-platform-running the acceptance suite (the suite crashes at the
crypto "generate — key pairs" group on linux-x86_64; every earlier group passes).

## Root cause (confirmed by core dump on 2227)

Not a "library unavailable", not a stack misalignment, and not an OpenSSL API
misuse. The core dump faults at `EC_KEY_generate_key+0x9` (`cmpq $0, 0x18(%rdi)`)
with `rdi = 0x17` — a garbage "EC_KEY pointer" that is actually a stale *argument*
left in `rdi`, and `RSP % 16 == 0` (the stack was correctly aligned).

The NIST-EC bodies (`crypto::generate`/`sign`/`verify`, `func_generate.rs` /
`func_sign.rs` / `func_verify.rs`) call the dlopen'd libcrypto functions through a
raw indirect `blr` (`gen_cert::call_fn` / an inline `branch_link_register`), then
read each call's result from `abi::return_register()` — the aligned **MFB**-return
bank. On **x86-64 SysV** that bank is `rdi`, but a C function returns in `rax`
(the **C**-return bank); the direct-call path (`emit_linux_c_call`, used for
`dlopen`/`dlsym`) already stages `mov rdi, rax`, but the indirect libcrypto calls
did not. So every "result" was the stale first argument: e.g.
`EC_KEY_new_by_curve_name(nid=415)` returned an `EC_KEY*` in `rax`, but the code
stored `rdi` (still `415`) as the key, and then a later value flowed in as
`rdi = 0x17` when `EC_KEY_generate_key` was called — dereferenced → SIGSEGV. On
AArch64/RISC-V the argument and result banks coincide (`x0`), which is exactly why
linux-aarch64, macOS-aarch64, and Windows (CNG, no `blr`-return-read) all ran the
identical code correctly and only x86-64 crashed.

## Fix

Read every external-call result from `abi::c_return(0)` instead of
`abi::return_register()` in the three Linux OpenSSL emitters (`emit_linux_ec`,
`emit_linux_sign`, `emit_linux_verify`). `c_return(0)` realizes to `rax` on x86-64
(correct) and to `x0` on AArch64/RISC-V — byte-identical to the old code there, so
only the linux-x86_64 codegen changes.

Verified on box 2227 (Alpine musl x86_64) and 2228 (Ubuntu glibc x86_64):
generate+sign+verify round-trips for P256/P384/P521 (and Ed25519) all return
`verify=TRUE`; the reproduction prints `P256 ok len=65` (was SIGSEGV, exit 139).

| Environment | arch | libc | Result |
| --- | --- | --- | --- |
| macOS | aarch64 | — | ok ✓ (unchanged) |
| Windows | x86_64 | — | ok ✓ (CNG path, not OpenSSL) |
| Linux (Alpine) | x86_64 | musl | ok ✓ (was segfault) |
| Linux (Ubuntu) | x86_64 | glibc | ok ✓ (was untested) |
| Linux | aarch64 | — | ok ✓ (byte-identical codegen) |

STATUS: FIXED (e52ae32d2)

Deviations from the plan / notes for the next session:
- The doc's "likely causes" (musl-vs-glibc, dlsym ABI mismatch, stack misalignment)
  were all wrong. The core dump settled it: `RSP % 16 == 0` (aligned) and the fault
  was inside libcrypto with a stale-argument pointer — an arg/result register-bank
  mismatch, reproducing identically on musl (2227) AND glibc (2228).
- The same latent bug lived in `crypto::sign`/`verify` (not just `generate`); all
  three are fixed. The acceptance suite only ever hit `generate` first because a key
  must exist before signing.
- Four `crypto-ec-valid` ncodesum goldens were already stale on `main` (commit
  8c1b93891's crypto ripple regenerated byte-identity/crypto but not this fixture);
  regenerated as part of this fix. Nine unrelated pre-existing artifact-gate diffs
  remain on main (5 windows byte-identity + 4 macos-app-mode), tracked by the
  concurrent plan-102 work — out of scope here.
