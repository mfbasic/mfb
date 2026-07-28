# bug-44: bug-04's `int`-return normalization was never ported to x86_64/macOS (`fsync`), and was never applied to `close` on any backend — atomic-write durability failures are silently swallowed

Last updated: 2026-07-09
Effort: medium (1h–2h)

The shared filesystem helpers test C `int` results from libc with a **64-bit**
compare (`cmp x0, #0` + `branch_lt`). That is only correct if the callee sign-extends
its `int` return into the full 64-bit register — which **no** relevant ABI guarantees:
AAPCS64 leaves `x0[63:32]` unspecified, the Darwin arm64 ABI likewise, and x86-64
SysV leaves `rax[63:32]` undefined for an `int` return. When libc leaves the upper
bits clear, a `-1` reads as `+4294967295`, `branch_lt` is not taken, and the failure
is reported as success.

bug-04 (fixed, `c9e8c706`) proved this is not theoretical: it was observed live on
`fsync` under linux-aarch64+glibc. The fix was applied **in the backend**, as a
`sign_extend_word` inside `linux_aarch64::emit_sync_file`. Two gaps remain:

1. **`fsync` is still unnormalized on `linux_x86_64` and `macos_aarch64`.** The fix
   was copied to `linux_riscv64` but not to the other two backends. All three
   `fs::writeTextAtomic` / `fs::writeBytesAtomic` sites check `fsync`'s result with
   the 64-bit `branch_lt`, so on those two targets an `fsync` failure (EIO on failing
   storage) makes a *durable atomic write* report success without the data reaching
   disk.
2. **`close` is unnormalized on *all four* backends** while being checked with the
   identical 64-bit `branch_lt` at three sites. `close` returns deferred write errors
   (`EIO`, `ENOSPC`) on NFS/CIFS and on any filesystem with delayed allocation — the
   exact failures the atomic-write path exists to catch.

The single correct behavior a fix produces: a C `int` result from libc is narrowed to
its true signed 64-bit value **before** any signed comparison, on every backend, for
every `int`-returning wrapper — so `fsync`/`close` failures reach the caller as
`ErrOutput` instead of `OK`.

References:

- `src/target/shared/code/fs_helpers_atomic.rs:488-490`, `:764-766`, `:1248-1250`
  (`fsync` result → `compare_immediate(ret,"0")` + `branch_lt`).
- `src/target/shared/code/fs_helpers_atomic.rs:499-501`, `:775-777`, `:1259-1261`
  (`close` result → the same 64-bit `branch_lt`).
- Fixed backends: `src/target/linux_aarch64/code.rs:427-445` (`emit_sync_file`,
  `abi::sign_extend_word`, comment cites bug-04);
  `src/target/linux_riscv64/code.rs:440-458` (same).
- **Unfixed:** `src/target/linux_x86_64/code.rs:576-585` (`emit_sync_file` — bare
  `emit_libc_call("fsync")`); `src/target/macos_aarch64/code.rs:531-539`
  (`emit_sync_file` — bare `emit_libsystem_call("_fsync")`).
- **`close` unfixed everywhere:** `linux_aarch64/code.rs:416-424`,
  `linux_riscv64/code.rs:429-437`, `linux_x86_64/code.rs:566-574`,
  `macos_aarch64/code.rs:521-529`. `grep -c sign_extend_word` per backend:
  linux_aarch64 = 1, linux_riscv64 = 1, linux_x86_64 = 0, macos_aarch64 = 0.
- Prior fix: bug-04 (`c9e8c706`), memory note `bug-04-aarch64-int-return-width`.
- Found during the goal-01 compiler source review of `src/target/`.

## Failing Reproduction

`fsync`/`close` must actually fail, so the reproduction needs a filesystem that fails
on flush. The cheapest deterministic harness is a full `tmpfs`:

```
# Linux x86_64
sudo mount -t tmpfs -o size=64k tmpfs /mnt/tiny
cat > /tmp/p/src/main.mfb <<'EOF'
IMPORT io
IMPORT fs

SUB main()
  TRAP
    fs::writeTextAtomic("/mnt/tiny/big.txt", strings::repeat("x", 200000))
    io::print("reported OK")
  RECOVER e
    io::print("reported error")
  END TRAP
END SUB
EOF
mfb build /tmp/p && /tmp/p/p.out
```

