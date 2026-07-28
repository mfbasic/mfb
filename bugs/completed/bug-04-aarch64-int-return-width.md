# MFBASIC bug-04: aarch64 int-return width — `io::flush()` swallows stream failures on aarch64+glibc

Last updated: 2026-07-06
Effort: medium (1h–2h)

`io::flush()` (and the input-prompt flush) silently reports success when the
underlying `fsync(fd)` actually failed, on **aarch64 + glibc** targets. The
runtime checks the C `int` return of `fsync` with a **64-bit** `cmp x0, #0`, but
AAPCS64 leaves the upper 32 bits of `x0` **unspecified** for a sub-64-bit return.
glibc/aarch64 leaves them clear, so `fsync`'s `-1` reads as `+0x00000000FFFFFFFF`
(a large positive), the `b.lt` error branch is not taken, and flush returns OK.
The single correct behavior a fix produces: a genuinely failed `fsync` (e.g.
stdout closed) is detected and surfaced as `ErrOutput` (`77020002`) on **every**
target — aarch64/x86_64 × glibc/musl × macOS — byte-for-byte identical
observable behavior to what x86_64 and macOS already produce.

It complements:

- `./mfb spec io flush` (the flush error contract; canonical spec under
  `src/docs/spec/io/**`).

## 1. Goal

- On aarch64+glibc, `io::flush()` with a broken stdout (closed fd / write error
  surfaced only by `fsync`) **traps** with `err.code == 77020002`, exactly as it
  already does on x86_64 (glibc + musl), aarch64+musl, and macOS.
- The same fix covers every other place a C function returning `int` is checked
  for `< 0` on aarch64 through the shared codegen (audit in Phase 1).
- The two integration tests that currently fail on aarch64+glibc
  (`native_io_runtime.rs`) pass on all boxes in `.ai/remote_systems.md`.

### Non-goals (explicit constraints)

- **No language-surface change.** No new builtins, no change to `io::flush`'s
  signature, error code, or the `TRAP` semantics.
- **No layout/ABI change**, no change to value/copy/move/freeze semantics.
- **Do NOT "fix" this by rewriting the tests to pre-buffer data.** That masks the
  bug (the drain-write path normalizes its own return and hides the `x0` hazard).
  The empty-buffer flush test is a *correct* reproduction and must keep asserting
  that a failed flush traps.
- The `fsync`-benign errno allowlist (`EINVAL 22`, `ENOTSUP 45`,
  `EOPNOTSUPP 95`) stays as-is; this bug is upstream of the errno check.

## 2. Current State

`lower_io_flush_helper` (`src/target/shared/code/io_helpers.rs:330`) builds one
abstract instruction stream (authored against the aarch64 abi, aliased for the
shared code at `src/target/shared/code/mod.rs:4` —
`use crate::arch::aarch64::{abi, ops::CodeOp}`; the x86_64 backend re-lowers the
same abstract ops). For non-stderr flush it:

1. `bl _mfb_rt_io_stdout_drain`, then `cmp x0,#0 / b.ne output_error`
   (`io_helpers.rs:359-365`). The drain is an **mfb-authored** helper returning a
   normalized full-width 0/nonzero, so this check is not affected.
2. `mov x0,#1` (fd = stdout) then `platform.emit_sync_file(...)` — a **raw C**
   `fsync` call (aarch64: `src/target/linux_aarch64/code.rs:409` →
   `emit_linux_c_call(..,"fsync",..)` at `:691`; x86_64:
   `src/target/linux_x86_64/code.rs:545`; macOS `_fsync`:
   `src/target/macos_aarch64/code.rs:503`).
3. The return check (`io_helpers.rs:378-380`):
   `abi::compare_immediate(abi::return_register(), "0")` then
   `abi::branch_lt(&sync_error)`.

