# bug-362: dropping a buffered `File` segfaults on linux-x86_64

Last updated: 2026-07-19
Effort: medium
Severity: HIGH (SIGSEGV in a shipped program on a supported target; no
diagnostic, no error code, exit 139)
Class: Compiler / native codegen — x86_64 backend, resource teardown

Status: Open — reproduced and triaged, not fixed
Regression Test: none yet. Four committed fixtures already reproduce it and are
currently failing on 2227 (`fs/func_fs_flush_valid`, `fs/func_fs_isBuffered_valid`,
`fs/func_fs_setBuffered_valid`, `resources/resource-reclaim-loop-valid`); they
pass on every aarch64 box, so a fixture is not what is missing — a runtime gate
on x86_64 is.

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
