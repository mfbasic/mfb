# bug-458: `os::uptime` raises `ErrUnsupported` on the Linux CI runner — `sysinfo(2)` returns non-zero, killing the program mid-fixture

Last updated: 2026-08-30
Effort: small — root-caused; the fix is one instruction
Severity: **HIGH** (raised from MEDIUM: the observed `ErrUnsupported` is the
*lucky* outcome — see "Root Cause". The same defect can instead corrupt memory
silently and return a wrong answer.)
Class: Correctness (unstaged ABI argument; implemented call fails or corrupts at runtime)

Status: **Root-caused and fixed on branch `worktree-winpass` (peer session
mfb-39) — NOT on `main`.** Keep this open until that fix lands; `origin/main`
at `980c79f2e` still has the defect.
Regression Test: — (`tests/acceptance` fixture `rt-behavior/os/func_os_system_status_valid` covers it and is red on Linux CI; see the coverage hole below for why nothing else caught it)

`os::uptime()` is implemented, gated in, and import-wired on Linux, but on the
GitHub Actions Linux runner it raises `ErrUnsupported` (`7-705-0007`) at
runtime. The raise is uncaught, so the fixture program dies at exit 255 and
every later line of the fixture never executes. **The single correct behavior a
fix produces: `os::uptime()` on Linux returns the host uptime in whole seconds,
as it does on macOS and Windows, and `func_os_system_status_valid` prints its
full expected output.**

This is NOT a missing-platform-implementation bug. `os.uptime` is present in
the Linux supported-calls set, its `sysinfo` libc import is wired, and
`func_uptime.rs` has a real `PlatformFamily::Linux` arm. The failure is the
*runtime* result of that arm's `sysinfo` call.

References:

- Reported by peer session mfb-39 from GitHub Actions job logs: main's own run
  (job 99339723239) and their `worktree-winpass` branch run (99351860186).
  Output is byte-identical on both, so it is pre-existing on main and
  independent of any in-flight branch work.
- Memory note `ci-jobs-run-on-linux-debug-not-mac-release.md` — this is
  invisible to a local macOS gate; `test-accept.sh` on macOS passes 1307/1307.
- Memory note `mfb-exe-tests-use-release-binary.md` — the stale-binary
  explanation was ruled out: CI's `build` job compiles `mfb-bin` from the
  commit under test and each job downloads that artifact on a fresh runner.

## Failing Reproduction

Linux only. On a Linux host (or the CI runner):

```
target/release/mfb test tests/acceptance
# or narrow it:
./scripts/test-accept.sh target/release/mfb /tmp/accept-out 'func_os_system_status_valid'
```

Observed `build.log` delta (`-` golden, `+` actual):

```
 version_nonempty=TRUE
-uptime_nonnegative=TRUE
-admin_bool=TRUE
+Error: 7-705-0007
+Operation is not supported by the implementation or platform.
+[exit 255]
```

| Environment | Result |
| --- | --- |
| Linux CI runner (GitHub Actions, debug artifact) | fails ✗ |
| macos-aarch64, release, local | passes ✓ (1307/1307) |

**`os::isAdmin` is not failing.** `version_nonempty` printed, then `uptime`
raised and the program exited 255 — `admin_bool` never ran. `func_is_admin.rs`
has no `ErrUnsupported` path at all (non-Windows is a plain `geteuid`), which
corroborates it. The two missing golden lines are one failing call plus one
line of collateral, not two failing calls.

## Root Cause — CONFIRMED

**`lower_uptime`'s Linux arm never stages ARG[0].** `sysinfo(struct sysinfo *info)`
takes a pointer to the buffer it fills. The buffer is meant to be the frame
(`stack_size = 128`, and the following `load_u64(seconds, sp, 0)` reads
`struct sysinfo`'s leading `long uptime` back out) — but the address was never
passed. Emitted code:

```
sub_sp 152
bl sysinfo          <- ARG[0] = whatever the caller happened to leave there
cmp rax, 0
b.ne fail
ldr r10, [rsp+16]
```

Found by peer session mfb-39 from the candidate list below; it was **#3 (does a
valid pointer reach the call), not #1 (glibc vs musl)**.

