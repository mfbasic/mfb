# bug-902: a program started with fd 0/1/2 closed hands those numbers to `fs::open`, so `io::readLine` reads the program's own file and `io::print` writes into it

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Security

Status: Fixed on macOS, Linux aarch64 glibc/musl, Linux x86-64 musl and Linux riscv64 musl/glibc (bs16-agent-c; 2224/2227 runs by the coordinator). Linux x86-64 glibc not run: box 2228 is down, and the owner is not bringing it up for now.
Regression Test: `tests/runtime/rt_closed_std_fds_not_reused.rs` (`cargo test --test rt_closed_std_fds_not_reused`). It closes fd 0, 1, 2 and 1+2 in the child before exec. An rt-behavior fixture cannot start a program with a closed descriptor.

**Owner decision (2026-10-05):** reserve each closed fd 0/1/2 with `/dev/null` at startup, before anything can open a
descriptor (`O_RDONLY` for 0, `O_WRONLY` for 1 and 2), as Rust std and Go do. Reads of a
closed stdin then see EOF, and writes to a closed stdout or stderr succeed silently. No file
the program opens can ever receive fd 0, 1 or 2.

Consequence to carry in the fix: `bugs/completed/bug-723`'s regression test
(`tests/runtime/rt_io_set_buffered_drain_failure.rs`) provokes the write failure by running
with stdout closed (`>&-`), which will no longer fail. The behavior it protects stays: a
failed drain is reported, never silently lost. Only the way the test makes stdout fail
changes, e.g. stdout to a pipe whose reader has exited (EPIPE/SIGPIPE ignored) or a full
device. Make that one change with this proof, and do not weaken the assertion. The same
applies to any other test or spec text that uses `>&-` to make a write fail. Grep `>&-` and
`<&-` in tests/ and src/docs.

The program entry does nothing about standard descriptors that are closed at exec
time. The kernel hands the lowest free number to the next `open`, so the first
`fs::open` in a program launched with `<&-` gets fd 0 and with `>&-` gets fd 1.
After that the runtime's standard streams quietly use the user's file:
`io::readLine()` returns the contents of a file the program opened for reading, and
`io::print` output is appended to a file the program opened for writing. Nothing
fails. The same goes for fd 2 and error banners. An attacker who controls how a
program is launched can therefore make it leak one of its own files through
"stdin" consumers, or corrupt a data file with console output.

**Correct behavior (original proposal; owner chose /dev/null, see decision above):** at startup, before
anything can open a descriptor, the entry checks each of fds 0, 1 and 2 (e.g.
`fcntl(fd, F_GETFD)` returns `EBADF`) and opens `/dev/null` on any that is closed
(`O_RDONLY` for 0, `O_WRONLY` for 1/2). This is what Rust std and Go do. Writes to a
closed stdout would then succeed into `/dev/null` instead of failing. bug-723 relies
on writes to a closed stdout failing, so the owner has to pick one of these:
reserve the fds with `/dev/null`, or keep "closed stream errors" by reserving them
with a descriptor that always fails with `EBADF`-like errors (e.g. a closed pipe
end).

References:

- `src/docs/spec/threading/09_os-integration.md` § Standard Input Broadcast:
  "Standard input (file descriptor 0) is owned by the runtime".
- `src/docs/spec/memory/08_program-startup.md` describes the whole entry sequence and
  has no fd-sanitation step. `src/codegen/engine/function/entry.rs:lower_program_entry`
  has none either (`grep -n "F_GETFD\|/dev/null" src/codegen/engine/function/entry.rs`
  finds nothing).
- bugs/completed/bug-723 (behavior of writes when stdout is closed).
- Found during spec hardening batch B09 (program startup, stdin states).

## Failing Reproduction

```
mfb init fd
# fd/src/main.mfb (A: stdin)
IMPORT io
IMPORT fs
FUNC main AS Integer
  RES f AS fs::File = fs::open("/tmp/fdsecret.txt", "r")
  LET l AS String = io::readLine()
  io::print("stdin gave [" & l & "]")
  RETURN 0
END FUNC

printf 'SECRET-LINE\n' > /tmp/fdsecret.txt
mfb build fd -q && ./fd/build/fd.out <&-
```

