# bug-71: stale/dead dynamic-symbol imports and arch-mismatched comments across the per-target backends (x86 `_exit`/`getentropy`, `io.flush` fsync+errno on all three, riscv `sext.w` rationale, riscv musl-flavored diagnostic dumps)

Last updated: 2026-07-09
Effort: small (<1h)

A cluster of LOW-severity dead-code / doc defects in the per-target Linux backends, all
"a plan declares an import the emitted code never references" or "a comment describes a
different architecture". None affects the shipped executable's behavior; they add unused
dynamic-symbol metadata to the import plan or mislead a maintainer.

The single correct behavior a fix produces: each backend's import plan declares only the
libc symbols its emitted code actually calls, and per-arch comments describe that arch.

References (paths under `src/target/`):

- **x86 `_exit` import never called.** `linux_x86_64/plan.rs:71-73` (`program_exit_imports`)
  adds `_exit` for any program with an explicit exit statement, but `linux_x86_64/code.rs:180-181`
  (`emit_program_exit`) terminates via the raw `exit_group` (nr 231) syscall — no `bl _exit`,
  no relocation. Copied from `linux_aarch64/plan.rs:69-71` (where `_exit` *is* called) without
  adjusting for the x86 raw-syscall exit. Fix: return `Vec::new()` from x86
  `program_exit_imports`.
- **x86 `getentropy` import never called.** `linux_x86_64/plan.rs:194` imports `getentropy`
  for `fs.createTempFile`, but x86 `emit_random_bytes` uses the raw `getrandom` syscall, so
  the import is unreferenced. Fix: drop the x86 `getentropy` import.
- **`io.flush` imports `fsync` + errno on all three Linux backends but the helper is
  drain-only.** `linux_x86_64/plan.rs:110-113` (`fsync`+`__errno_location`),
  `macos_aarch64/plan.rs:220-231` (`_fsync`+`___error`), and the linux_aarch64 sibling — but
  `io_helpers.rs:344-402` (`lower_io_flush_helper`) is drain-only since plan-14-A: it calls
  `STDOUT_DRAIN` and never fsyncs / reads errno. The two imports are dead on every backend.
  Fix: drop `fsync` + errno from the `io.flush` arm on all three platforms. (Verify `io.input`
  in the same arm, which may share the stale `fsync` import, before touching it.)
- **riscv `sext.w` rationale cites AAPCS64.** `linux_riscv64/code.rs:emit_sync_file` (~`:440-458`)
  carries the aarch64 comment "AAPCS64 leaves x0[63:32] unspecified" to justify a
  `sign_extend_word` that on RISC-V lp64d is a guaranteed no-op (the ABI sign-extends 32-bit
  `int` results). Copied verbatim from `linux_aarch64/code.rs`. Correct result either way;
  the comment is arch-mismatched. (Coordinate with bug-44, which moves the `int`-narrowing to
  the shared seam — the riscv comment goes away with it.)
- **riscv diagnostic dumps use `LinuxFlavor::Musl` while aarch64 uses `Glibc`.**
  `linux_riscv64/mod.rs` `write_native_plan`/`write_native_object_plan`/`write_native_code_plan`/
  `write_mir` (~`:324-375`) bake musl library names into the single-flavor `.nplan`/`.nobj`/
  `.ncode`/`.mir` dumps, whereas `linux_aarch64/mod.rs` (~`:307-355`) uses glibc names. Shipped
  executables are unaffected (`write_executable` builds both flavors on both targets); only the
  diagnostic dumps diverge, which a cross-target golden diff would flag. Fix: pick one flavor
  convention for diagnostic dumps across all Linux backends (glibc, matching aarch64), or
  document why riscv differs.

- Found during the goal-01 compiler source review of `src/target/`.

## Failing Reproduction

- Import-plan dumps (`mfb ... --nplan` / inspect dynamic symbols) show `_exit` / `getentropy`
  / `fsync` / `__errno_location` for programs that never call them.
- A cross-target diff of `.ncode`/`.nplan` dumps shows riscv baking musl names where aarch64
  bakes glibc.

- Observed: unreferenced dynamic symbols in the plan; flavor-divergent diagnostic dumps;
  arch-mismatched comment.
- Expected: imports match actual call sites; one flavor convention for dumps; per-arch comments.

Contrast: `setenv`, `clock_gettime`, `signal`, `write`, and the other genuinely-called imports
are correct; `write_executable` (the real output path) builds both flavors identically on both
targets.

## Root Cause

Each import list / comment was copied from the aarch64 backend without adjusting for the target
(x86 raw-syscall exit and getrandom; the drain-only io.flush rewrite that outdated the fsync
imports; the riscv ABI's guaranteed sign-extension; the riscv-was-validated-on-musl default for
diagnostic dumps).

## Goal

- Each backend's `runtime_imports`/`program_exit_imports` declare only referenced symbols.
- One flavor convention for single-flavor diagnostic dumps across the Linux backends.
- Per-arch comments describe the arch they annotate.

### Non-goals (must NOT change)

- The shipped executables (`write_executable` dual-flavor output).
- Genuinely-called imports.
- The bug-44 `int`-narrowing behavior (this only touches the riscv *comment*).

## Blast Radius

Each `file:symbol` above — independent, metadata/comment only. No runtime code change.

## Fix Design