### Why the severity is HIGH, not MEDIUM

The `ErrUnsupported` we observed is the *benign* branch. Whatever junk is in
ARG[0] decides which of two things happens:

- **Unmapped junk** → `EFAULT` → `sysinfo` returns `-1` → `ErrUnsupported`.
  Loud, safe, and what CI happened to hit.
- **Writable junk** → `sysinfo` returns **0** after writing a 112-byte
  `struct sysinfo` over an arbitrary address, and the load then reports a
  garbage uptime from a slot nothing wrote. **Silent memory corruption plus a
  wrong answer, no error at all.**

Which branch you get is down to register residue from the caller. Any Linux
program that called `os::uptime()` may have hit the silent one.

### The fix

One instruction — `add_immediate(c_arg(0), stack_pointer(), 0)`, the same form
the macOS arm already uses to stage every `sysctl` argument. It must be
`add_immediate` from `sp` rather than a raw pointer: `finalize_frame` shifts
sp-relative accesses past the callee-saved area, and shifts this along with the
load, so both land on the same address (`add_imm rdi, rsp, 16` against
`ldr [rsp+16]`).

### Verified, and the libc axis is eliminated

Baseline vs fixed compiler, executed on real hosts — Ubuntu x86-64 (glibc) and
Alpine x86-64 (musl) **both** go `Error: 7-705-0007 / exit 255` →
`uptime_nonnegative=TRUE / exit 0`. So Phase 1's glibc-vs-musl split is
*eliminated*, not halved, and it reproduces off CI entirely: "runner artifact"
was never on the table. The fixture's output now matches its committed golden
with **no golden touched**, and `artifact-gate all` is 0 diffs.

### Sibling audit (clean)

Everything else in `builtins/os` that takes arguments stages them. `getuid` →
`getpwuid` looks like the same shape but is safe: the external-call seam emits
`mov rdi, rax` after each call, and on SysV `rdi` is both the MFB return bank
and ARG[0], so the uid lands correctly by construction. `sysinfo` was unique in
needing a *frame pointer* staged rather than a value flowing out of a previous
call.

### Why this survived: a coverage hole

`tests/byte-identity/os` never calls `os::uptime` — it covers
`arch`/`args`/`cpuCount`/`environ`/… So `artifact-gate` reporting 0 diffs after
the fix is *correct*, not a missed rebuild: no golden exercises this path. The
acceptance fixture was the only thing touching it, and it was red and being
read as environmental. **Worth closing that hole regardless of this bug.**

---

### Original narrowing (kept for the record)

The mechanism was narrowed to a single branch before the root cause was found:

`src/codegen/builtins/os/func_uptime.rs:23` is the Linux arm:

```rust
PlatformFamily::Linux => {
    // struct sysinfo starts with long uptime at offset 0.
    ctx.platform.emit_external_call("sysinfo", ...)?;
    builder.instructions.extend([
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_ne(&fail),                       // <-- taken
        abi::load_u64(&seconds, abi::stack_pointer(), 0),
    ]);
    builder.stack_size = 128;
}
```

`ErrUnsupported` is raised at `func_uptime.rs:117`, reachable only via the
`fail` label. On the Linux arm the only edge into `fail` is that `branch_ne`.
Therefore **`sysinfo(2)` returned non-zero on the runner.**

Supporting evidence that the call is otherwise fully wired:

- `src/target/linux_common/mod.rs:79` — `"os.uptime"` in the supported set.
- `src/target/linux_common/plan.rs:219` — `"os.uptime" => libc_import("sysinfo")`.

Candidate causes, in the order worth checking:

1. **glibc vs musl.** A Linux build emits both `-glibc.out` and `-musl.out`.
   If only one of the two fails, that is the answer. `sysinfo` exists in both
   but the static-musl path is the likelier suspect. *Check this first — it is
   one line of CI log and it either splits the problem in half or eliminates a
   whole branch.*
