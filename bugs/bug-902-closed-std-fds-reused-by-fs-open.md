# bug-902: a program started with fd 0/1/2 closed hands those numbers to `fs::open`, so `io::readLine` reads the program's own file and `io::print` writes into it

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Security

Status: Open
Regression Test: tests/rt-behavior/os/closed-std-fds-not-reused-rt (proposed)

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
