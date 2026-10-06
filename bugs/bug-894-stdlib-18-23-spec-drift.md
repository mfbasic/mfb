# bug-894: stdlib spec 20–23 imprecisions (io ESC-in-data claim, openData write-mode reads, openData man vs spec backing, stale compress sizes)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (documentation)

Status: Open
Regression Test: none (doc-only); verify by `mfb spec stdlib <topic>` rendering and the probes below

Four places where the spec (or the man page it defers to) says something the measured behavior does
not match. Each item is classed separately. None of them is a crash or a wrong value in a common
path.

References:

- `src/docs/spec/stdlib/20_compress.md`, `22_io-input.md`, `23_fs-memory-files.md`
- `./mfb man fs openData`
- Found during spec-hardening batch B08.

## Failing Reproduction

### 1. SPEC — 22_io-input.md: "an escape byte in data is text" is false for the keystroke readers

Claim (§"The read engine", input-delay bullet): "On **data** it waits for as long as the call allows:
the rest of the sequence is on its way, and an escape byte in data is text."

The keystroke readers on a pipe decode ESC sequences as keys. They do not treat ESC as text
(`/tmp/hardening/B08/io1`, `MODE` selects the reader):

```
printf '\x1b[Ax' | MODE=scalar ./build/io1.out     # S len=1 crc=2363233923  ("x" only; ESC [ A consumed as a key)
printf '\x1bx\x1b' | MODE=key ./build/io1.out      # K Unicode … a=TRUE (Alt+x), K Escape …
(printf '\x1b'; sleep 0.5; printf '[A') | MODE=key ./build/io1.out   # K Up …
```

The first half of the sentence is correct: the third probe shows the reader waiting 500 ms on data
for the rest of the sequence. "An escape byte is just a byte" holds only for the **line** readers
(§"The readers", Data bullet; `printf '\x1b[A\n' | MODE=line` → `L len=3`). Fix: delete the clause
from the input-delay bullet, or limit it to the line readers.

### 2. UNCLEAR — 23_fs-memory-files.md mode table: `"w"` "a read raises `ErrReadFailed`" depends on the reader

`/tmp/hardening/B08/md2` (each line a separate `fs::openData("", "w")`):

```
readAll=[]            <- no error
readAllBytes len=0    <- no error
2 ERR 77020001        <- readLine raises ErrReadFailed
readBytesAt len=0     <- no error
eof=TRUE
```

The same is true of a disk file from `fs::open(path, "w")` (`disk w readAll=[]`). Once the write-only
file has content, `readAll` does raise 77020001 (`tWrite("w")` in `/tmp/hardening/B08/md`). The
size-measuring readers skip the host `read` when the remainder is 0, so they never see the
access refusal. Either the table should say "a read of a non-empty file raises", or the readers
should check access (owner decision).

**Owner decision (2026-10-05): always raise (CODE change).** Every `fs` reader checks the
handle's access before the size shortcut. Any read on a write-only (`"w"`) file, disk or
`openData`, is `ErrReadFailed`, whether the file is empty or not. The mode table keeps saying
"a read raises `ErrReadFailed`".

### 3. SPEC/man conflict — `fs::openData` backing

`./mfb man fs openData`: "The data lives in memory, not in any directory: no other program can see it,
nothing is left behind when the File is closed, and nothing needs cleaning up after a crash."

`23_fs-memory-files.md` §"Backing per OS", and the code (`gen_open_data.rs:lower_fs_open_data_helper`,
`"/mfb-data-"` path build, mode `384` = 0600), say that on macOS the backing is a named 0600 file in
the user temp dir, unlinked only after the reopen. On Windows it is a named `CREATE_NEW` file in
`GetTempPathW` that stays visible until the scratch handle closes. So on those hosts the data is on
disk, has a name for a window, and a crash inside that window leaves a file. The man sentence holds
only for Linux `memfd`. (Measured: `ls $TMPDIR | grep -c mfb-data` is 0 before and after a normal run
of `/tmp/hardening/B08/md`. The crash window was not provoked.)

### 4. SPEC (stale, dated) — 20_compress.md size table

Measured now (`/tmp/hardening/B08/sz{a,b,c}`, same-length project names, macos-aarch64):

| Program | spec (2026-09-14) | now |
|---|---|---|
| `IMPORT io` + one print | 66,600 B | 66,724 B |
| + `IMPORT compress`, no call | 66,604 B | 66,724 B |
| + one `crc32` | 83,116 B | 83,236 B |

The relative claims still hold exactly: importing without a call adds 0 B, and one `crc32` adds
16,512 B, one quantum block. The absolute figures are stale. Either refresh them or present the
table as deltas only.

## Root Cause

Documentation drift. For item 2, the code path is
`src/codegen/builtins/fs/gen_read_write.rs:lower_fs_read_all_helper` /
`lower_fs_read_all_bytes_helper`, where a zero-length remainder never reaches a host read.

## Goal

- Items 1, 3 and 4 corrected in the spec/man prose. Item 2 resolved by an owner decision, then
  either the prose or the code changes.

### Non-goals (must NOT change)

- The keystroke readers' decoding of ESC sequences on data (they are correct; the prose is wrong).
