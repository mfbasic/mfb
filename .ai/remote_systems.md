# Remote Test Machines

- ssh -p 2222 test@127.0.0.1 # ArchLinux (libc)
- ssh -p 2223 test@127.0.0.1 # Kali (libc)
- ssh -p 2224 test@127.0.0.1 # Alipine (musl)
- ssh -p 2225 test@127.0.0.1 # Alipine gtk (musl)
- ssh -p 2226 test@127.0.0.1 # Debian 12 gtk (libc)
- ssh -p 2227 test@127.0.0.1 # Alipine x86_64 (musl)
- ssh -p 2228 test@127.0.0.1 # Ubuntu x86_64 gtk (libc)
- ssh -p 2229 test@127.0.0.1 # Alipine riscv64 (musl)
- ssh -p 2230 test@127.0.0.1 # Win11 x86_64
- ssh -p 2231 test@127.0.0.1 # Android aarch64
- ssh -p 2232 test@127.0.0.1 # Debian riscv64 (libc)

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

