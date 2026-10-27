### 1. `openFileNoFollow` refuses a symlink at any component, not only the final one
UNIT:      man-page:fs/overview
CLAIM:     "Where a path names a symlink, the final component is followed (so reads and writes act on the target) except in fs::openFileNoFollow, which refuses a symlinked final component, and fs::deleteFile, which removes the link itself."
VERDICT:   wrong
EVIDENCE:  Probe `p4.mfb` (dir `sym/` with `dlink -> real` and `link.txt -> real/target.txt`): `fs::openFileNoFollow(dir & "/dlink/target.txt")` printed `noFollow intermediate symlink raised 77030003 Filesystem access was denied.` and the final-component case printed the same. Source: `src/codegen/builtins/fs/func_open_file_no_follow.rs` DESC says "refuses to traverse a symbolic link at *any* component of `path`, not just the terminal name" (`O_NOFOLLOW_ANY` on macOS, `openat2`+`RESOLVE_NO_SYMLINKS` on Linux; final-only is just the pre-5.6 Linux fallback). The page's own Functions table row for `fs::openFileNoFollow` says "at any path component", so the overview contradicts itself on one screen. The rest of the sentence held: `readText` via `link.txt` printed `orig`, a `writeText` through the link changed `real/target.txt`, and `deleteFile(link.txt)` left `target exists=TRUE`.
SUGGESTED: "…except in fs::openFileNoFollow, which refuses to open through a symlink at any component of the path (on an old Linux kernel, only a symlinked final component), and fs::deleteFile, which removes the link itself."

### 2. The existence predicates do raise on a NUL byte
UNIT:      man-page:fs/overview
CLAIM:     "The existence predicates fs::exists, fs::fileExists, and fs::directoryExists return a Boolean and report a missing or unreadable path as FALSE rather than raising; only an internal running out of memory can raise from them."
VERDICT:   wrong
EVIDENCE:  Probe `p1.mfb`: `fs::exists("a" & "\0" & "b")` printed `exists NUL raised 77050002 Argument value is not valid for the requested operation.` and `fs::directoryExists` did the same. `src/codegen/builtins/fs/func_exists.rs` DESC: "raises only `ErrInvalidArgument`, when `path` contains a NUL byte, and `ErrOutOfMemory`"; its `errors: vec!["ErrOutOfMemory", "ErrInvalidArgument"]`.
SUGGESTED: "…report a missing or unreadable path as FALSE rather than raising; they raise only ErrInvalidArgument for a path containing a NUL byte, or ErrOutOfMemory."

### 3. "Every path argument must be non-empty" is false for the existence predicates
UNIT:      man-page:fs/overview
CLAIM:     "Every path argument must be non-empty and free of embedded NUL bytes, because the host call requires a NUL-terminated path."
VERDICT:   misleading
EVIDENCE:  Probe `p1.mfb`: `fs::exists("")`, `fs::fileExists("")`, `fs::directoryExists("")` each printed `FALSE` with no error, while `fs::readText("")` printed `readText empty raised 77050002`. `func_exists.rs` DESC: "An empty `path` is not an error: nothing exists there, so the result is `FALSE`." The sentence also never says which error an empty or NUL path raises everywhere else (`ErrInvalidArgument`, per every other fs descriptor, e.g. `func_read_text.rs`, `func_delete_file.rs`, `func_list_directory.rs`).
SUGGESTED: "A path argument must be non-empty and free of embedded NUL bytes; otherwise the call raises ErrInvalidArgument. The three existence predicates are the exception for an empty path: they report FALSE."

### 4. "A use the compiler can see" asks the reader to model the compiler
UNIT:      man-page:fs/overview
CLAIM:     "A File that has been closed — explicitly or when its binding ends — cannot be used again, not even by a second fs::close: a use the compiler can see is rejected at compile time, and one it cannot see raises ErrResourceClosed."
VERDICT:   out-of-scope
EVIDENCE:  The developer-visible rule is concrete and the page does not give it: probe `p3.mfb` (v1) — naming `f` after `fs::close(f)` in the same function is build error `TYPE_USE_AFTER_MOVE` ("Binding `f` was moved and cannot be used again"); probe `p3.mfb` (v2) — closing through a record copy inside another function, then `fs::close(f)` in main, printed `second close raised 77030004 Resource handle is already closed.` `func_close.rs` DESC states it in those terms ("In the function that closes it… build error `TYPE_USE_AFTER_MOVE`. A closed `File` can still be reached another way — through a second copy of a record that holds it, after a function it was passed to closed it — and there… raises `ErrResourceClosed`"). "What the compiler can see" is the §3 compiler mental model; the reader cannot predict it from the page.
SUGGESTED: "…cannot be used again, not even by a second fs::close. Naming the binding again in the function that closed it is a build error; reaching the closed File another way — through a copy of a record that holds it, or after a function it was passed to closed it — raises ErrResourceClosed at run time."

### 5. "Portable open modes" implies non-portable ones exist
UNIT:      man-page:fs/overview
CLAIM:     "Portable open modes are "read"/"r", "write"/"w", "readWrite"/"rw", and "append"/"a"."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/fs/func_open.rs` DESC: "The mode string is matched exactly, byte for byte, and is case sensitive; any other value is rejected before the file is touched" (raises `ErrInvalidArgument`). There is no second, host-specific set.
SUGGESTED: "The open modes are "read"/"r", "write"/"w", "readWrite"/"rw", and "append"/"a"; any other string raises ErrInvalidArgument."

### 6. Handle-function roll call omits `fs::size` and `fs::readBytesAt`
UNIT:      man-page:fs/overview
CLAIM:     "fs::readLine, fs::readAll, fs::readAllBytes, fs::writeAll, fs::writeAllBytes, fs::eof, and fs::seek act on any of them."
VERDICT:   incomplete
EVIDENCE:  The page's own Functions table lists `fs::size` ("Report the length in bytes of an open File") and `fs::readBytesAt` ("Read bytes from a chosen position in an open File"), both File-handle operations (`src/codegen/builtins/fs/func_size.rs`, `func_read_bytes_at.rs`). Probe `p2.mfb` showed `fs::size` works on an `openData` handle too (the `openData` example prints `3` for `[0,255,10]`). The sentence that enumerates "the handle functions" leaves two out.
SUGGESTED: "fs::readLine, fs::readAll, fs::readAllBytes, fs::readBytesAt, fs::writeAll, fs::writeAllBytes, fs::eof, fs::seek, and fs::size act on any of them."

Everything else on the page checked out against the binary. Verified by probe: `IMPORT fs` builds in a bare `mfb init` project with no manifest dependency (all probes); `readText` on bytes `[104,255,105]` raised `77020004 ErrEncoding` (`p2`); `listDirectory` returned `a, b, bad.bin, z, é` in byte order with no `.`/`..` (`p2`); `isBuffered` is `FALSE` on a fresh handle (`p2`); `canonicalPath` and `isWithin` resolve through the `dlink` symlink while `pathNormalize("/no/such/dir/../x/./y")` returned `/no/such/x/y` without touching disk and `canonicalPath("/no/such/dir")` raised `77030001` (`p4`); `fs::writeAll` on a File handed to a worker with `thread::transfer` raised `77030009 ErrResourceMoved` (`proj2/src/main.mfb` with a scratch `workers` companion package). Memory-vocabulary sweep of the rendered page: the only banned word is "Allocation failed." in the derived Errors row, which is carve-out 2.