Per the fix note on each bullet: prune the dead imports, unify the diagnostic-dump flavor, and
correct/remove the riscv comment (or let bug-44 subsume it).

## Phases

### Phase 1 — audit

- [x] Confirm each import is unreferenced (grep the emitter for the call). Confirm the io.flush
      `io.input` sibling before pruning.

### Phase 2 — the fixes

- [x] Prune the imports; unify the dump flavor; fix the comment.

### Phase 3 — validation

- [x] Regenerate import-plan / diagnostic-dump goldens (delta = removed symbols / flavor
      alignment); `scripts/test-accept.sh`. Shipped executables byte-identical.

## Validation Plan

- Regression test(s): import-plan goldens without the dead symbols; cross-target dump-flavor
  consistency.
- Runtime proof: none needed (executables unchanged).
- Doc sync: the riscv comment.
- Full suite: `scripts/test-accept.sh`.

## Summary

Copied-from-aarch64 import lists and comments left several backends declaring libc symbols they
never call (`_exit`, `getentropy`, `fsync`+errno for the now-drain-only `io.flush`), an
arch-mismatched `sext.w` rationale on riscv, and musl-flavored diagnostic dumps where aarch64
uses glibc. All LOW, all metadata/comment-only; the shipped executables are unaffected.

## Resolution

Fixed 2026-07-09. Scope note: the prompt reassigned "all three backends" for the `io.flush`
fix to the three this agent owns — `linux_x86_64`, `linux_riscv64`, `macos_aarch64` — and
explicitly barred touching `linux_aarch64` (another agent owns its identical `io.flush` prune).

Items (each dead import proven unreferenced by grepping the emitter):

- **x86 `_exit`** — `emit_program_exit` (`linux_x86_64/code.rs:180`) terminates via the raw
  `exit_group` (nr 231) syscall; no `bl _exit`, no relocation. `program_exit_imports` now
  returns `Vec::new()`.
- **x86 `getentropy` for `fs.createTempFile`** — `fs.createTempFile` draws its suffix through
  `platform.emit_random_bytes`, which on x86 (`code.rs:298`) is the raw `getrandom` syscall
  (nr 318), not libc. Dropped the `fs.createTempFile` getentropy push. Kept the
  `crypto.randomBytes` getentropy import (line 85): `lower_crypto_random_bytes_helper`
  (`shared/code/crypto.rs:86`) emits `emit_libc_call("getentropy")` directly on every platform,
  so it is genuinely called.
- **`io.flush` `fsync`+errno on x86 / riscv / macOS** — `lower_io_flush_helper`
  (`shared/code/io_helpers.rs:390`) is drain-only (calls `STDOUT_DRAIN`, never fsyncs / reads
  errno) since plan-14-A. The `io.flush` arm now returns `Vec::new()` on all three. The drain's
  `write` still comes from the `io.print` arm (verified: a program using `io::print`+`io::flush`
  runs and the emitted plan carries `write` via io.print). The `io.input` sibling arm is a
  *separate* match arm and was left untouched.
- **riscv `sext.w`/AAPCS64 comment** — already fixed by bug-44 (commit 0efab113); the comment at
  `linux_riscv64/code.rs:451` now reads "riscv64's lp64d ABI already sign-extends `int`
  returns". No change needed.
- **Diagnostic-dump flavor** — both `linux_x86_64/mod.rs` and `linux_riscv64/mod.rs` baked
  `LinuxFlavor::Musl` into the single-flavor `.nplan`/`.nobj`/`.ncode`/`.mir` dumps; switched
  both to `LinuxFlavor::Glibc` to match `linux_aarch64`, with a clarifying comment. Shipped
  executables are unaffected (`write_executable` still builds both flavors).

Latent (NOT fixed — out of this bug's scope, and removal would desync `src/docs/spec`):
- x86 `getentropy` for `math.rand`/`math.seed` (`native_call_imports`, plan.rs:324) is *also*
  unreferenced on x86 (entry RNG seed uses `emit_random_bytes` = getrandom syscall), but
  `spec/linker/08_linux-x86_64.md` documents it as present on all targets; left as-is.
- x86/riscv `io.input` `fsync`+errno appear dead too (the prompt path comment in `io_helpers.rs`
  says "No fsync"), but `io.input` is out of this bug's `io.flush` scope; left as-is.

Regression tests (all green): per-backend plan unit tests asserting `program_exit_imports`
empty (x86), `fs.createTempFile` excludes/includes getentropy (x86/riscv), `crypto.randomBytes`
keeps getentropy (x86), and `io.flush` imports nothing (x86/riscv/macOS).

Validation: `cargo test --bins plan::tests` (11 ok), `cargo test --test fs_atomic_int_return`
(4 ok), `cargo test --test native_io_runtime` (19 ok incl. flush-failure + input-flush). Runtime:
built and ran `io::flush`, `crypto::randomBytes`, `fs::createTempFile` on macOS host (correct
output, exit 0); cross-built both flavors of the x86_64 and riscv64 executables successfully.

Golden test dirs that shift: only `tests/syntax/app/macos-app-mode-io` (its
`.macos-aarch64.app.nplan` drops the two `io.flush` `_fsync`/`___error` import lines, and the
`.app.ncode` GOT/stub layout shifts accordingly). No linux-target dumps are stored as goldens
(the accept suite only snapshots `macos-aarch64`), so the x86/riscv import prunes and the
mod.rs flavor unification shift no test goldens.
