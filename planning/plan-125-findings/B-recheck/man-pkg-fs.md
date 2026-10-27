### 1. Overview's blanket "non-empty path" rule contradicts the five path-syntax pages
UNIT:      man-pkg:fs
PAGE:      overview (package description)
CATEGORY:  overview-mismatch
CLAIM:     "A path argument must be non-empty and contain no NUL character; otherwise the call raises ErrInvalidArgument. The three existence predicates are the exception for an empty path: they report FALSE."
VERDICT:   wrong
EVIDENCE:  Probe `/tmp/plan-125-scratch/B-recheck/man-pkg-fs/p1` (built with `mfb build -q p1`, run `./p1/build/p1.out`) printed `[]`, `[.]`, `[.]`, `[]`, `[]` for `fs::pathBaseName("")`, `fs::pathDirName("")`, `fs::pathNormalize("")`, `fs::pathExtension("")`, `fs::pathJoin([])` with no error. Each of those five pages also says "May be empty" in its parameter table, and `fs::pathJoin` says embedded NUL bytes "are copied verbatim". The predicates are not "the exception": eight functions accept an empty path, and one accepts NUL.
SUGGESTED: "A path that names something on disk must be non-empty and contain no NUL character; otherwise the call raises ErrInvalidArgument. Two groups are exempt: the three existence predicates report FALSE for an empty path, and the five path-syntax functions accept any String, including an empty one."

### 2. Atomic writers replace a symlink instead of writing through it, and reset the file mode; neither the overview nor the atomic pages say so
UNIT:      man-pkg:fs
PAGE:      overview; fs::writeTextAtomic; fs::writeBytesAtomic
CATEGORY:  overview-mismatch
CLAIM:     Overview: "Symbolic links in a path are followed (so reads and writes act on the target) except in fs::openFileNoFollow and fs::openWithin, ... and fs::deleteFile, which removes the link itself." and "(the atomic-write pages give the caveats)". Neither atomic page mentions symlinks or an existing file's permissions.
VERDICT:   missing
EVIDENCE:  Probe `p5` (first run): with `link -> target.txt` (target holds "old"), `fs::writeTextAtomic(link, ...)` left `target.txt` at "old" and `ls -l` showed `link` became a regular file (`-rw-------`), the symlink gone. `fs::writeText(link, ...)` on the same layout wrote through to the target. Second run: `target.txt` chmod 644, then `fs::writeTextAtomic(target.txt)` left it `-rw-------`, while `fs::writeText` on a 644 file left it `-rw-r--r--`. Both effects follow from the documented rename-over-path design but are stated nowhere. `writeTextAtomic`'s ErrInvalidPath list omitting the symlink-loop case is consistent with this (probe `p3`: a loop symlink is replaced, "ok").
SUGGESTED: On both atomic pages, add: "Because the new file is renamed over path, a symbolic link at path is replaced by a regular file rather than followed — the link's target is untouched — and an existing file's permissions are not kept: the result always has mode 0600 (before the umask). Use fs::writeText / fs::writeBytes to write through a link or keep an existing mode." In the overview's symlink sentence, add the atomic writers to the exceptions: "... and fs::deleteFile, fs::writeTextAtomic and fs::writeBytesAtomic, which act on the link itself."

### 3. "Seekable handle — a regular file on disk" on size/seek/readBytesAt excludes openData, which eof and openData include
UNIT:      man-pkg:fs
PAGE:      fs::size; fs::seek; fs::readBytesAt (versus fs::eof and fs::openData)
CATEGORY:  consistency
CLAIM:     fs::size: "fs::size only works on a seekable handle — a regular file on disk." fs::readBytesAt: "this only works on a seekable handle — a regular file on disk." fs::seek: "Moving needs a handle the host can reposition". fs::eof instead says "a regular file on disk, or a File from fs::openData", and fs::openData lists size, seek, readBytesAt as working.
VERDICT:   inconsistent
EVIDENCE:  Probe `p2`: on `fs::openData("hello world", "rw")`, `fs::size` printed 11, `fs::seek(f, 6)` then `fs::readAll` printed "world", `fs::readBytesAt(f, 0, 5)` printed "hello", `fs::eof` printed TRUE. All four work on an in-memory File; the size and readBytesAt pages tell a reader they do not.
SUGGESTED: On fs::size, fs::seek and fs::readBytesAt, use eof's phrasing: "a regular file on disk, or a File from fs::openData".

### 4. fs::open claims to behave identically to fs::openFile but omits the readWrite overwrite sharp edge the other three open pages carry
UNIT:      man-pkg:fs
PAGE:      fs::open (versus fs::openFile, fs::openFileNoFollow, fs::openWithin)
CATEGORY:  consistency
CLAIM:     fs::open: "\"readWrite\" opens the file for both reading and writing, creating it when it does not exist but preserving existing contents." fs::openFile adds: "Reading and writing both start at the beginning of the file, so the first write overwrites what is there byte for byte rather than adding to it; use \"append\" to add at the end, or fs::seek to choose a position." fs::openFile also says the two "behave identically" apart from the default mode.
VERDICT:   inconsistent
EVIDENCE:  Probe `p4`: `fs::open(p, "readWrite")` on a file holding "hello world", one `fs::writeAll(f, "J")`, close, `fs::readText` printed "Jello world". The overwrite is real and is the thing a reader of "preserving existing contents" would not expect; three sibling pages warn, the one named `open` does not.
SUGGESTED: Add the same sentence to fs::open's readWrite description: "Reading and writing both start at the beginning of the file, so the first write overwrites what is there byte for byte rather than adding to it; use \"append\" to add at the end, or fs::seek to choose a position."

