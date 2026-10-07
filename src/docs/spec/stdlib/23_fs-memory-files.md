# In-memory files, seeking and line reading (fs)

The model behind `fs::openData`, `fs::seek`, and the `File` line readers
(`fs::readLine` and the internal `fs::readLineRaw` the `csv` readers use). The
per-function API is owned by `./mfb man fs`; this topic is what a reimplementation
needs beyond it. All three were added or changed by bug-809, which replaced the
`csv` package's value-threaded streaming reader with one that reads a `File`.

## fs::openData: a real File over a copy

`fs::openData(value, mode)` — `value` a `String` or a `List OF Byte` — returns an
ordinary `File` record: the canonical resource header and the same buffer slots
`fs::open` writes, with an OS handle in the descriptor slot. Every `fs::` handle
operation and the `File` close op therefore work on it unchanged; nothing in them
knows the file came from memory. The bytes of `value` are copied into the file
when it is opened, and the `File` never writes back to `value`.
[[src/codegen/builtins/fs/gen_open_data.rs:lower_fs_open_data_helper]]

The two overloads read their bytes from different places — a `String` at `+8`, a
`List OF Byte` past its collection header and (zero-stride) entry table — so they
are two code forms: the `List OF Byte` one lowers as `fs.openDataBytes`, selected
by the value's static type exactly as `tcp::write` selects its byte or text form,
and both share one body that branches on the form.
[[src/codegen/builtins/fs/func_open_data.rs:OPEN_DATA_BYTES]]
[[src/codegen/engine/value/builder_values.rs:net_write_payload_form]]

### Backing per OS

The backing is a host file with no name anyone can open, gone once its last handle
closes — including when the process dies.

| OS | backing |
|---|---|
| Linux | `memfd_create("mfb-data", MFD_CLOEXEC)` through libc `syscall()` (number 319 on x86-64, 279 on AArch64 and RISC-V; glibc and musl both export `syscall`) |
| macOS | a `0600` file created exclusively (`O_CREAT\|O_EXCL`) in the user temp directory (`confstr(_CS_DARWIN_USER_TEMP_DIR)`) as `mfb-data-<uuid v4>.tmp`, and unlinked as soon as the mode handle is open |
| Windows | `CreateFileW(CREATE_NEW, FILE_ATTRIBUTE_TEMPORARY \| FILE_FLAG_DELETE_ON_CLOSE)` in `GetTempPathW`, same name shape |

macOS has no memory-only equivalent that works here: a POSIX `shm_open` descriptor
refuses `read`/`write` (`ENXIO`) and `lseek` (`ESPIPE`) — measured with a C probe on
macOS 15 — so it cannot carry the `fs::` operations.
[[src/codegen/builtins/fs/gen_open_data.rs:memfd_create_number]]
[[src/target/win_x86_64/code.rs:emit_create_file_w]]

### The mode is enforced by a second open

The scratch handle is opened read+write to receive the copy. The handle the `File`
keeps is a SECOND open of the same file, with the mode's own access and no
create/truncate bits — `/proc/self/fd/<n>` on Linux, the temp path elsewhere
(before the macOS unlink; while the Windows scratch handle still holds the file).
The order is: create → reopen → unlink (macOS) → copy through the scratch handle →
close it. On Windows that close leaves the file delete-pending: no new open
succeeds, the kept handle works until it closes, then the file is removed (measured
on box 206, Windows 11 26200).
[[src/codegen/builtins/fs/gen_open_data.rs:reopen_flags]]

So the mode means exactly what it means for `fs::open`, enforced by the host the
same way:

| mode | data | access | starts at |
|---|---|---|---|
| `"r"`/`"read"` | the copy | read only — a write raises `ErrWriteFailed` | 0 |
| `"w"`/`"write"` | empty (`value` is ignored) | write only — a read raises `ErrReadFailed` | 0 |
| `"rw"`/`"readWrite"` | the copy | read and write | 0 |
| `"a"`/`"append"` | the copy | write only; every write goes to the end (`O_APPEND` / `FILE_APPEND_DATA`) | 0 |

Any other mode raises `ErrInvalidArgument` before anything is created. Every host
failure — creating, reopening, unlinking, copying, closing the scratch handle, or
(macOS/Windows) a temp directory longer than 4,096 bytes — raises `ErrWriteFailed`
after closing whatever handles were opened and, on macOS, removing the name. These
failures do not go through the path mapping of `./mfb spec stdlib fs-host-errors`:
there is no caller path to classify.

## fs::seek

`fs::seek(file, position)` sets the descriptor to the absolute byte offset
`position` (`lseek(SEEK_SET)` / `SetFilePointerEx(FILE_BEGIN)`). Before moving it
drains pending buffered output (`_mfb_rt_fs_file_drain`): those bytes were written
for the old position. After moving it empties the read cache (`READ_POS`,
`READ_FILL`, `READ_AT_EOF` to 0): its bytes came from the old position, and an
absolute move needs no rewind. A negative `position` raises `ErrInvalidArgument`
before anything is touched; a failed drain raises `ErrWriteFailed` and does not
move; a host refusal to move (a non-seekable handle) raises `ErrReadFailed`, as
`fs::size` and `fs::eof` do. A position past the end is allowed.
[[src/codegen/builtins/fs/gen_read_write.rs:lower_fs_seek_helper]]

## The line readers

`fs::readLine` and `fs::readLineRaw` are one emitter with a `keep_terminator`
switch, serving lines out of the per-`File` block read buffer (`READ_PTR`,
`FILE_READ_BUFFER_CAPACITY`) and refilling it with one host read when it runs out.
[[src/codegen/builtins/fs/gen_read_write.rs:lower_fs_read_line_helper]]

- A line found whole in the buffer is copied straight from it into the result —
  one allocation. Only a line that spans a refill goes through the growing
  accumulator, created on its first use.
- `fs::readLine` strips the LF and, only when the line ended at an LF, a CR right
  before it. A final line with no LF keeps a trailing CR (before bug-809 it was
  stripped there too). At end of input it raises `ErrEndOfFile`.
- `fs::readLineRaw` (internal; only toolchain source can call it) returns the line
  with its terminator — `"a\n"`, `"a\r\n"`, or an unterminated final `"a"` — and
  returns `""` at end of input instead of raising. A line that is not the end
  always holds at least its LF, so `""` is unambiguous. This is what lets the
  `csv` readers rebuild a quoted field spanning lines byte for byte
  (`./mfb spec stdlib csv`).

Both validate the line as UTF-8 (`ErrEncoding`). Because they share the read
buffer, the two — and `csv::readRow` and `csv::parse(File)`, which use the raw
reader — can be mixed on one `File`; whole-file reads and writes reconcile the
buffer first, as before.

## See Also

* ./mfb man fs — the per-function API (`openData`, `seek`, `readLine`, …)
* ./mfb spec stdlib csv — the CSV readers built on these
* ./mfb spec stdlib fs-host-errors — the path-error mapping the other members use
