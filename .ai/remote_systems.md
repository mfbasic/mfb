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

**Four do, and no single probe finds all four.** Measured 2026-09-03 (with 2223
re-probed 2026-09-10); the claim in
`scripts/linux-runtime-proof.sh`'s header that *"none of the Linux boxes carries a Rust
toolchain (2229 is the lone exception)"* is false and has been corrected there too.

| box | cargo | where | cores |
|---|---|---|---|
| **2227** Alpine x86_64 musl | 1.96.1 | `/usr/bin/cargo` (distro package) | **4** |
| **2228** Ubuntu x86_64 gtk glibc | 1.96.0 | `~/.cargo/bin/cargo` — **not on the non-interactive PATH** | 1 |
| **2229** Alpine riscv64 musl | 1.96.0 | `/usr/bin/cargo` | 8 |
| **2223** Kali aarch64 glibc | verified 2026-09-10 | `/usr/bin/cargo` | probe before use |

The trap is in the middle row. `ssh -p 2228 'command -v cargo'` answers **nothing**,
because a non-login shell does not source the rustup env — yet 2228 is the box this
project has built on for months. Conversely `ls ~/.cargo/bin/cargo` answers nothing on
2227 and 2229, where it is a distro package. **Probe both:**

```
ssh -p PORT test@127.0.0.1 "ls ~/.cargo/bin/cargo 2>/dev/null; command -v cargo"
```

and invoke it by the path you found, not by name.

**Prefer 2223 for a Linux `cargo` row.** Its aarch64 release build completed a clean
archived tree and native `os::prog` proof on 2026-09-10. Use 2227 when its x86_64 musl
coverage is specifically required; its final release link can be substantially slower.
Caveats worth knowing before moving a row to 2227:

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

**2227 and 2228 are QEMU-TCG-emulated x86_64 VMs on the ARM Mac** (check with
`ps -axo command | grep QEMULauncher`). That is why 2223 is preferred: a release
`cargo build` took 145 minutes on 2227, and the emulation takes CPU away from local jobs
running at the same time. Use them only when x86_64 itself is the point, and keep those
runs short.

**No Linux box has `strace` or passwordless `sudo`.** On apt box 2223, install it without root:
`apt-get download strace && dpkg -x strace_*.deb root && ./root/usr/bin/strace -f -i ./prog-glibc.out`.
That build has no `-k`; use `-i` instead (PIE executable IPs are `0xaaaa…`, libc's `0xffff…`).
2223 has no musl loader, so run the `-glibc.out` build there.

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
Linux-host only, so it cannot run on the Mac). `tools/math-kernels/rvv-qemu-runner.sh` ships a
build to 2232 and runs it under `qemu-riscv64 -cpu rv64,v=true,vlen=128` / `v=false`;
`tools/math-kernels/rvv-ulp-two-profile.sh` drives the ULP harness across both profiles.

## Vulkan: every reachable driver is a CPU one

Probed 2026-09-24 (bug-688). **No reachable box has a Vulkan GPU**, so a Vulkan frame
rate measured here is a CPU rasteriser's, not a GPU's; correctness (the software-oracle
comparison) is meaningful, performance is not.

| box | GPU as the VM sees it | Vulkan driver | display |
|---|---|---|---|
| 2226 Debian 12 aarch64 glibc | virtio GPU | Mesa lavapipe, system ICD (`lvp_icd.aarch64.json`) | a live GNOME **Wayland** session (`/run/user/1001/wayland-0`) |
| 2230 Win11 x86_64 (emulated) | Red Hat VirtIO GPU DOD | Mesa lavapipe under `C:\mfbvk\mesa\x64`, registered in `HKLM\SOFTWARE\Khronos\Vulkan\Drivers` | none used (headless) |

* 2226 runs aarch64 natively, so it is the fast box for Vulkan rows
  (`scripts/test-canvas-gpu-rows.sh --target linux-aarch64`). It also compiles SPIR-V:
  `MFB_SPIRV_PORT=2226 scripts/regen-spirv.sh` — but its glslang (Debian 11:12.0.0) is not
  2228's, so an unchanged `.vert` comes back with different bytes. Restore a blob whose GLSL
  did not change rather than committing the churn.
* 2230 is x86-64 under emulation: a software-oracle render of a large polygon scene takes
  tens of minutes there. Order the geometry rows first when time matters.
* Probe: `ls /usr/share/vulkan/icd.d` (Linux); on Windows,
  `Get-ItemProperty HKLM:\SOFTWARE\Khronos\Vulkan\Drivers` from PowerShell.

## Box 2230 (Windows): ssh quirks and crash diagnosis without a debugger

`cmd.exe` over ssh produces several things that look like a broken box but aren't:

- `ssh -p 2230 test@127.0.0.1 true` fails because `cmd.exe` has no `true`. Probe with `ver`.
- `The system cannot find the path specified.` prints on almost every command. It is session noise.
- `set X=1 && prog` gives `X` a trailing space, and quoting through ssh is unreliable. Write a
  **CRLF** `.bat`, `scp` it, and run that.
- `timeout /t` fails ("Input redirection is not supported"); use `ping -n <sec+1> 127.0.0.1 >nul`.
- No C compiler and no `openssl`. Probe Win32 struct layouts with a PowerShell `Add-Type` C#
  P/Invoke snippet; for a TLS client, use PowerShell `SslStream`.
- An access violation exits with `-1073741819` (`0xC0000005`).

To find a fault without cdb/windbg:
1. `Get-WinEvent -FilterHashtable @{LogName='Application'; ProviderName='Application Error'} -MaxEvents 1`
   gives the faulting module and offset.
2. Symbolize by walking the module's PE export table in PowerShell, and report the nearest
   exports **below and above** the offset. Most MFB functions are private, so the nearest one
   below can be far off.
3. For a stack, enable `HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps`
   (`DumpFolder`, `DumpCount`, `DumpType=1`) and parse the minidump: streams 3 (threads),
   4 (modules), 6 (exception). In an AMD64 `CONTEXT`, `Rip` is at `0xF8` and `Rsp` at `0x98`.
