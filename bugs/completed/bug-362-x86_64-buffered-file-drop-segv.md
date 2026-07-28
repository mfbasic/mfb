# bug-362: dropping a buffered `File` segfaults on linux-x86_64

Last updated: 2026-07-19
Effort: medium
Severity: HIGH (SIGSEGV in a shipped program on a supported target; no
diagnostic, no error code, exit 139)
Class: Compiler / native codegen — x86_64 backend, resource teardown

Status: Fixed (2026-07-19)
Regression Test:
- `arch::x86_64::select::tests::a_parameter_read_on_an_arm_before_a_call_still_maps_to_its_argument_register`
  — the helper's exact CFG shape, asserting the enable arm addresses the `File*`
  through `rdi`. **Verified to FAIL against the pre-fix selector** with
  `got Some("rax")`.
- The four committed fixtures (`fs/func_fs_flush_valid`,
  `fs/func_fs_isBuffered_valid`, `fs/func_fs_setBuffered_valid`,
  `resources/resource-reclaim-loop-valid`) are the behavioral proof, run on real
  hardware by `scripts/linux-runtime-proof.sh` on 2227. All four now pass.

## Reproduction

Four lines. No write, no flush, no `tempDirectory` — enabling buffering on a
`File` and letting it fall out of scope is enough.

```
IMPORT fs

FUNC main AS Integer
  w("target/p5.txt")
  RETURN 0
END FUNC

SUB w(path AS String)
  RES f = fs::openFile(path, "write")
  fs::setBuffered(f, TRUE)
END SUB
```

```
$ mfb build --target linux-x86_64 .      # then run on an x86_64 box
Segmentation fault
[exit 139]
```

## Triage

| variant | linux-x86_64/musl | linux-aarch64/musl |
| --- | --- | --- |
| `openFile` + `setBuffered(TRUE)` + drop | **SIGSEGV** | exit 0 |
| `openFile` + `setBuffered(TRUE)` + `writeAll` + drop | **SIGSEGV** | exit 0 |
| `openFile` + `writeAll` + drop (**no** `setBuffered`) | exit 0 | exit 0 |
| `fs::tempDirectory()` alone | exit 0 | exit 0 |

Three things this pins down:

- **`setBuffered(TRUE)` is necessary and sufficient.** The write is irrelevant;
  the unbuffered handle is fine.
- **It is x86_64-specific, not musl-specific.** The same source, same libc
  (musl), same `RES` drop passes on aarch64. That rules out a libc difference and
  points at the x86_64 backend.
- **It is not bug-360.** `fs::tempDirectory()` — the function bug-360 fixed —
  is clean here in isolation. `func_fs_flush_valid` happens to call it, which is
  what made the two look related; they are not.

## Where to look

`lower_fs_set_buffered_helper` (`src/target/shared/code/fs_helpers_io.rs:511`)
is written in target-neutral `abi::` vreg ops, so the helper body itself is
shared with the backends that work. That makes the x86_64 *lowering* of those
ops, or the teardown that runs after them, the more likely home.

The enable path is small — `move_immediate %v1, 1` then
`store_u64 %v1, <File*>, FILE_OFFSET_BUF_ENABLED` — which suggests a hypothesis
worth testing first, **stated as a lead and not as a finding**: the drop path
sees `BUF_ENABLED = 1` and drains a buffer that was never allocated, reading a
buffer pointer that happens to be zero on aarch64 and garbage on x86_64. That
would explain why enabling buffering is sufficient and why writing changes
nothing. It has not been confirmed — confirm by inspecting the emitted x86_64
teardown for the `File` resource and the field's initial value at `openFile`.

## Why it was not found sooner

It has been failing on 2227 the whole time and was recorded in bug-360's triage
as part of a list of x86_64 failures that "were not investigated". It sat there
because the proof harness on that box was itself producing false failures, so
its output had stopped being read closely — the failure mode that
`scripts/linux-runtime-proof.sh`'s three harness fixes (this session) were meant
to end. With the harness now clean on aarch64 and down to nine known failures on
x86_64, four of those nine are this bug.

## Blast radius

`fs::setBuffered(f, TRUE)` is documented and shipped (`mfb man fs setBuffered`,
plan-14-B). Any x86_64 Linux program that enables per-file buffering crashes on
scope exit. macOS and aarch64 Linux are unaffected.

## Root cause (proven)

Not the drop path, and not the lead recorded above — the buffer pointer and
fill count are both zeroed at `openFile`, so `_mfb_rt_fs_file_drain`
short-circuits on `BUF_FILLED == 0` and never touches the unallocated buffer.
The crash is in `fs::setBuffered` itself, in x86 instruction selection.

`remap_x86_abi` (`src/arch/x86_64/select.rs`) resolves each residual `x0`–`x8`
operand to a SysV home by its ABI role. A *use* is treated as an incoming
parameter — and so mapped to `CALL_ARGS[n]` (`rdi`, `rsi`, …) — only while no
call/syscall boundary has been seen and `xN` has not been redefined. Both facts
were tracked by flags advanced in **emitted order** (`boundary_since_entry`,
`defined_since_entry`), not along control flow.

`lower_fs_set_buffered_helper` emits the *disable* arm first, and that arm calls
`_mfb_rt_fs_file_drain`. The *enable* arm is branched to from `entry`, so the
call never executes on its path — but the linear walk had already passed the
`bl` by the time it reached the arm. `is_param_use` was therefore false, and
`map_abi_register` fell through to the arm's next boundary (`ret`) → `RETS[0]`
= `rax`.

Confirmed by dumping the helper for `--target linux-x86_64` with the compiler
before and after the fix:

```
 before:  { "op": "str_u64", "src": "r10", "base": "rax", "offset": "40" }
 after:   { "op": "str_u64", "src": "r10", "base": "rdi", "offset": "40" }
```

`rax` at that point holds whatever the caller left in it, so the helper stored
the buffering flag through a garbage pointer. That explains every entry in the
triage table: enabling buffering is necessary and sufficient (it is the only
path through the enable arm), writing is irrelevant, and aarch64/rv64 are
unaffected because neither remaps.

## Fix

Replace the two linear flags with a forward MUST dataflow over the CFG the
function already builds: `entry_clean[i]` (no call/syscall on ANY path
entry → i) and `entry_undef[i]` (bit n set means `xN` undefined on every such
path), meeting by intersection over predecessors. For a straight-line function
this is identical to the old flags; it diverges exactly where control flow
diverges from emitted order, which is the bug.

## Verified

- 2227 (linux-x86_64/musl), `scripts/linux-runtime-proof.sh`: **460 passed /
  9 failed → 464 passed / 5 failed.** The four that flipped are exactly this
  bug's fixtures. The five remaining are pre-existing —
  `os/func_os_userName_valid` was re-run against the pre-fix compiler and fails
  identically (`7-705-0007`, platform-unsupported).
- aarch64 (2224) 469/0 and riscv64 (2229) 456/0, both unchanged: an A/B
  `linux-artifact-baseline` over 1016 fixtures × 3 targets shows zero
  `linux-aarch64` and zero `linux-riscv64` hash changes.
- Full local acceptance 1016/1016; `cargo test` 3103 passed.

Landed alongside bug-350 (the x86 caller-saved clobber masks), which is why the
x86-64 artifact delta this session covers both.
