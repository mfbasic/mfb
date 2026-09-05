# bug-544: on Windows, `fs::createTempFile` and the atomic writers returned their result in the wrong register

Last updated: 2026-09-04
Effort: medium (3h–1d)
Severity: HIGH (a documented `fs` API failed unconditionally on a supported platform)
Class: correctness / Win64 ABI

Status: Fixed (2026-09-04) — `emit_random_bytes` and `emit_mkstemps` in
`src/target/win_x86_64/code.rs` now stage the C result (`rax`) into the MFB
result register (`rcx`), which is what every shared caller reads. Regression
test: `tests/rt_fs_temp_and_atomic_write.rs`, deliberately platform-neutral so it
RUNS on the Windows CI row. Verified on box 2230 (Win11 x86_64).

## Symptom

On Windows, every call to `fs::createTempFile`, `fs::writeTextAtomic` and
`fs::writeBytesAtomic` raised:

```
Error: 7-702-0002
Write or flush operation failed.
```

The first test to notice was `rt_http_handle_request_serves`, whose MFB server
publishes its port with `fs::writeTextAtomic` — so it hung to its 20-second
deadline and failed with `mfb http server never published its port`, a message
that names neither `fs` nor the cause.

## Root cause

Win64's MFB result register is **not** the C result register. plan-85-A aligned
`mfb_return(0)` onto the call-argument bank, so `return_register()` is `rcx`
(`CALL_ARGS_WIN64[0]`), while `c_return(0)` is `rax`. `emit_linux_c_call` stages
`rax → rcx` for every Windows OS seam that goes through it — the comment there
records plan-110-D, where the same omission had `socket()`/`connect()`/
`getsockname()` all checked against `rcx`, the third outgoing argument.

`win_x86_64`'s `call_external` does **not** stage. Two emitters that use it
directly never made up the difference:

* **`emit_random_bytes`** — carried a comment claiming "the NTSTATUS return is
  ignored". True of the emitter, false of its callers: `gen_temp_file`
  sign-extends the value and takes the error path when it is negative, so a
  garbage `rcx` decided at random whether `fs::createTempFile` had "failed".
* **`emit_mkstemps`** — the sharper case, and the reason a reader misses it:
  it *does* write `return_register()`, but **only on the give-up path** (`-1`).
  The success path fell through `label(&success)` straight to `label(&done)`
  leaving the handle in `rax`, so `gen_atomic_write` sign-extended whatever
  `CreateFileW` left in `rcx` and used it as a descriptor. A partial write is
  what makes this invisible to "does this function assign the result register".

## Attribution (measured, box 2230)

Each fix is independently load-bearing:

| build | `fs::createTempFile()` | `fs::writeTextAtomic` |
| --- | --- | --- |
| pre-fix | **FAIL** `7-702-0002` | — (never reached) |
| `emit_random_bytes` staged only | ok | **FAIL** `7-702-0002` |
| both staged | ok | ok, contents read back correctly |

End-to-end after the fix, the `rt_http_handle_request_serves` server program on
box 2230 publishes its port and answers:

```
HTTP/1.1 200 OK
content-type: text/plain; charset=utf-8
hello from /
```

## Why it survived this long

Nothing ever RAN the atomic-write path on Windows:

* `tests/rt_fs_create_mode_0600.rs` is `#![cfg(unix)]` — it asserts a `0600` mode
  that has no Windows meaning, so the whole file is skipped there.
* `tests/rt_fs_atomic_int_return.rs` is a codegen-INSPECTION test: it reads the
  emitted instruction stream for the `sxtw`/`movsxd` narrowing and executes
  nothing.

So Windows coverage of `fs::createTempFile` and the atomic writers was "it
compiles". `tests/rt_fs_temp_and_atomic_write.rs` closes that: no mode bits, no
path syntax, no `cfg` — create a temp file, write atomically, read it back, then
atomically REPLACE an existing file (the rename half, a different Win32 call than
the create half).

## Goldens

Windows codegen changed, so the sums moved. Measured: `artifact-gate.sh all`
reported **28 diffs, every one `windows-x86_64`, zero on the other six targets** —
the blast radius the change predicts. Regenerated with
`bash scripts/regen-ncodesum.sh target/release/mfb` (141 refreshed, 28 changed,
all windows), gate re-run clean: **1371 tests, 1900 goldens, 0 diffs**.

## Follow-up worth doing

The audit that found these two was "for every `win_x86_64` emitter whose result a
shared caller reads through `return_register()`, is that register written on
EVERY path?" Most emitters pass — `emit_path_exists` and `emit_is_terminal`, for
instance, compute into `return_register()` rather than staging `rax`, which a
naive grep for the staging move reports as a false positive. A mechanical sweep
therefore needs the per-path question, not the "does it contain the move"
question. Not attempted here beyond the fs seams, which are now proven end to end
on box 2230.
