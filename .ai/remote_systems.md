# Remote Test Machines

- ssh -p 2222 test@127.0.0.1 # ArchLinux (libc)
- ssh -p 2223 test@127.0.0.1 # Kali (libc)
- ssh -p 2224 test@127.0.0.1 # Alpine (musl)
- ssh -p 2225 test@127.0.0.1 # Alpine gtk (musl)
- ssh -p 2226 test@127.0.0.1 # Debian 12 gtk (libc)
- ssh -p 2227 test@127.0.0.1 # Alpine x86_64 (musl)
- ssh -p 2228 test@127.0.0.1 # Ubuntu x86_64 gtk (libc)
- ssh -p 2229 test@127.0.0.1 # Alpine riscv64 (musl)
- ssh -p 2230 test@127.0.0.1 # Win11 x86_64
- ssh -p 2231 test@127.0.0.1 # Android aarch64
- ssh -p 2232 test@127.0.0.1 # Debian riscv64 (libc)

## Which boxes have a Rust toolchain — probe, do not read

**Three do, and no single probe finds all three.** Measured 2026-09-03; the claim in
`scripts/linux-runtime-proof.sh`'s header that *"none of the Linux boxes carries a Rust
toolchain (2229 is the lone exception)"* is false and has been corrected there too.

| box | cargo | where | cores |
|---|---|---|---|
| **2227** Alpine x86_64 musl | 1.96.1 | `/usr/bin/cargo` (distro package) | **4** |
| **2228** Ubuntu x86_64 gtk glibc | 1.96.0 | `~/.cargo/bin/cargo` — **not on the non-interactive PATH** | 1 |
| **2229** Alpine riscv64 musl | 1.96.0 | `/usr/bin/cargo` | 8 |

The trap is in the middle row. `ssh -p 2228 'command -v cargo'` answers **nothing**,
because a non-login shell does not source the rustup env — yet 2228 is the box this
project has built on for months. Conversely `ls ~/.cargo/bin/cargo` answers nothing on
2227 and 2229, where it is a distro package. **Probe both:**

```
ssh -p PORT test@127.0.0.1 "ls ~/.cargo/bin/cargo 2>/dev/null; command -v cargo"
```

and invoke it by the path you found, not by name.

**Prefer 2227 for a `cargo test` row.** Four cores against 2228's one turns the slowest
gate in a plan series into something an hour shorter. Caveats worth knowing before
moving a row there:

* It is **musl**, so it is a different libc world from 2228's glibc — a row that is
  about glibc behaviour still belongs on 2228.
* It has **no `rsync`**. Ship with `git archive HEAD -o /tmp/tree.tar`, `scp`, `tar -x`,
  which is arguably better for a gate anyway: it ships exactly the committed tree, so
  uncommitted local state cannot leak into a result you are about to cite.
* Put the target dir on tmpfs — `CARGO_TARGET_DIR=/tmp/target` — `/` has little free
  space and `/tmp` has 7.8 G.

**2228's linker crashes on a large link.** `rust-lld` segfaulted twice, identically,
linking the `mfb` test binary (`ld terminated with signal 11`, LLVM stack dump), with
5 GB free and 26 GB disk — so not resource exhaustion. `RUSTFLAGS='-C
link-arg=-fuse-ld=bfd'` links it, at the cost of invalidating the dependency cache.

*None of this stays true by itself.* This section is a snapshot of a probe; the probe is
the part to keep.

App-mode proof surface (plan-56-C §4.2.1) — **re-probe, do not assume**; three of
these facts changed during plan-56 itself:

| box  | arch    | libc  | GTK4 | /dev/fuse | suid fusermount3 | FUSE mount  |
| ---- | ------- | ----- | ---- | --------- | ---------------- | ----------- |
| 2228 | x86_64  | glibc | yes  | yes       | yes              | works       |
| 2227 | x86_64  | musl  | yes  | yes       | yes              | works       |
| 2224 | aarch64 | musl  | yes  | **no**    | yes              | unavailable |
| 2226 | aarch64 | glibc | yes  | —         | —                | often down  |

App mode builds for BOTH libc worlds (plan-56-B), so the Alpine boxes are proof
surface, not out of scope. A FUSE mount needs `/dev/fuse` **and** a suid
`fusermount3` and the two fail independently, so probe both and fall back to
`--appimage-extract-and-run`.

`gcompat` was deliberately REMOVED from both Alpines: it symlinks
`/lib/libc.so.6` to `libgcompat.so.0` and would let a glibc-linked binary run,
masking exactly the bug plan-56-A fixes.

A Linux AppImage **cannot be tested under emulation** — its type-2 magic at ELF
offset 8 is ignored by a real kernel but rejected by qemu-user and Rosetta, so
`scripts/test-appimage.sh` ships the artifact to a real box rather than running
it in a container on the Mac.

2232 (Debian riscv64) has GTK4 and FUSE, but riscv64 app mode is still
impossible: the GTK entry was never ported (bug-117.1) and upstream publishes no
riscv64 AppImage runtime to seal with.

RISC-V Vector (`V`) status (plan-32): **both** riscv64 boxes lack `V` in hardware
— `/proc/cpuinfo` `isa` is `rv64imafdch_...zba_zbb_zbc_zbs_...` with no `v` (2229
Alpine musl, 2232 Debian glibc). A native run therefore only ever exercises the
scalar (`v=false`) path of the one-binary RVV dual-path. To get the `v=true`
(native-RVV) path, run under **qemu-user**, which emulates `V` and sets
`AT_HWCAP` bit 21: it is fetched without root on 2232 via `apt-get download
qemu-user` → `dpkg -x qemu-user_*.deb ~/qemuroot` (→ `~/qemuroot/usr/bin/qemu-riscv64`,
Linux-host only, so it cannot run on the Mac). `scripts/rvv-qemu-runner.sh` ships a
build to 2232 and runs it under `qemu-riscv64 -cpu rv64,v=true,vlen=128` / `v=false`;
`scripts/rvv-ulp-two-profile.sh` drives the ULP harness across both profiles.

