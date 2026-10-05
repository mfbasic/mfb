# bug-957: riscv64 thread trampoline clobbers callee-saved `s0` — every `thread::start` segfaults on glibc

Last updated: 2026-10-05
Effort: small (<1h)
Severity: HIGH
Class: Correctness

Status: Open
Coordinator re-run (HEAD 846aa55fd, podman `--platform linux/riscv64`, qemu-user): the musl
build prints `thread=42` (rc 0). A thread-free hello-world glibc build runs fine. The thread
repro's glibc build never prints anything: the process sits in state `S` for more than 5
minutes until killed. The failure reproduces (glibc fails, musl passes), but here it shows
as a hang where the agent saw SIGSEGV. The qemu-user signal delivery for a faulting worker
thread differs between setups. Real riscv64 hardware was not available to either run.
Regression Test: tests/rt-behavior/thread/<new riscv64-glibc thread fixture> (must run on a glibc riscv64 box, e.g. 2232)

On `linux-riscv64` **glibc** builds, any program that calls `thread::start` dies with
SIGSEGV as soon as the worker finishes. The musl flavor of the same build, and the
aarch64/x86-64 glibc flavors, run correctly. The hand-written thread trampoline
`_mfb_rt_thread_trampoline` (a pthread start routine, i.e. a C-ABI callee) uses the
neutral machine-floor scratch `abi::SCRATCH[4]`, which the riscv64 backend realizes as
**`s0`** — a callee-saved register under lp64d — and never saves/restores it. glibc's
`start_thread` keeps its `struct pthread *pd` in `s0` across `pd->start_routine(pd->arg)`
and dereferences it immediately after the call returns, so it stores the worker result
through a garbage pointer. musl's start path happens not to need `s0` after the call,
which is why the bug is invisible on the only reachable riscv64 test box (2229, Alpine/musl).

Correct behavior: the trampoline preserves every lp64d callee-saved register it writes
(or uses only caller-saved scratch), so `thread::start`/`waitFor` behaves identically on
riscv64 glibc and musl.

References:

- `mfb spec linker linux-riscv64` ("Calling convention and ABI": callee-save `s0`–`s11`;
  "Both worlds share every kernel struct layout ... so only the import library names and the
  interpreter differ")
- `mfb spec memory native-calling-convention` (riscv64 callee-saved set)
- Found during B15 linker hardening (running every target's output in containers).

## Failing Reproduction

```
mkdir -p /tmp/b15thr && cd /tmp/b15thr && mfb init thr
cat > thr/src/main.mfb <<'EOF'
IMPORT io
IMPORT thread
ISOLATED FUNC spin(t AS ThreadWorker OF String TO Integer, n AS Integer) AS Integer
  RETURN n * 2
END FUNC
FUNC main() AS Integer
  RES t AS Thread OF String TO Integer = thread::start(spin, 21)
  io::print("thread=" & toString(thread::waitFor(t)))
  RETURN 0
END FUNC
EOF
/tmp/hardening/mfb-head build -q --target linux-riscv64 thr
# glibc (Debian trixie riscv64, via podman/qemu-user binfmt):
podman run --rm --platform linux/riscv64 -v ... debian:trixie-slim ./thr/build/thr-glibc.out
# musl (Alpine riscv64):
podman run --rm --platform linux/riscv64 -v ... alpine:3 ./thr/build/thr-musl.out
```

- Observed (glibc): `Segmentation fault (core dumped)`, exit 139, nothing printed.
- Observed (musl): `thread=42`, exit 0.
- Expected: `thread=42`, exit 0 on both.

Same SIGSEGV reproduced independently with `qemu-riscv64 7.2 -L <trixie riscv64 rootfs>`.
gdb-multiarch on the qemu gdbstub stops in the worker thread inside glibc `start_thread`,
right after the start routine returns:

```
   0x40028a0dfc:	jalr	a4               ; pd->start_routine(pd->arg)  (our trampoline)
   0x40028a0dfe:	ld	a5,-392(s0)        ; reload pd via s0 -- s0 now garbage
=> 0x40028a0e02:	sd	a0,1064(a5)        ; SIGSEGV, si_addr=0xe989899300005dbf
```

Trampoline disassembly (`--debug` build, symbol `_mfb_rt_thread_trampoline` @0x7340):
prologue saves only `ra`, `s11`, `s2`, `s10`; then `ld s0,152(s2)` / `ld s10,8(s0)` /
`ld s0,0(s0)` / `jalr s0`; epilogue restores `s11`, `s10`, `s2`, `ra` — **`s0` is never
saved or restored**.

| Environment | flavor | Result |
| --- | --- | --- |
| podman linux/riscv64 debian:trixie-slim (glibc 2.41) | glibc | fails ✗ (SIGSEGV) |
| qemu-riscv64 7.2 + trixie rootfs | glibc | fails ✗ (SIGSEGV) |
| podman linux/riscv64 alpine:3 | musl | works ✓ |
| podman linux/arm64 debian trixie / alpine | glibc / musl (aarch64) | works ✓ |
| qemu-x86_64 -cpu max, bookworm | glibc (x86-64) | works ✓ |

Contrast: `process::detach` (whose reaper `_mfb_rt_process_reaper` is also a pthread start
routine) works on riscv64 glibc — its prologue saves `s1`, the only callee-saved register it uses.

## Root Cause

`src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_trampoline` loads the worker
entry into `abi::SCRATCH[4]` (`load_u64(abi::SCRATCH[4], CURRENT_THREAD, CONTROL_OFFSET_ENTRY)`
… `branch_link_register(abi::SCRATCH[4])`) and its frame saves only LR, the arena register,
`CURRENT_THREAD` and `CLOSURE_ENV`. The comment there assumes SCRATCH[4]/[5] are caller-saved
(`x13`/`x14` on AArch64, `r9`/`r10` on x86-64). On riscv64,
`src/arch/riscv64/select.rs:map_scratch_register` maps `x13 → s0` and `x14 → s3` — callee-saved
"substitutes" justified by "over-preserving is safe", which is only true for callers, not for a
routine that is itself called by C code and returns to it.

## Goal

- `thr-glibc.out` above prints `thread=42` and exits 0 on riscv64 glibc.
- The trampoline (and any other hand-written C-ABI callee on riscv64) leaves `s0`–`s11` as found.

### Non-goals (must NOT change)

- aarch64/x86-64 trampoline bytes and frame size (the bug-408 88-/80-byte frames).
- Do not "fix" by switching the riscv64 thread path to musl-only or skipping glibc tests.

## Blast Radius

- `lower_thread_trampoline` — fixed by this bug (save/restore `s0`, or pick a scratch that is
  caller-saved on riscv64, e.g. map via a C-callee-safe scratch).
- `_mfb_rt_process_reaper` — unaffected (saves `s1`, measured by disassembly).
- `_mfb_rt_signal_handler` — uses `t3` only, never returns normally (`j .` after shutdown).
- Any other machine-floor helper using SCRATCH[4]/[5] that is entered *from C* — latent,
  needs an audit (`grep -rn "SCRATCH\[4\]\|SCRATCH\[5\]" src/codegen`).
- Test gap: riscv64 runtime goldens run only on 2229 (musl); 2232 (Debian riscv64 glibc) was
  unreachable during this review, so no glibc riscv64 thread run exists.