### 5. Symlink-loop ErrInvalidPath case listed on the append pages but dropped from writeText/writeBytes
UNIT:      man-pkg:fs
PAGE:      fs::writeText; fs::writeBytes (versus fs::appendText, fs::appendBytes, fs::readText)
CATEGORY:  consistency
CLAIM:     fs::writeText / fs::writeBytes: "ErrInvalidPath if path cannot name a file — it names a directory, a name in it is too long, or a file is used as a directory." The append pages and fs::readText add "or a symbolic link in it loops back on itself".
VERDICT:   inconsistent
EVIDENCE:  Probe `p3` with `loop -> loop` under the temp directory: `writeText: ErrInvalidPath`, `writeBytes: ErrInvalidPath`, `appendText: ErrInvalidPath`. The behaviour is the same across the four path-writers; only two pages document the loop cause.
SUGGESTED: Add ", or it is a loop of symbolic links" to the ErrInvalidPath clause on fs::writeText and fs::writeBytes (the atomic pages are correctly silent: see finding 2).

### 6. fs::tempDirectory documents macOS and Linux sources only; Windows is implemented but unmentioned
UNIT:      man-pkg:fs
PAGE:      fs::tempDirectory
CATEGORY:  coverage
CLAIM:     "The source of the path is platform specific: • On macOS ... • On Linux ..." — no Windows entry, while sibling pages (createDirectory, createDirectories, createTempFile, openFileNoFollow) all state Windows behaviour.
VERDICT:   missing
EVIDENCE:  `grep -n "fn emit_temp_directory" src/target/win_x86_64/code.rs` → line 3316; its body (`sed -n 3316,3360p`) calls `GetTempPathW` and notes the result carries a trailing `\`. Whether that trailing separator is stripped before the String is returned I could not run (no Windows host): UNVERIFIED — the page's "no trailing path separator is added" needs a Windows probe.
SUGGESTED: Add a third bullet: "On Windows the user's temporary directory as the host reports it (the TMP/TEMP environment setting)." and have the main thread confirm on a Windows host whether the trailing backslash is removed; if it is not, say so, since `fs::pathJoin` would then not duplicate it but `fs::pathBaseName` would behave differently.

### 7. The unit never says there is no rename, move, copy, or metadata query
UNIT:      man-pkg:fs
PAGE:      overview
CATEGORY:  coverage
CLAIM:     "The fs package provides filesystem access: one-shot whole-file reads and writes, an open File handle for streaming I/O, purely syntactic path-string manipulation, directory creation and listing, and existence tests."
VERDICT:   missing
EVIDENCE:  `grep -rn 'name: "rename\|name: "move\|name: "copy' src/codegen/builtins/` finds only `term::moveTo`; `ls src/codegen/builtins/fs/` lists 46 `func_*.rs`, none for rename, copy, modification time, or size-by-path. A developer arriving with "how do I rename or move a file?" or "how big is this file / when was it modified?" reads 45 pages and never learns the answer is "not in fs". The writeBytes example titled "Report the size of a file in bytes" reads the whole file to do it, which hides the gap rather than naming it.
SUGGESTED: One sentence at the end of the overview's first paragraph: "fs has no rename, move, copy, or file-metadata call: to move a file, write it to the new path (fs::writeBytesAtomic for an all-or-nothing result) and fs::deleteFile the old one; to learn a file's length, open it and call fs::size."

### 8. One-way cross-references between the path-based and handle-based twins
UNIT:      man-pkg:fs
PAGE:      fs::readText; fs::writeText; fs::writeBytes; fs::listDirectory
CATEGORY:  discoverability
CLAIM:     fs::readAll's See also lists fs::readText, but fs::readText's See also is only fs::readBytes. fs::writeAll / fs::writeAllBytes link fs::writeText / fs::writeBytes, but neither whole-file writer links back. fs::listDirectory has no See also at all.
VERDICT:   missing
EVIDENCE:  Rendered `mfb man fs --all`: READTEXT "See also • fs::readBytes"; WRITETEXT "• fs::appendText • fs::writeTextAtomic"; WRITEBYTES "• fs::appendBytes • fs::writeBytesAtomic"; LISTDIRECTORY ends after its examples with no See also section. Since See also is derived from names in the description, the fix is a sentence, not a table edit. A developer who starts from fs::readText (the natural first page) is the one who needs the handle twin for a large file, and that page does not name it.
SUGGESTED: fs::readText: "To read line by line, or only part of a large file, open it and use fs::readLine, fs::readAll or fs::readBytesAt instead." fs::writeText / fs::writeBytes: "To write incrementally through an open handle, use fs::writeAll / fs::writeAllBytes." fs::listDirectory: name fs::directoryExists (guard) and fs::pathJoin (to build the full path from each name, as its own example does) in the description.