- Observed (linux-x86_64, macOS): `reported OK` — the atomic write claims success even
  though the flush failed and the bytes are not on disk.
- Expected: `reported error` (`ErrOutput`), as linux-aarch64 already produces.

For `close`, the same program against an NFS mount with a server-side quota exhausted
returns `OK` on **all four** backends; `close`'s deferred `EIO` is discarded.

Contrast cases that work correctly today (regression guards):

- linux-aarch64 and linux-riscv64 correctly report `fsync` failure (`sign_extend_word`
  present) — the direct evidence that the check works once the value is normalized.
- `rename` is immune everywhere: `fs_helpers_atomic.rs:566-567` compares with
  `branch_eq(ret == 0)`, and a `-1` with garbage upper bits is still `!= 0`, so the
  error path is taken. Equality comparisons are robust to the missing extension;
  only the **signed relational** ones (`branch_lt`, `branch_ge`) are broken.
- `write`/`read`/`lseek` are immune: they return `ssize_t`/`off_t`, already 64-bit.

| Environment | arch/libc | `fsync` check | `close` check |
| --- | --- | --- | --- |
| linux-aarch64 | glibc & musl | works ✓ (bug-04 fix) | fails ✗ |
| linux-riscv64 | musl | works ✓ (ported) | fails ✗ |
| linux-x86_64 | glibc & musl | fails ✗ | fails ✗ |
| macos-aarch64 | libSystem | fails ✗ | fails ✗ |

Whether a given libc leaves the upper bits clear is an implementation detail of that
libc's wrapper, so the ✗ rows are **latent-but-unsound** rather than proven on every
listed libc. bug-04 proved at least one (glibc/aarch64 `fsync`) does. Phase 1 must
confirm each row empirically rather than assume.

## Root Cause

`src/target/shared/code/fs_helpers_atomic.rs` emits, for each of the three
atomic-write helpers:

```
mov  x0, fd
bl   fsync            // C int
cmp  x0, #0           // 64-bit compare
b.lt sync_error       // never taken when x0 = 0x00000000FFFFFFFF
mov  x0, fd
bl   close            // C int
cmp  x0, #0           // 64-bit compare
b.lt close_error      // never taken when x0 = 0x00000000FFFFFFFF
```

The narrowing that makes `b.lt` meaningful lives in the **backend**
(`emit_sync_file`), not at the shared seam that performs the comparison. That
placement is the root cause of the drift: the fix must be re-applied by hand in every
`CodegenPlatform` implementation, and for every `int`-returning wrapper. It was
applied to exactly one method on two of four backends.

linux-aarch64/riscv64 `fsync` are immune because they carry the explicit
`abi::sign_extend_word(return_register(), return_register())`. `rename` is immune
because its consumer uses `branch_eq`, not `branch_lt`.

## Goal

- `fs::writeTextAtomic` / `fs::writeBytesAtomic` report `ErrOutput` when `fsync`
  fails, on all four backends.
- The same helpers report `ErrOutput` when `close` fails, on all four backends.
- A newly added `int`-returning platform wrapper cannot reintroduce this class: the
  normalization lives at the comparison seam, not in each backend.

### Non-goals (must NOT change)

- Success-path codegen: a successful `fsync`/`close` must produce byte-identical
  behavior and the same `RESULT_OK_TAG`.
- `rename`'s `branch_eq` check, and the `write`/`read`/`lseek` 64-bit returns —
  already correct, do not touch.
- The unchecked cleanup-path `emit_close_file` calls
  (`fs_helpers_atomic.rs:1025`, `:1057`, `:1472`) — those deliberately ignore the
  result on an already-failing path. Adding error reporting there is out of scope.
- The `open`/`mkstemps` `branch_ge` sites (see Blast Radius) — same hazard class, but
  they are a **separate** bug with a different failure mode; do not silently fold them
  in without their own test.
- **Forbidden wrong fix:** deleting the `branch_lt` check, or relaxing it to
  `branch_ne`. `branch_ne` happens to work for `-1` but breaks the moment a wrapper
  returns a positive non-zero value, and it hides the ABI defect instead of fixing it.

