All probes are done and every runtime claim on the page checked out against the binary. Findings follow.

### 1. The write-retry paragraph explains a syscall model, not a developer fact
UNIT:      man-page:fs/appendText
CLAIM:     "The write is retried until every byte has been written or the host reports an output failure, so a short host write that transfers only part of the buffer is resumed rather than treated as complete, and an interruption never loses or duplicates bytes."
VERDICT:   out-of-scope
EVIDENCE:  True at HEAD: `src/codegen/builtins/fs/gen_atomic_write.rs:lower_fs_write_path_helper` loops on `write_loop` via `emit_transfer_loop_tail` (0 bytes → ErrWriteFailed, negative/EINTR → retry). But "short host write", "part of the buffer", and "interruption" are host-syscall vocabulary (`.ai/man-content.md` §1/§3). Nothing here is observable from MFBASIC: a developer sees either the whole text appended or ErrWriteFailed. The only developer-visible residue is already stated two paragraphs later ("a failure partway through leaves the file extended by only the bytes written so far").
SUGGESTED: Delete the sentence. Keep the following one ("An empty String leaves the file's length unchanged…"). If any of it is wanted, one sentence: "Either the whole of value is written or the call raises ErrWriteFailed; it never reports success after writing only part of it."

### 2. "because the host open call requires a NUL-terminated path" is a C justification
UNIT:      man-page:fs/appendText
CLAIM:     "The string must not be empty and must not contain an embedded NUL byte, because the host `open` call requires a NUL-terminated path."
VERDICT:   out-of-scope
EVIDENCE:  The rule is true: probe `p_nul` with path `"a\u{0}b"` printed `nul path: 77050002 Argument value is not valid…`; empty path printed the same code (`p_probe` run above). The "because" clause names the C `open` call and NUL-termination, which only matters to someone reading the lowering (`emit_cstring_copy` → `invalid` label). The developer needs the rule, not the C reason.
SUGGESTED: "The string must not be empty and must not contain a NUL byte; either raises ErrInvalidArgument."

### 3. "opened with the append flag set" names an OS flag instead of the observable behavior
UNIT:      man-page:fs/appendText
CLAIM:     "The file is opened with the append flag set, so every write is positioned at the current end of the file."
VERDICT:   misleading
EVIDENCE:  True (`open_flag_set(...).append` = `O_WRONLY|O_CREAT|O_APPEND|O_CLOEXEC`, `gen_open.rs:open_flag_set`). But "append flag" is POSIX vocabulary, and the sentence stops short of the fact a developer actually cares about: because the position is taken at write time, another program appending to the same file at the same time does not overwrite this text, and the two writes land one after the other. The paragraph as written is a restatement of the first sentence of the Description with an OS term added.
SUGGESTED: "The text always lands at the end of the file as it is at the moment of the write, so if another program appends to the same file at the same time neither overwrites the other; each addition follows whatever was there."

### 4. Page never points at fs::appendBytes
UNIT:      man-page:fs/appendText
CLAIM:     (See also lists only `fs::writeText`)
VERDICT:   incomplete
EVIDENCE:  `mfb man fs appendText` "See also" section contains one entry. `src/codegen/builtins/fs/func_append_bytes.rs` exists; a developer holding a `List OF Byte` will reach for `appendText` and hit a type error. `.ai/man-content.md` §2 requires "cross-references to the sibling a developer would otherwise reach for by mistake."
SUGGESTED: Append to the first Description paragraph: "To append raw bytes rather than text, use `fs::appendBytes`."

### 5. Windows directory-as-path claim is unverified here
UNIT:      man-page:fs/appendText
CLAIM:     "…including `path` naming a directory, which Windows reports as `ErrAccessDenied`."
VERDICT:   UNVERIFIED
EVIDENCE:  macOS half verified: probe `path is directory` on `adir` printed `77020002 Write or flush operation failed.` (EISDIR=21 falls through `emit_errno_error_mapping` to ErrWriteFailed). The Windows arm maps GetLastError 5 → ErrAccessDenied (`gen_shared.rs:emit_errno_error_mapping`), and `CreateFileW` with `FILE_APPEND_DATA` on a directory without backup semantics returns ERROR_ACCESS_DENIED, so the claim is consistent with the code, but no Windows binary was run. Nothing to change unless a Windows run disagrees.
SUGGESTED: Leave as is; the main thread may confirm on a Windows host.

No other findings. Verified true, with the command behind each: both page examples compile (`mfb build -q p_ex1`, `p_ex2`) and run; ex1 run twice produced `line\nline\n` at mode `-rw-------`, ex2 printed the four lines the page implies. The probe run (`p_probe/build/p_probe.out`) confirmed: empty value on a missing file creates a zero-length file; missing parent directory → ErrNotFound; read-only file → ErrAccessDenied; symlink is followed (`target.txt` became `Tvia-link\n`); `"a\r\nb"` is written as 4 bytes with no newline translation; a Unicode path and Unicode text write 10 bytes; an existing 0644 file keeps its mode; a created file gets 0600. The Errors table matches every raise site in `lower_fs_write_path_helper`, and ErrAlreadyExists is indeed only reachable through the shared errno mapping, as the page says. Memory-vocabulary scan of the rendered page: no banned words and no sentence teaching a borrow or ownership model.
