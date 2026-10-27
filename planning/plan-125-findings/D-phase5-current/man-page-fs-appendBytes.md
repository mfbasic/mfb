### 1. "data region" is layout vocabulary on a parameter description
UNIT:      man-page:fs/appendBytes
CLAIM:     "The bytes to append, in order, taken verbatim from the list's data region after the file's existing contents."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/fs/func_append_bytes.rs:Parameter bytes.desc`. "Data region" is the collection layout term the lowering uses (`gen_atomic_write/mod.rs:lower_fs_write_path_helper`, `COLLECTION_HEADER_SIZE` / "the data region starts past the header"). A developer has no "data region"; a `List OF Byte` is just its elements. The sentence is also garbled: "taken … from the list's data region after the file's existing contents" reads as if the list has the file's contents in it.
SUGGESTED: "The bytes to append, written in list order after the file's existing contents. An empty list adds nothing; the file is still created if it did not exist."

### 2. Open-flag, short-write and NUL-terminated-path mechanics are compiler facts
UNIT:      man-page:fs/appendBytes
CLAIM:     "The file is opened with the append flag set, so every write is positioned at the current end of the file." / "The write is retried until every byte has been written or the host reports an output failure, so a short host write that transfers only part of the buffer is resumed rather than treated as complete, and an interruption never loses or duplicates bytes." / "The string must not be empty and must not contain an embedded NUL byte, because the host open call requires a NUL-terminated path."
VERDICT:   out-of-scope
EVIDENCE:  All three are true of `lower_fs_write_path_helper` (`open_flag_set(...).append` = O_WRONLY|O_CREAT|O_APPEND|O_CLOEXEC in `gen_open.rs`; `emit_transfer_loop_tail` EINTR retry; `emit_cstring_copy` NUL rejection). Each is phrased as implementation justification ("flag set", "host write", "buffer", "host open call requires a NUL-terminated path") per `.ai/man-content.md` §1/§3. The only developer-visible facts are: every append goes at the current end even if another writer extended the file; the whole list is written or `ErrWriteFailed` is raised; empty or NUL-containing paths are rejected with `ErrInvalidArgument`. Probe: `empty path: ErrInvalidArgument`, `nul path: ErrInvalidArgument` (probe.mfb, run from `p_probe/build/p_probe.out`).
SUGGESTED: "Every append lands at the file's current end, even when another program has extended the file since it was opened. Either every byte of bytes is written, or the call raises ErrWriteFailed. path must not be empty and must not contain a NUL character; either raises ErrInvalidArgument."

### 3. "an interruption never loses or duplicates bytes" contradicts the partial-failure sentence
UNIT:      man-page:fs/appendBytes
CLAIM:     "an interruption never loses or duplicates bytes" … "a failure partway through leaves the file extended by only the bytes written so far."
VERDICT:   misleading
EVIDENCE:  In `lower_fs_write_path_helper` "interruption" means EINTR on `write` (retried at `write_loop`, `emit_transfer_loop_tail` comment: "bug-62"); "failure" means a hard write error. To a developer both read as "something went wrong mid-write", so the page says bytes are never lost and then says they can be. The signal-retry distinction is not one the developer can observe or act on.
SUGGESTED: Drop "and an interruption never loses or duplicates bytes"; keep the partial-failure sentence, which is the sharp edge that matters.

### 4. ErrInvalidPath list omits a symlink loop
UNIT:      man-page:fs/appendBytes
CLAIM:     "ErrInvalidPath if path cannot name a file — it names a directory, a name in it is too long, or a file is used as a directory."
VERDICT:   incomplete
EVIDENCE:  `gen_shared.rs:emit_fs_host_error_mapping` maps ELOOP (and EILSEQ) to `invalid_path` when `no_follow` is false. Probe: `ln -sf loop.bin work/loop.bin`, then `fs::appendBytes(".../work/loop.bin", [1])` printed `code 77030002 Filesystem path string is invalid for the host platform.` The page's three cases are all verified (`is directory: ErrInvalidPath`, `long name: ErrInvalidPath`, `file as dir: ErrInvalidPath`) but the loop case is missing.
SUGGESTED: "… it names a directory, a name in it is too long, a file is used as a directory, or a symbolic link in it loops back on itself."

### 5. "the final path component is followed when it is a symlink" understates what is followed
UNIT:      man-page:fs/appendBytes
CLAIM:     "the final path component is followed when it is a symlink, so appending through a symlink appends to the target file."
VERDICT:   misleading
EVIDENCE:  `open_flag_set(platform.family(), false)` in `lower_fs_write_path_helper`: no `O_NOFOLLOW`/`O_NOFOLLOW_ANY`, so symlinks in every component are followed, not only the last. Singling out the final component suggests intermediate links are treated differently (as `fs::openFileNoFollow` does). Probe: `symlink: ok`, `work/target.bin` grew 5→8 bytes through `work/link.bin`.
SUGGESTED: "Symbolic links anywhere in path are followed, so appending through a link appends to the file it points to; use fs::openFileNoFollow when that must be refused."

### 6. The same facts are stated two and three times
UNIT:      man-page:fs/appendBytes
CLAIM:     "The bytes are written exactly as the list holds them." and, two sentences later, "Bytes are written exactly as held in the list, with no encoding, decoding, or newline translation…"; "An empty list leaves the file's length unchanged." (parameter) and "An empty byte list leaves the file's length unchanged, creating it as an empty file if it did not exist." (description); "creating it with no contents if it does not already exist" (paragraph 1) and "creating it as an empty file if it did not exist" (paragraph 2).
VERDICT:   misleading
EVIDENCE:  Rendered page, Description paragraphs 1–2 and the `bytes` row. All true (probe: `empty list new file: ok`, `empty.bin exists: TRUE size 0`; `empty list existing: ok now plain`; `xxd work/bin.bin` = `00ff 0a0d 07`), but the repetition pads the page and buries the one real sharp edge (non-atomic partial append) in paragraph 5.
SUGGESTED: Keep one "written exactly as held, with no encoding or newline translation, so it suits binary data" sentence and one empty-list sentence in the description; shorten the parameter row to the wording in finding 1.

### 7. Verified-true claims (for the audit record)
UNIT:      man-page:fs/appendBytes
CLAIM:     Examples; `0600` on create and existing mode kept; error rows; `value` alias; close on write-failure path.
VERDICT:   (no defect — evidence only)
EVIDENCE:  Both examples built with `mfb init` + `mfb build` and ran, exit 0; example 2 printed `header`, `first line`, `second line`. `stat -f '%Sp %z %N' work/*`: new files `bin.bin`, `empty.bin`, `named.bin` are `-rw-------` (umask 022), pre-existing `plain.txt`/`target.bin` remain `-rw-r--r--`. Error probes: `missing parent: ErrPathNotFound`, `denied: ErrAccessDenied` (chmod 000 file), plus the rows cited above; the Errors table lists exactly the six codes raised by `lower_fs_write_path_helper` and `emit_fs_host_error_mapping(FsFallback::Write)`. `fs::appendBytes(p, value := na)` compiled and appended `A`. `write_error` label calls `emit_close_file` before `raise_error_into("ErrWriteFailed")`. Banned-word grep against the rendered page hit only the derived message "Allocation failed." (§4.4 carve-out 2).
SUGGESTED: none.
