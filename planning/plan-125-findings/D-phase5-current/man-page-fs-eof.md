### 1. "at or beyond the last byte" is off by one
UNIT:      man-page:fs/eof
CLAIM:     "It returns `TRUE` when the position is at or beyond the last byte and `FALSE` while one or more bytes remain to be read."
VERDICT:   wrong
EVIDENCE:  `position.rs:lower_fs_eof_helper` compares the saved position against the length (`compare_registers(&start,&end)` / `branch_ge`), so TRUE needs position ≥ length. Probe `edges.mfb` on a 5-byte file "abcde": `seek 4 (last byte): FALSE`, `seek 5 (length): TRUE`, `seek 50 (past end): TRUE`. A position *at* the last byte still has one byte to read and reports FALSE.
SUGGESTED: "It returns `TRUE` when the position is at or beyond the end of the contents (at or past the file's length in bytes) and `FALSE` while one or more bytes remain to be read."

### 2. Not only "a regular file on disk": in-memory Files work too
UNIT:      man-page:fs/eof
CLAIM:     "Because determining the length requires seeking, `fs::eof` only works on a seekable handle — a regular file on disk."
VERDICT:   misleading
EVIDENCE:  Probe `edges.mfb`: `RES m = fs::openData("ab\n", "r")` → `openData start: FALSE`, after one `readLine` → `openData after line: TRUE`. `mfb man fs` overview explicitly lists `fs::eof` among the calls that act on an `fs::openData` File. The page's own openData example on `mfb man fs openData` loops on `fs::eof`.
SUGGESTED: "`fs::eof` needs a handle whose position and length can be measured: a regular file on disk, or a File from `fs::openData`. On a pipe, a socket, or another non-seekable handle …"

### 3. "the other reading functions" do not all raise at end of input
UNIT:      man-page:fs/eof
CLAIM:     "This is the intended pattern because end of input is reported by those functions as an error rather than as an empty result: testing `fs::eof` first lets a loop stop cleanly at the end of the file."
VERDICT:   wrong
EVIDENCE:  Probe `edges.mfb` at end of a file: `readAll at eof: []` with no error, then `readLine at eof raised: Read operation reached end of file where a value was required.` `mfb man fs readAll` Errors table has no `ErrEndOfFile` row and its Description says "When file is already at end of input, no bytes remain and the empty String is returned." Only `fs::readLine` (and the byte-reading siblings that list `ErrEndOfFile`) treat end of input as an error.
SUGGESTED: "Guard a `fs::readLine` loop with `fs::eof`, because `fs::readLine` reports end of input as `ErrEndOfFile` rather than returning an empty String; testing `fs::eof` first lets the loop stop cleanly. `fs::readAll` and `fs::readAllBytes` do not need the guard: at end of input they return an empty value."

### 4. The buffer-and-seek mechanics paragraph is compiler-facing
UNIT:      man-page:fs/eof
CLAIM:     "The test is buffer-aware: if the transparent per-handle read buffer still holds unconsumed bytes (its read cursor is before its fill mark), `fs::eof` returns `FALSE` immediately without querying the host. Otherwise it asks the host for the file's current position and total length and compares them — the position is captured, the handle is seeked to end to read the length, then seeked back to the captured position, so the read position is left exactly where it was."
VERDICT:   out-of-scope
EVIDENCE:  Every clause describes `position.rs:lower_fs_eof_helper` (`FILE_OFFSET_READ_POS`/`READ_FILL` compare, the three `emit_seek_file` calls). "read cursor", "fill mark", "querying the host", "seeked to end" are implementation steps a developer cannot observe or act on (§3 "codegen mechanics"); "unconsumed" is the banned `consume` family in a prefix form the census grep cannot see (§4.2). The developer-visible facts are: read-ahead by `fs::readLine` does not make `fs::eof` report TRUE early, and the position is unchanged afterwards. Probe `edges.mfb`: after `readLine` returned "abcde" (no trailing newline), `after readLine: TRUE`; the example's loop stops exactly after the last line.
SUGGESTED: "Text that `fs::readLine` has read ahead but not yet returned still counts as remaining input, so `fs::eof` never reports `TRUE` while a line is still to come. The call reads no contents and has no side effects: the read position is left exactly where it was, nothing is written, and `file` stays open."

### 5. Append-mode sharp edge omitted
UNIT:      man-page:fs/eof
CLAIM:     "`fs::eof` reports whether `file`'s current read position has reached the end of its contents."
VERDICT:   incomplete
EVIDENCE:  Probe `edges.mfb`: on an existing 3-byte file reopened with `fs::open(data, "a")`, `append mode fresh: FALSE`. The position of a freshly opened append-mode File is the start, so `fs::eof` is FALSE even though every write will go to the end (`mfb man fs seek`: "On a File opened in "append" mode every write still goes to the end, wherever the position is"). On "w" mode: `write mode fresh: TRUE`, `write mode after write: TRUE`, `write mode seek 0: FALSE`.
SUGGESTED: Add: "`fs::eof` tests the read position only. A File opened with mode "append" starts at the beginning of its existing contents, so `fs::eof` reports `FALSE` on a non-empty file even though every write still goes to the end. A File opened with "write" is empty and reports `TRUE` until you seek back."

### 6. A closed File is usually a build error, not `ErrResourceClosed`
UNIT:      man-page:fs/eof
CLAIM:     "Calling it on a `File` that has been closed raises `ErrResourceClosed`, and on one handed to another thread with `thread::transfer`, `ErrResourceMoved`."
VERDICT:   misleading
EVIDENCE:  Probe `closed.mfb` (close then eof in the same function) and `closed2.mfb` (close inside an IF, then eof) both fail to build: `error[2-203-0055 TYPE_USE_AFTER_MOVE]: binding is used after move`. Only `closed3.mfb`, where a called function `closer(m)` closes the handle, reaches the runtime path: `closed raised: Resource handle is already closed.` `mfb man fs close` already documents this split; `mfb man fs` overview: "a use the compiler can see is rejected at compile time, and one it cannot see raises ErrResourceClosed". The `ErrResourceMoved` half is UNVERIFIED: I did not run a `thread::transfer` probe (needs the worker package the examples harness builds); the guard exists in `position.rs:closed_or_moved_guard`.
SUGGESTED: "Naming `file` after `fs::close(file)` in the same function is the build error TYPE_USE_AFTER_MOVE. When the close happened where the compiler cannot see it — in a function the handle was passed to, or through a second copy of a record holding it — `fs::eof` raises `ErrResourceClosed`. A handle handed to another thread with `thread::transfer` raises `ErrResourceMoved`."

Example verified: `example.mfb` copied verbatim into an `mfb init` project under /tmp/plan-125-scratch/D-phase5-current/man-page-fs-eof/p_example built cleanly and printed `first line` / `second line`, exit 0. Pipe probe `pipe.mfb` (`echo hi | … ` with `fs::open("/dev/stdin","r")`) printed `pipe raised: Read operation failed.`, confirming the `ErrReadFailed` row. Empty-file probe printed `empty: TRUE`.
