All probes complete. Writing up the findings.

### 1. `openFileNoFollow` refuses a symlink at *any* component, not only the final one
UNIT:      man-page:fs/overview
CLAIM:     "Where a path names a symlink, the final component is followed (so reads and writes act on the target) except in fs::openFileNoFollow, which refuses a symlinked final component, and fs::deleteFile, which removes the link itself."
VERDICT:   wrong
EVIDENCE:  Probe p1 (`ln -s sub linkdir`, then `fs::openFileNoFollow("linkdir/t.txt")` where only the *intermediate* directory is a symlink) printed `intermediate symlink refused: Filesystem access was denied. code=77030003`. The function's own page (`src/codegen/builtins/fs/func_open_file_no_follow.rs:DESC`, and the overview's own Functions-table summary "refusing to traverse a symbolic link at any path component") says the same. The overview contradicts its own table two screens down.
SUGGESTED: "... except in fs::openFileNoFollow, which refuses to open through a symlink at any component of the path, and fs::deleteFile, which removes the link itself rather than the target."

### 2. "Every path argument must be non-empty" is false for a third of the package
UNIT:      man-page:fs/overview
CLAIM:     "Every path argument must be non-empty and free of embedded NUL bytes, because the host call requires a NUL-terminated path."
VERDICT:   wrong
EVIDENCE:  Probe p2: `fs::exists("")`, `fs::fileExists("")`, `fs::directoryExists("")` each printed `FALSE` with no error; `fs::pathBaseName("")` returned `[]` and `fs::pathNormalize("")` returned `[.]`, both without error. Only `fs::isWithin("", ...)`, `fs::readText("")` and `fs::open("", "read")` (probe p9) raised ErrInvalidArgument (77050002). The exists descriptors say so explicitly: "An empty `path` is not an error" (`func_exists.rs:42`, `func_file_exists.rs:47`, `func_directory_exists.rs:47`). The stated reason ("the host call requires a NUL-terminated path") also does not apply to the path-syntax functions, which the very next sentence says never touch the filesystem.
SUGGESTED: "No path may contain an embedded NUL byte. Functions that hand a path to the host also reject an empty path with ErrInvalidArgument; the existence predicates report an empty path as FALSE, and the path-syntax functions accept it as ordinary text (fs::pathNormalize("") is ".")."

### 3. Existence predicates also raise ErrInvalidArgument
UNIT:      man-page:fs/overview
CLAIM:     "The existence predicates fs::exists, fs::fileExists, and fs::directoryExists return a Boolean and report a missing or unreadable path as FALSE rather than raising; only an internal running out of memory can raise from them."
VERDICT:   wrong
EVIDENCE:  Probe p11 with `LET nul AS String = "a\0b"`: `fs::exists(nul)` printed `exists NUL: Argument value is not valid for the requested operation. code=77050002`; `fs::directoryExists(nul)` the same. Registry: `errors: vec!["ErrOutOfMemory", "ErrInvalidArgument"]` on all three (`func_exists.rs:90`, `func_file_exists.rs:95`, `func_directory_exists.rs:95`).
SUGGESTED: "... report a missing or unreadable path as FALSE rather than raising; they raise only ErrInvalidArgument, for a path containing a NUL byte, and ErrOutOfMemory."

### 4. Handle-function roster omits `fs::size` and `fs::readBytesAt`
UNIT:      man-page:fs/overview
CLAIM:     "fs::readLine, fs::readAll, fs::readAllBytes, fs::writeAll, fs::writeAllBytes, and fs::eof act on one."
VERDICT:   incomplete
EVIDENCE:  The Functions table on the same page lists `fs::size` ("Report the length in bytes of an open File") and `fs::readBytesAt` ("Read bytes from a chosen position in an open File"); both take a `File` (`func_size.rs:INTRO`, `func_read_bytes_at.rs:INTRO`). The Description's "act on one" list is presented as the complete set of handle operations and leaves out the only two positional/metadata ones. `fs::flush`, `fs::setBuffered`, `fs::isBuffered` are covered in the next paragraph, so those are fine.
SUGGESTED: "fs::readLine, fs::readAll, fs::readAllBytes, fs::readBytesAt, fs::writeAll, fs::writeAllBytes, fs::size, and fs::eof act on one."

### 5. ErrResourceMoved is in the Errors table but has no condition in the prose
UNIT:      man-page:fs/overview
CLAIM:     "A File may be handed to another thread with thread::transfer."
VERDICT:   incomplete
EVIDENCE:  The derived Errors table lists 77030009 ErrResourceMoved; `func_close.rs` errors include `"ErrResourceMoved"`, and its parameter desc reads "Must not already be closed or moved." The description names transfer but never says what the sender sees afterward, so the only explanation of that row is the table's message column. `.ai/man-content.md` §2 requires every raisable error and its condition in prose. Also relevant but unstated: `thread/func_transfer.rs:178` notes a moved handle "starts unbuffered", so a `setBuffered(TRUE)` does not survive the transfer. UNVERIFIED by probe (a thread entry point needs a companion package); taken from the descriptor source.
SUGGESTED: "A File may be handed to another thread with thread::transfer; afterwards the sender's binding is unusable and any use of it raises ErrResourceMoved. The receiving thread's handle starts with buffering off."

### 6. "Portable open modes" implies non-portable ones exist; none do
UNIT:      man-page:fs/overview
CLAIM:     "Portable open modes are "read"/"r", "write"/"w", "readWrite"/"rw", and "append"/"a"."
VERDICT:   misleading
EVIDENCE:  `gen_open.rs:245-251` and `:770-776` match exactly `r`, `read`, `w`, `write`, `rw`, `readWrite`, `a`, `append` and nothing else; probe p7 `fs::open("sub/t.txt", "READ")` raised ErrInvalidArgument (77050002). "Portable" invites the reader to look for a platform-specific extra mode that does not exist, and the sentence never says the match is exact and case-sensitive, which is the sharp edge a developer hits.
SUGGESTED: "The open modes are exactly "read"/"r", "write"/"w", "readWrite"/"rw", and "append"/"a", matched case-sensitively; any other string raises ErrInvalidArgument before the file is touched."

Everything else on the page checked out against the binary. Probes and their output, all under /tmp/plan-125-scratch/D-phase5/man-page-fs-overview/work: p3 (close hidden inside a conditional in another SUB, then close again) printed `Resource handle is already closed. code=77030004`, and p3b (visible second use) was rejected at compile time with TYPE_USE_AFTER_MOVE, matching the "compiler can see / cannot see" sentence. p4/p10 showed `fs::deleteFile` on a file symlink and on a directory symlink removes the link and leaves the target. p5 `listDirectory` printed `.hidden, a, b, z` in byte order with `.`/`..` excluded. p6 `readText`/`readAll` on bytes `ab\xff\xfecd` raised ErrEncoding (77020004). p8/p8b showed buffering off by default, buffered bytes invisible before `flush`, visible after, and the trailing unflushed write landed when the binding went out of scope. `scripts/man-census.sh --memory-scope fs` and `--scope fs` (with MFB pointed at the P-125 binary) both report 0 hits, and I found no sentence teaching an ownership model without a banned word. The page has no Examples section, so there was nothing to compile from it.