`abi::return_register()` is `"x0"` (`src/arch/aarch64/abi.rs:3,50`).
`abi::compare_immediate` emits `cmp_imm` (`src/arch/aarch64/abi.rs:362`), which
the aarch64 encoder lowers as a **64-bit** compare — `emit_cmp_imm` emits base
`0xf100_001f` (`src/arch/aarch64/encode/emitter.rs:829`); the `0xf1` prefix is the
`sf=1` (64-bit) form of `SUBS Xn,#imm` / `CMP`. There is **no `sxtw`** narrowing
the `int` return after `emit_linux_c_call`.

Why x86_64 and aarch64+musl are immune (observed, see reproduction): x86_64
re-lowers the check 32-bit-safely, and musl/aarch64 happens to leave `x0`
sign-extended — both accidents of the caller, not guarantees. glibc/aarch64
zero-extends, exposing the bug.

## 3. Design Overview

> **Implemented (deviation from the sketch below):** rather than insert the
> narrowing in the *shared* flush helper (which would shift x86_64 and macOS
> native goldens for no behavioral benefit — both are already correct), the
> `sxtw` is emitted inside the **aarch64-only `emit_sync_file` seam**
> (`src/target/linux_aarch64/code.rs`), the one place that knows its callee
> (`fsync`) returns `int`. This confines the codegen change to the Linux-aarch64
> target (glibc **and** musl share it; the narrow is a harmless no-op where the
> libc already sign-extends) and leaves **all host/x86 goldens byte-identical**
> (artifact-gate: 1040 tests, 1400 goldens, 0 diffs). macOS was left untouched —
> it works today; narrowing it is a principled follow-up but would churn the
> acceptance oracle for an unproven case.

The abstract flush stream must **narrow the `int` return to 32 bits (sign-extend
`w0`→`x0`) before the `< 0` compare**, so the comparison is correct regardless of
what the callee left in the upper 32 bits. The fix adds a 1:1 abstract op
`abi::sign_extend_word(reg)` (aarch64 `SXTW`, x86 `movsxd`; `mirror` group in
`mir.rs`) that each backend encodes:

- aarch64: `SXTW x0, w0` (`0x9340_7c00 | (rn<<5) | rd`, sf/N set — verify against
  the encoder’s existing `sbfm`/extend helpers).
- x86_64: `movsxd rax, eax` (or `CDQE`) in the x86 re-lowering of the op.

Insert one `abi::sign_extend_word(abi::return_register())` between
`emit_sync_file` and the `compare_immediate` at `io_helpers.rs:377-378`.

Correctness risk concentrates in: (a) getting the aarch64 `SXTW` encoding right,
(b) the x86_64 re-lowering of the new op, and (c) the golden shift — adding one
instruction changes native code for the flush helper on **all** targets, so
`.ncode`/native goldens for programs that emit the flush helper will move.

Alternative (Open Decision §): instead of a shared `sign_extend_word` op, make the
raw-C-`int` seam (`emit_linux_c_call` / `emit_libc_call` / `emit_libsystem_call`)
sign-extend `int`-returning calls at the call site. Rejected as default: those
seams also carry pointer/`ssize_t`/`off_t` returns that must **not** be narrowed
(e.g. `lseek`/`read`/`write`), so narrowing must be opt-in at the check, not
blanket at the call.

## Layout / ABI Impact

None to `mfb spec memory` / package layout. Native **code** output shifts for any
program that emits `_mfb_rt_io_flush` (and any other call sites touched in
Phase 1): one extra `sxtw`/`movsxd`. No data/record/closure layout changes, so
value/copy/transfer output is unaffected. Golden regeneration is limited to
native-code goldens (`.ncode` and downstream `.nplan`/build artifacts), not
`.ir`/`.ast`/`.mfp`.

## Phases

### Phase 1 — audit + failing test (no codegen change)

Establish the reproduction and the blast radius before touching codegen.

- [ ] Add/confirm the failing repro. `tests/native_io_runtime.rs:420`
  (`native_io_flush_reports_standard_stream_failures`) and `:757`
  (`native_io_input_reports_prompt_flush_failure`) already reproduce on
  aarch64+glibc; keep them. Record the exact manual repro (below) in the test
  comment so the width dependency is documented.