- Observed: `stdin gave [SECRET-LINE]`, exit 0.
- Expected: `io::readLine` fails (closed stdin or EOF), and the file is never read
  through stdin.

```
# (B: stdout)
FUNC main AS Integer
  RES g AS fs::File = fs::open("/tmp/fdout.txt", "w")
  fs::writeAll(g, "file-data;")
  io::print("this went to stdout")
  RETURN 0
END FUNC

./fd/build/fd.out >&- ; cat /tmp/fdout.txt
```

- Observed: `/tmp/fdout.txt` contains `file-data;this went to stdout`, exit 0.
- Expected: the file contains only `file-data;`.

Contrast: with fd 0 closed and no `fs::open` before the read, `io::readLine` fails
with `Error: 7-702-0005 Standard input operation failed.`. That is the correct
closed-stream behavior. The bug needs a later open to take the number.

## Root Cause

`lower_program_entry` (src/codegen/engine/function/entry.rs) goes straight to
arena setup, signal dispositions, seeding and the language entry. Nothing makes
sure fds 0–2 are open. The stdin broadcast reader
(`src/codegen/io/stdin/stdin_broadcast.rs:lower_stdin_next_byte`) and the stdout
drain write to the fixed numbers 0/1/2 no matter what file now has them.

## Goal

- Neither repro mixes user-file data with standard streams. A closed standard
  stream behaves as the owner decides (null device, or still an error) and is
  never aliased to a user file.

### Non-goals (must NOT change)

- The behavior when the std fds are open (terminals, pipes, files).
- The Windows path, where handles are not number-allocated the same way. Verify it
  separately.

## Blast Radius

- POSIX program entry (macOS, Linux glibc/musl): fixed by this bug.
- App-mode entry (`_main` toolkit bootstrap): same hazard, same fix point.
- Worker threads: not affected, because fds are process-wide and startup covers them.

## Resolution

The owner's decision is implemented by one shared emitter,
`emit_reserve_standard_fds` (`src/codegen/engine/function/entry.rs`). For fd 0, 1, 2 in
order it calls `fcntl(fd, F_GETFD)`, sign-extending the C `int`. A negative result means the
descriptor is closed, and `open("/dev/null", O_RDONLY)` (fd 0) or `open("/dev/null", O_WRONLY)`
(fd 1 and 2) then lands on exactly that number, by the lowest-free rule. If `/dev/null` cannot
be opened, the program calls `abort()`, as Rust std does. The doc did not cover this case;
Rust std and Go both terminate there. Call sites:

- the POSIX console entry (`lower_program_entry`), right after the arena address and stack
  limit, before any `--debug` feature's entry work (macOS, Linux glibc and musl, all three
  arches);
- the macOS app `_main` and the GTK `_mfb_gtkapp_main`, as their first act. AppKit, GTK's
  display connection and the window pipe would otherwise take a closed number. The app
  entry, called later on the worker, skips the step.

Windows: unchanged (non-goal). Its standard handles come from `GetStdHandle`, and `CreateFile`
never allocates into a standard-handle slot, so the lowest-free-number hazard does not exist
there. This is reasoning, not a run: 2230 was down. Supporting pieces: the `/dev/null` data
object `_mfb_rt_dev_null_path` for every POSIX module with an entry, and the
`fcntl`/`open`/`abort` imports in both POSIX `entry_imports` and the GTK app import table.

bug-723's test needed no change. `tests/runtime/rt_io_set_buffered_drain_failure.rs` already
makes stdout fail with a read-only descriptor, not `>&-`, and it still passes.
`grep -rn -e '>&-' -e '<&-' tests src/docs scripts tools` finds no other user.

Spec: `mfb spec memory program-startup` has a new "Reserving the Standard Descriptors"
section. `mfb spec app macos-runtime` and `mfb spec app linux-runtime` describe the bootstrap
step. Man: `mfb man io` has a paragraph on starting with a standard stream closed.

### Proof

Probes `fda` (open a secret file, then `io::readLine`) and `fdb` (open a file for writing,
`fs::writeAll "file-data;"`, then `io::print` and `io::printError`). The exploit is
"stdin gave [SECRET-LINE]" (A), or console text appended to the file (B).