2. **The buffer.** `builder.stack_size = 128` and the result is loaded from
   `sp+0`. `struct sysinfo` is 112 bytes on 64-bit glibc plus padding; confirm
   the pointer actually passed to `sysinfo` is that reservation and that 128 is
   large enough for the musl definition too.
3. **The argument register.** Confirm the `sysinfo` call is passed a valid
   pointer at all — a null or garbage pointer makes `sysinfo` return `-1`
   (`EFAULT`), which is exactly the observed branch.
4. **Container/seccomp.** Less likely (`sysinfo` is not commonly filtered), but
   cheap to rule out with a two-line C program on the runner image.

## Goal

- `os::uptime()` returns real uptime seconds on Linux (both libc flavors), and
  `rt-behavior/os/func_os_system_status_valid` passes on the Linux CI row.

### Non-goals (must NOT change)

- The macOS and Windows arms of `func_uptime.rs`, and their goldens —
  byte-untouched.
- `func_is_admin.rs` — it is not implicated; do not "fix" it.
- Do **not** resolve this by relaxing the fixture (gating the assertion,
  accepting `ErrUnsupported` as valid) unless a check above proves the
  platform genuinely cannot answer. `os::uptime` on Linux is documented as
  `sysinfo`-backed (`func_uptime.rs:128`); weakening the test would hide a
  real runtime failure of a shipped, documented API.
- Do not change the `ErrUnsupported` raise into a different error to make the
  message nicer — the error is a symptom, not the bug.

## Blast Radius

- `src/codegen/builtins/os/func_uptime.rs` Linux arm — the fix.
- Linux `.ncodesum` goldens for every fixture importing `os` — expect a ripple
  if the lowering changes (the Linux analogue of the Win64 ripple in memory
  note `win64-change-ripples-all-io-importers`); re-sync all of them, not just
  the `os` fixture.
- macOS/Windows arms and their goldens — unaffected.
- `func_os_system_status_valid`'s golden — unchanged by a correct fix (the
  golden already records the passing output).

## Phases

### Phase 1 — split the axis (no behavior change)

- [ ] From the CI logs, determine whether `-glibc.out`, `-musl.out`, or both
      fail. Record the answer here.
- [ ] On a Linux host, run a minimal `sysinfo` C program to confirm the
      syscall itself works on that image.
- [ ] Confirm from the emitted `.ncode` that the pointer handed to `sysinfo`
      is the 128-byte stack reservation.

Acceptance: the failing libc flavor(s) named, and `sysinfo`-works-on-image
answered yes/no.
Commit: —

### Phase 2 — the fix

- [ ] Correct whichever of (buffer size / argument setup / flavor-specific
      lowering) Phase 1 identified.

Acceptance: `func_os_system_status_valid` passes on Linux for every emitted
flavor; `os::uptime()` returns a plausible non-negative value.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `artifact-gate.sh all` — classify the Linux `.ncodesum` delta; regen
      exactly the affected fixtures. Run it **uncontended** (memory note
      `contended-artifact-gate-reports-phantom-diffs`; `exit=98` is "refused",
      not "diffs found").
- [ ] `cargo test --no-fail-fast`; full `test-accept.sh` on macOS (must stay
      1307/1307).
- [ ] Green Linux CI row.

Acceptance: suite green on both axes; golden delta is exactly the Linux `os`
importers.
Commit: —

## Related, but out of scope

The same CI acceptance job reports three ICMP mismatches
(`rt-behavior/net/func_net_ping_valid`,
`rt-error/net/func_net_ping_range_invalid`, and the `func_net_ping_valid`
`build.log`), all from the runner denying raw-socket permission
("Network operation failed before a connection was established"). That is a
separate, already-recognized axis — see `scripts/check-icmp-permission.sh` and
`scripts/icmp-capability-probe.c` — and needs capability gating on those
fixtures rather than anything in `os`. Not fixed here; worth its own bug if one
does not already exist.