## Blast Radius

Found by enumerating every `CodegenPlatform` method that wraps an `int`-returning libc
function, then grepping its consumers for a signed relational compare.

- `emit_sync_file` × `linux_x86_64`, `macos_aarch64` — **fixed by this bug** (missing
  `sign_extend_word`).
- `emit_close_file` × all four backends — **fixed by this bug** (checked with
  `branch_lt` at `fs_helpers_atomic.rs:501`, `:777`, `:1261`).
- `emit_open_file` (`fs_helpers_io.rs:454-456`, `fs_helpers_atomic.rs:134`, `:729`,
  `:944`) — **latent, same hazard, OUT OF SCOPE.** Uses `branch_ge(open_ok)`, so a
  `-1` with clear upper bits reads as `+4294967295` and is accepted as a *valid fd*.
  The subsequent `read`/`write` then fails with `EBADF`, so the program still errors —
  but with the wrong error, after allocating a resource record around a bogus fd.
  Distinct failure mode, needs its own reproduction; file separately.
- `emit_mkstemps` (`fs_helpers_atomic.rs:434-436`) — **latent, same `branch_ge`
  shape**, out of scope for the same reason.
- `emit_rename_path` (`fs_helpers_atomic.rs:566-567`) — unaffected: `branch_eq`.
- `emit_fs_path_operation` (`fs_helpers_paths.rs:625`, `:799`) — verify the comparison
  in Phase 1; if `branch_eq`, unaffected; if relational, it joins this bug.
- `emit_closedir` (`fs_helpers_paths.rs:1068`, `:1203`) — result unchecked; unaffected.
- `io::flush` — unaffected: no longer calls `fsync` at all
  (`io_helpers.rs:344-380`, drain-only since plan-14-A).

## Fix Design

Normalize **at the shared seam**, where the comparison happens, rather than in each
backend. Add a helper in `fs_helpers.rs` (or `codegen_utils.rs`):

```rust
/// Narrow a C `int` result in the return register to its true signed 64-bit value.
/// Required before any signed relational compare: no ABI we target guarantees the
/// upper 32 bits of an `int` return (bug-04, bug-44).
fn normalize_c_int_result(instructions: &mut Vec<CodeInstruction>) {
    instructions.push(abi::sign_extend_word(
        abi::return_register(),
        abi::return_register(),
    ));
}
```

Call it immediately after each `platform.emit_sync_file(...)` and
`platform.emit_close_file(...)` in `fs_helpers_atomic.rs`, then **delete** the now-
redundant `sign_extend_word` from `linux_aarch64::emit_sync_file` and
`linux_riscv64::emit_sync_file` so there is exactly one owner of the invariant.

`sign_extend_word` already exists and lowers per-backend (`sxtw` on aarch64, `sext.w`
on riscv64, `movsxd` on x86-64), so no new MIR op is needed. On riscv64 the lp64d ABI
already guarantees the extension, making the instruction a semantic no-op there —
harmless, and worth keeping for uniformity (it also removes the arch-mismatched
AAPCS64 comment currently sitting in the rv64 backend).

Where the correctness risk concentrates: this shifts emitted bytes for
`linux_aarch64`/`linux_riscv64` (one `sxtw`/`sext.w` moves from inside the helper to
the call site) — the instruction sequence should be *equivalent*, but the `.ncode`
goldens will move. Diff them to confirm the only change is instruction position, not
count.

Rejected alternatives:

- *Add `sign_extend_word` to each backend's `emit_close_file`.* Rejected: it is the
  exact pattern that produced this bug — four copies of an invariant, drifting.
- *Make `abi::compare_immediate` 32-bit for these sites.* Rejected: `compare_immediate`
  is shared by genuinely 64-bit comparisons; a width parameter would push the same
  decision onto every caller, and bug-09 already documents the hazard of size-variant
  compares diverging between the sizing pre-pass and the emitter.