- [ ] Audit every aarch64 check of a **raw C `int`** return compared with
  `branch_lt`/`< 0` through the shared codegen: grep `emit_linux_c_call`,
  `emit_libc_call`, `emit_libsystem_call` call sites and their following
  `compare_immediate(return_register(), "0")` / `branch_lt`. Classify each callee
  by return type: `int` (needs narrowing) vs `ssize_t`/`off_t`/pointer (must NOT
  be narrowed). Produce the list in this file.
- [ ] Tests: none new required beyond the two existing reproductions, unless the
  audit finds a second `int`-return site with no coverage — add one there.

Acceptance: the two `native_io_*flush*` tests fail on Kali (`ssh -p 2223`,
aarch64+glibc) and pass on Ubuntu x86_64 (`ssh -p 2228`); the audited list of
`int`-return check sites is written into §Current State with each site’s verdict.
Commit: — (repro confirmed on 2222/2223; sibling audit below)

**Sibling `int`-return audit (io_helpers.rs):** other raw-C-`int` calls checked
with `branch_lt` on aarch64 share the *latent* hazard but were **not** observed to
fail and are **out of scope** for this bug (they are behind a real
close/poll/termios failure, not exercised by the empty-flush path): `poll`
(`:552`), `tcgetattr`/`tcsetattr` (terminal raw-mode, `:650/:719/:763`). NOTE:
the `read`/`write` checks (`:120/:149/:264/:286/:856/…`) return `ssize_t`
(64-bit) and must **not** be narrowed. `emit_seek_file`→`lseek` returns
`off_t`/64-bit — correctly left unnarrowed. If any of the termios/poll paths is
later shown to swallow a failure on aarch64+glibc, apply the same
`abi::sign_extend_word` after that seam.

### Phase 2 — narrow the flush return check (the fix)

- [ ] Add `abi::sign_extend_word(reg)` (`src/arch/aarch64/abi.rs`) emitting a new
  `sxtw` abstract op; encode it in `src/arch/aarch64/encode/emitter.rs` and add
  the x86_64 re-lowering (`movsxd`/`cdqe`).
- [ ] Insert `abi::sign_extend_word(abi::return_register())` between
  `emit_sync_file` and the compare at `src/target/shared/code/io_helpers.rs:377`.
- [ ] Apply the same narrowing to any other `int`-return site Phase 1 flagged.
- [ ] Encoder unit test: `sxtw x0,w0` encodes to the expected word
  (`src/arch/aarch64/encode/tests.rs`), and the x86_64 encoder likewise.

Acceptance: rebuild; the empty-flush repro (below) traps with `77020002` on Kali
**and** Arch aarch64 (`ssh -p 2222`); still traps on x86_64 (2228/2227) and
aarch64+musl (2224); macOS unchanged. Both `native_io_*flush*` tests pass on every
box in `.ai/remote_systems.md`.
Commit: **c9e8c706** — DONE. Verified fd-closed flush traps `77020002` on 2222
Arch + 2223 Kali (both were failing) and still traps on 2224 (aarch64-musl) +
2227 (x86_64-musl). Sibling termios/poll `int`-return sites left as documented
latent-only (Phase 1 audit); no other site fixed.

### Phase 3 — golden regeneration + full acceptance (highest-risk last)

- [ ] Regenerate native-code goldens for programs emitting the flush helper
  (`scripts/artifact-gate.sh` for the execution-free codegen gate, then the
  affected `tests/**/golden/*.ncode`). Confirm the *only* delta is the added
  `sxtw`/`movsxd` in the flush helper (and any Phase-1 sites), nothing else.
- [ ] Full acceptance on macOS: `scripts/test-accept.sh target/debug/mfb
  target/accept-actual` (byte-identical except the intended native-code shift).
