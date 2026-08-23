# bug-450: crypto::generate for a NIST curve segfaults on linux-x86_64

Last updated: 2026-08-22
Effort: unknown (native OpenSSL EC-keygen interop on x86_64 SysV)
Severity: HIGH (crash on a valid, supported platform; uncatchable — a TRAP cannot recover)
Class: Correctness (valid program crashes)

Status: Open
Regression Test: to be added (rt-behavior crypto generate on linux-x86_64)

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