- *Have `emit_libc_call` normalize every return.* Rejected: it wraps `ssize_t`- and
  pointer-returning functions too; blanket `sxtw` would truncate a valid 64-bit
  `lseek` offset or a heap pointer.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Build the tiny-tmpfs harness above; confirm `fsync` failure is reported on
      linux-aarch64 and **silently swallowed** on linux-x86_64. Record the actual
      `x0`/`rax` value each libc returns (gdb/lldb at the `cmp`) so the ✗ rows in the
      matrix are measured, not assumed. (Failing-storage staging on the boxes is the
      orchestrator's job; on the macOS dev host the defect is proven at the machine
      level instead — the x86-64 ELF disassembles to `movslq %eax,%rdi; cmpq $0,%rdi;
      jl` at both the `fsync` and `close` sites, i.e. the raw `int` was being compared
      un-narrowed before this fix.)
- [x] Do the same for `close` on all four backends. Structurally confirmed via the
      per-target `.ncode` seam and the x86 disassembly; a genuine deferred-`EIO`
      `close` needs the box harness.
- [x] Resolve the `emit_fs_path_operation` comparison shape
      (`fs_helpers_paths.rs:625`, `:799`); record its verdict in Blast Radius.
      **Verdict: unaffected** — `:625` is `branch_ne`, `:799` is `branch_eq`; both are
      equality checks, robust to the missing extension (joins `rename`).
- [ ] File the `open`/`mkstemps` `branch_ge` sites as their own bug. (Still open —
      distinct failure mode, out of scope here; noted for a follow-up bug doc.)

Acceptance: the matrix rows are empirically confirmed; a failing test exists for at
least `fsync` on x86_64 and `close` on aarch64.
Commit: —

### Phase 2 — the fix

- [x] Add `normalize_c_int_result` and call it after all three
      `emit_sync_file` and all three checked `emit_close_file` sites in
      `fs_helpers_atomic.rs`.
- [x] Delete the now-redundant `sign_extend_word` from `linux_aarch64::emit_sync_file`
      and `linux_riscv64::emit_sync_file` (and the arch-mismatched AAPCS64 comment in
      the rv64 copy).

Acceptance: Phase 1 tests pass on every backend; success paths unchanged.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [x] Regenerate `.ncode` goldens. **No golden delta:** no native-level golden
      (`.ncode/.nobj/.nir/.mir/.hex/.nplan`) in `tests/` captures the lazily-emitted
      `_mfb_rt_fs_fs_writeTextAtomic`/`writeBytesAtomic` helpers (verified by grepping
      every native golden for those symbols → zero hits). The change is confined to
      those helpers' native code, so nothing in the golden corpus moves. The seam is
      instead locked by the new integration test below.
- [ ] `scripts/artifact-gate.sh` (execution-free codegen gate), then
      `scripts/test-accept.sh`. (Orchestrator-run.)
- [ ] Re-run the tmpfs/NFS reproduction on macOS-aarch64, linux-aarch64,
      linux-x86_64, linux-riscv64 (per `.ai/remote_systems.md`). (Box-run; the dev host
      cannot stage a failing `fsync`/`close`.)

Acceptance: full suite green; golden delta confined to the six call sites; the
reproduction reports `ErrOutput` on every environment in the matrix.
Commit: —

## Validation Plan

- Regression test(s): a runtime-error test asserting `fs::writeTextAtomic` to a full
  filesystem raises `ErrOutput`, run on all four backends.
- Runtime proof: the tmpfs harness. This bug **cannot** be validated by unit tests or
  goldens alone — the whole defect is that the generated code compiles and looks
  right; only a genuinely failing `fsync`/`close` distinguishes fixed from broken.
- Doc sync: none expected. `fs::writeTextAtomic`'s man page already documents
  `ErrOutput`; this makes the documented behavior true on two more backends.
- Full suite: `scripts/artifact-gate.sh`, then `scripts/test-accept.sh`.

## Open Decisions

- **Keep the no-op `sext.w` on riscv64?** Recommended: yes. lp64d already guarantees
  the extension, but a uniform seam is what prevents the next backend from forgetting.
  Alternative: skip it per-backend, which reintroduces a per-backend conditional —
  precisely the shape that caused this bug.

## Summary