| platform (box) | base | fixed |
| --- | --- | --- |
| macOS aarch64 (this Mac) | A: `stdin gave [SECRET-LINE]`. B `>&-`: file `file-data;this went to stdout`. B `2>&-`: `file-data;this went to stderr` | A: `ErrEndOfFile` (7-702-0003), rc 255, nothing leaked. B `>&-`, `2>&-`, `>&- 2>&-`: file `file-data;`, rc 0 |
| Linux aarch64 glibc (2226) | same exploit on all four cases (`>&- 2>&-` rc 255) | same as macOS, all four cases clean |
| Linux riscv64 musl (2229) | same exploit | clean |
| Linux riscv64 glibc (2229 + Debian glibc 2.41 loader) | same exploit | clean |
| macOS `--app`, None mode (headless and GUI) | `>&-`: file `file-data;this went to stdout`. `2>&-`: `...stderr` | file `file-data;` |
| GTK `--app` aarch64 glibc (2226, Xvfb, extracted AppImage binary) | `>&-`: fd 1 is a GTK-internal object (`fs::canonicalPath("/proc/self/fd/1")` unresolvable), the print goes into it, rc 255. Same for `2>&-` | fd 1 and fd 2 are `/dev/null`, rc 0, file `file-data;` |

The open-fd case is unchanged everywhere: both lines reach the terminal and the file holds
`file-data;`.

The abort path is not dead code. On 2229 an `LD_PRELOAD` shim made `open("/dev/null")` fail
with ENOENT. With fd 2 closed the fixed binary exited 134 (SIGABRT) before writing anything.
With every fd open it ran normally (rc 0, file written).

Debugger (`mfb dbg` on a `--debug -O0` macOS build, `bp fcntl`, `run`, `bt 2`, `list reg x0
x1`, `finish`): the first stop is in `_main` with `x0 = 0`, `x1 = 1`
(`fcntl(0, F_GETFD)`), and it returns 0 (open). `mfb dbg` cannot launch the debuggee with
a closed standard stream: run with `2>&-`, the debuggee still got an fd 2, so the reservation
branch could not be stepped in the debugger.

Coordinator runs, 2026-10-06 (main c6dd68acc base vs bug-staging-16), same two programs as
`tests/runtime/rt_closed_std_fds_not_reused.rs`, cross-built with `-target linux-x86_64` /
`-target linux-aarch64` (musl binaries):

| box | base: stdin closed | base: fd 1 / fd 2 closed | fixed: stdin closed | fixed: fd 1 / fd 2 closed |
|---|---|---|---|---|
| 2227 Alpine x86_64 musl | `stdin gave [SECRET-LINE]` | file `file-data;this went to stdout` / `…stderr` | rc 255 `ErrEndOfFile` | file `file-data;` / `file-data;` |
| 2224 Alpine aarch64 musl | `stdin gave [SECRET-LINE]` | file `file-data;this went to stdout` / `…stderr` | rc 255 `ErrEndOfFile` | file `file-data;` / `file-data;` |

Windows: unchanged by design. Across every windows-x86_64 fixture in the batch-16 artifact-gate
attribution, `_main` is byte-identical to main's. Linux x86-64 glibc (2228) was not run: the box is
down and is not being brought up for now.

## Golden impact

Every POSIX program with an entry changes: the entry, or the app `_main`, gains the
three-descriptor block, plus three imports and the 10-byte `_mfb_rt_dev_null_path` data
object. Measured with `scripts/artifact-gate.sh <mfb> all`: 187 diffs, against 62 before this
bug. That is 125 new diffs in 43 fixtures, all on macos-aarch64, linux-aarch64, linux-x86_64
and linux-riscv64 (`.ncode`/`.ncodesum`/`.nplan`/`.nobj`). The windows-x86_64 diff set is
identical before and after. One fixture was checked
(`syntax/project/project-entry-nothing-main`, linux-x86_64 `.ncode`, branch base vs fix): the
diff is exactly the `fcntl`/`open`/`abort` block, its relocations, the imports and the data
object.