- [ ] Cross-validate on Linux boxes per `.ai/remote_systems.md`
  (`mfb build -target linux-aarch64` → scp → run) that the reproduction is fixed
  and nothing else regressed.

Acceptance: acceptance suite green; goldens reflect only the intended one-op
shift; the manual reproduction passes on all listed boxes.
Commit: **c9e8c706** — DONE (subsumed into Phase 2's commit). No golden
regeneration was needed: the aarch64-seam fix leaves all *host* (macOS) goldens
byte-identical — artifact-gate reported 1040 tests / 1400 goldens / **0 diffs**,
and `test-accept.sh` passed 1040 tests. The intended native-code shift is
linux-aarch64-only, which the macOS acceptance oracle does not build.

## Validation Plan

- Manual reproduction (the proof):
  ```
  # program: FUNC main AS Integer / io::flush() / RETURN 17 / TRAP(err) io::printError(toString(err.code)) RETURN 0 END TRAP / END FUNC
  mfb build -target linux-aarch64 <proj>     # glibc .out
  ./fdtest-glibc.out 1>&- 2>/tmp/e           # close stdout
  # BEFORE: exit 17, empty stderr   (bug)
  # AFTER:  exit 0,  stderr "77020002"  (trap fired)
  ```
  Ground truth that the kernel reports the failure (so the bug is ours, not the
  OS): C probe on Kali showed `fsync(closed 1) => r=-1 errno=9 (EBADF)` while the
  unfixed mfb binary returned 17.
- Function/integration tests: `tests/native_io_runtime.rs`
  `native_io_flush_reports_standard_stream_failures`,
  `native_io_input_reports_prompt_flush_failure`.
- Doc sync: none expected (behavior is being brought *to* the documented
  contract; note in `src/docs/spec/io/flush` only if the current text implies the
  old aarch64 behavior).
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Observed matrix (why the fix is target-shaped)

| Box (`.ai/remote_systems.md`) | arch / libc | kernel | empty flush, fd1 closed |
| --- | --- | --- | --- |
| macOS (dev) | aarch64 | — | traps ✓ |
| 2228 Ubuntu | x86_64 glibc | 6.x | traps ✓ |
| 2227 Alpine | x86_64 musl | — | traps ✓ (`fsync`→EBADF probed) |
| 2224 Alpine | aarch64 musl | — | traps ✓ (accidental sign-extend) |
| 2222 Arch | aarch64 **glibc** | 5.18 | **exit 17** ✗ |
| 2223 Kali | aarch64 **glibc** | 6.3 | **exit 17** ✗ (`fsync`→EBADF probed) |

Kali is decisive: the kernel returns `EBADF`, yet mfb misses it — so this is an
mfb aarch64 codegen bug, not a kernel/`fsync` portability quirk, and not a flaky
test. CI failing implies the CI runner for the coverage job is aarch64+glibc.

## Open Decisions

- Narrowing location — new shared `abi::sign_extend_word` op inserted at the
  check (recommended; surgical, backend-encoded once) vs. sign-extending inside
  the `emit_*_c_call` seams (rejected: those seams also return
  pointer/`ssize_t`/`off_t` that must not be narrowed). (§3)
- Whether to also add a general aarch64 lint/debug-assert that a
  `compare_immediate(return_register, 0)` following a raw C `int` call is preceded
  by a narrow — nice-to-have, defer. (§Phase 1)

## Non-Goals

- Broad sweep of every 64-bit compare in the tree — scope is raw C `int` returns
  checked for sign on aarch64, surfaced by the Phase 1 audit only.
- Changing `io::flush`/drain buffering behavior or the benign-errno allowlist.

## Summary

Real engineering risk is the aarch64 `SXTW` encoding + x86_64 re-lowering of the
new op and the resulting native-golden shift; everything else (drain path, errno
allowlist, layout, semantics) stays untouched. The fix is one narrowing op before
one compare, plus whatever sibling `int`-return sites the audit turns up.
