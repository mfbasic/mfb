### 1. "a loop of symbolic links" is refused only when the loop is in a parent component; a looping final link is removed
UNIT:      man-page:fs/deleteFile
CLAIM:     "and `ErrInvalidPath` if `path` cannot name a file — it names a directory, a name in it is too long, a file is used as a directory, or it is a loop of symbolic links."
VERDICT:   misleading
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-deleteFile/probe.mfb` with shell fixtures `loop1 -> loop2 -> loop1` under the temporary directory. Output: `loop-component: code=77030002 Filesystem path string is invalid for the host platform.` for `.../loop1/x`, but `loop-final: ok` for `.../loop1`, and `ls` afterwards shows `loop1` gone and `loop2` still present. The page's own earlier sentence ("the link itself is removed rather than the file it points to") already implies this; the Errors paragraph contradicts it by listing a symlink loop as an unconditional refusal. The mapping is in `src/codegen/builtins/fs/gen_shared.rs:emit_fs_host_error_mapping` (`ELOOP` arm), which only fires when the host resolves a non-final component.
SUGGESTED: "...or a directory on the way to it is a loop of symbolic links. A symbolic link at the end of `path` is never followed, so even a self-referential one is simply removed."

### 2. Deleting a file that is still open is not mentioned
UNIT:      man-page:fs/deleteFile
CLAIM:     "The function removes exactly one non-directory entry; it does not recurse and it does not remove directories." (the description as a whole, by omission)
VERDICT:   incomplete
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-deleteFile/open.mfb`: `fs::writeText(p, "held")`, `RES h = fs::openFile(p)`, then `fs::deleteFile(p)`. Output: `exists after delete while open: FALSE`, exit 0 — the delete succeeds while a `File` handle to it is open, and the handle stays usable. The Windows open uses `FILE_SHARE_READ|WRITE|DELETE` (`src/target/win_x86_64/code.rs` comment at the `CreateFileW` call, "dwShareMode=7") so `DeleteFileW` is not refused there either, but I could not run Windows; whether `fs::exists` still reports `TRUE` until the last handle closes on Windows is UNVERIFIED. This is the sharp edge a developer reaches for this page to learn, and a sibling (`fs::openFile`) cannot answer it.
SUGGESTED: Add after the directory sentence: "A file that is currently open through a `File` handle can still be deleted; the open handle keeps working until it is closed, and `fs::exists` reports the name gone."

### 3. Host syscall name used as the explanation
UNIT:      man-page:fs/deleteFile
CLAIM:     "When the final component of `path` is a symbolic link, the link itself is removed rather than the file it points to, because `unlink` does not follow a trailing symlink."
VERDICT:   out-of-scope
EVIDENCE:  `.ai/man-content.md` §1/§3: a sentence that only means something to someone who knows the host API does not belong on the page. "`unlink`" appears twice ("with a single host `unlink` operation", "because `unlink` does not follow"). The developer-visible fact (the link, not its target, is removed; verified by `probe.mfb`: `symlink: ok` then `link exists after: FALSE target exists: TRUE`, and `dangling-symlink: ok`) stands on its own without the POSIX name. The ban is a judgment call, not a mechanical hit: `unlink` is OS vocabulary, not compiler vocabulary, so this is the weakest finding here.
SUGGESTED: "`fs::deleteFile` removes the filesystem entry named by `path` in one step. ... When the final component of `path` is a symbolic link, the link itself is removed and the file it points to is left in place; a link whose target no longer exists is removed the same way."

### 4. Second example prints a value the page never states
UNIT:      man-page:fs/deleteFile
CLAIM:     "Write a file and then remove it:" (second example, ending `io::print(toString(fs::exists(scratch)))`)
VERDICT:   incomplete
EVIDENCE:  Built as `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-deleteFile/proj_ex2` with `mfb build`; running `proj_ex2/build/proj_ex2.out` prints `FALSE`, exit 0. The example compiles and runs, but the reader is given a print statement with no expected output, so they cannot tell whether `FALSE` is the intended result or a bug in their copy.
SUGGESTED: Change the lead-in to "Write a file, remove it, and confirm it is gone — this prints `FALSE`:".

Everything else on the page checked out against the binary. Both examples build and run (`proj_ex1` exits 0 with no output; `proj_ex2` prints `FALSE`). The probe confirmed: missing file, missing parent and a relative missing name raise `ErrPathNotFound` (77030001); a directory, a file used as a directory, a 300-byte name and a trailing-slash file raise `ErrInvalidPath` (77030002); a file in a directory with no write permission raises `ErrAccessDenied` (77030003); empty path and a path with an embedded NUL raise `ErrInvalidArgument` (77050002); a Unicode name is removed (`unicode: ok`, `exists` then `FALSE`). The `ErrWriteFailed` fallback for any other host code was verified by reading `FsFallback::Write` in `lower_fs_path_operation_helper`, not by provoking it. No banned memory vocabulary appears on the rendered page.