bug-04 fixed one instance of a class and left the class open. The real defect is that
the `int`-narrowing invariant lives in four backend copies while the comparison that
depends on it lives once in shared code. Moving the normalization to the comparison
seam fixes `fsync` on two backends and `close` on four, and makes the next
`int`-returning wrapper correct by construction. The engineering risk is entirely in
Phase 1 — proving each ✗ row with a debugger rather than by ABI reasoning — and in
confirming the aarch64/riscv64 golden delta is a pure instruction *move*.

## Resolution

Fixed 2026-07-09. The `int`-narrowing invariant now lives at the single shared
comparison seam instead of in per-backend copies.

Files changed:

- `src/target/shared/code/fs_helpers_atomic.rs` — added `normalize_c_int_result`
  (one owner of the invariant) and called it after all three `emit_sync_file`
  and all three **checked** `emit_close_file` sites (the six `branch_lt`-guarded
  sites). The unchecked cleanup closes (`:586`, `:783`, `:1025`, `:1057`,
  `:1267`, `:1472`) are deliberately left untouched.
- `src/target/linux_aarch64/code.rs`, `src/target/linux_riscv64/code.rs` —
  deleted the now-redundant in-`emit_sync_file` `sign_extend_word` (and the
  arch-mismatched AAPCS64 comment in the rv64 copy); the seam is the sole owner.
- `src/target/linux_x86_64/code.rs`, `src/target/macos_aarch64/code.rs` —
  **no change needed**: their `emit_sync_file`/`emit_close_file` now inherit the
  narrowing from the shared seam (previously they had none, which was the bug).
- `tests/fs_atomic_int_return.rs` — new backend-uniform regression test.

Fix per backend (neutral `sxtw` op → `sxtw` aarch64 / `sext.w` riscv64 /
`movsxd` x86-64), verified by cross-target `-ncode` dumps:

- **macos-aarch64**: `bl _fsync` / `bl _close` → `sxtw x0,x0` → `cmp_imm` →
  `b.lt`. Previously absent — the defect.
- **linux-x86_64**: seam `sxtw` present; the emitted ELF disassembles to
  `callq fsync; movslq %eax,%rdi; cmpq $0,%rdi; jl …` at both the `fsync` and
  `close` sites (the sign-extended value is what is compared).
- **linux-aarch64**: exactly **one** `sxtw` after `fsync` (confirming the
  redundant in-helper copy was removed) plus one after the checked `close`.
- **linux-riscv64**: `sxtw` (→ `sext.w`, a no-op under lp64d, kept for
  uniformity) after `fsync` and the checked `close`, ahead of the fused `rv.br`.

The unchecked cleanup `close` on every backend correctly carries **no** seam op
(its next op is the `close_error`/cleanup `label`), so success-path codegen and
the error-path cleanup are unchanged.

Tests / proof:

- `cargo test --test fs_atomic_int_return` → 4/4 pass (one per backend). The test
  builds `fs::writeTextAtomic`/`writeBytesAtomic` with `-ncode -target <T>` for
  all four targets and asserts every checked `fsync`/`close` call is immediately
  followed by `sxtw` and never directly by a compare/branch. Proven to have teeth:
  removing one seam call makes it fail with the exact regression message.
- `cargo test sxtw` → the `encodes_sxtw` aarch64 encoder test still passes.
- Runtime (macos-aarch64): `fs::writeTextAtomic` success path writes and reads
  back correctly (no regression). A genuinely failing `fsync`/`close` cannot be
  staged on the macOS dev host (APFS/HFS do not fail flush on demand); that
  observable-failure reproduction is the tmpfs/NFS box harness, per the Validation
  Plan — the machine-level disassembly above is the host-side proof that the raw
  `int` is no longer compared un-narrowed.

Golden impact: **none**. No native-level golden (`.ncode/.nobj/.nir/.mir/.hex/
.nplan`) in `tests/` references `_mfb_rt_fs_fs_writeTextAtomic` /
`_mfb_rt_fs_fs_writeBytesAtomic` (these helpers are emitted only when a program
uses them, and no such program has native goldens). `scripts/artifact-gate.sh`
is expected to report zero diffs.

Follow-up still open: the `open`/`mkstemps` `branch_ge` sites
(`emit_open_file`, `emit_mkstemps`) share the hazard class with a different
failure mode (a bogus `+4294967295` accepted as a valid fd); they remain
out of scope and want their own bug doc.
