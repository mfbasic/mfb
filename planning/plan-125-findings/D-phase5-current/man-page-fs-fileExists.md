Review complete. Both page examples compile and run (example 1 prints `hello`, example 2 prints `FALSE`), and the edge-case probe at `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-fileExists/proj1/src/main.mfb` confirmed TRUE/FALSE for regular, missing, directory, symlink-to-file, symlink-to-dir, broken symlink, FIFO, `/dev/null`, empty path, relative path, and `ErrInvalidArgument` (77050002) for a NUL byte. Findings follow.

### 1. "unreadable parent directory" names the wrong permission
UNIT:      man-page:fs/fileExists
CLAIM:     "A failed `stat` — for example a missing path or an unreadable parent directory — is reported as `FALSE` rather than raised as an error."
VERDICT:   misleading
EVIDENCE:  Probe with two parents: `chmod 600 fx/noexec` (readable, not searchable) and `chmod 300 fx/noread` (not readable, searchable), each holding `inner.txt`. Output: `noexec parent (rw-, no search) -> FALSE`, `noread parent (-wx, searchable) -> TRUE`. A parent the program cannot *read* still yields TRUE; it is the parent the program cannot *search* (traverse) that yields FALSE. The page's reader will conclude a listing-denied directory hides its files, which it does not.
SUGGESTED: "A failed lookup — a missing path, or a parent directory the program is not permitted to traverse — is reported as `FALSE` rather than raised as an error."

### 2. Mode-mask arithmetic is a compiler fact
UNIT:      man-page:fs/fileExists
CLAIM:     "The check masks the entry's mode with the file-type bits (`61440`) and compares against the regular-file type (`32768`), so only regular files qualify."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/error/constants/error_constants.rs:FS_MODE_TYPE_MASK`/`FS_MODE_REGULAR` and `src/target/macos_aarch64/code.rs:emit_stat_is_kind`. The numbers are correct (`S_IFMT`/`S_IFREG`), but they describe how the lowering tests `st_mode`, which only matters to someone reading the backend. The preceding sentence already states the developer-visible fact in full (TRUE only for a regular file; FALSE for directory, FIFO, socket, device node). §3 of `.ai/man-content.md`: "if a sentence only matters to someone reading compiler source, it does not belong on the page."
SUGGESTED: Delete the sentence. If anything, it belongs in the `fs` spec topic.

### 3. `stat` / `lstat` are host C library names, not MFBASIC
UNIT:      man-page:fs/fileExists
CLAIM:     "The final path component is followed when it is a symlink, because the host `stat` call is used rather than `lstat`" (also "`fs::fileExists` stats `path`" and "A failed `stat`")
VERDICT:   out-of-scope
EVIDENCE:  `src/target/macos_aarch64/code.rs:emit_path_stat` branches to `_stat`; that is the implementation, and the "because" clause explains behaviour by naming it. Probe confirms the developer-visible fact without the vocabulary: `link_to_reg -> TRUE`, `link_to_dir -> FALSE`, `link_broken -> FALSE`. A reader who does not know the POSIX `stat`/`lstat` distinction gets nothing from the clause; one who does already knew.
SUGGESTED: "`fs::fileExists` looks `path` up and reports whether it names a regular file." / "A symlink as the final path component is followed: a symlink pointing at a regular file reports `TRUE`, and a symlink whose target is missing or non-regular reports `FALSE`." / "A failed lookup — …".

### 4. Second example cannot show what its caption claims
UNIT:      man-page:fs/fileExists
CLAIM:     "Unicode paths are accepted:" followed by the example that prints `toString(present)`
VERDICT:   misleading
EVIDENCE:  Built and ran `ex2.mfb` in `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-fileExists/proj1`; output is `FALSE`. The file is never created, so the program prints `FALSE` whether the path was accepted or silently rejected. The example proves nothing about Unicode acceptance, and a reader running it sees a result that looks like rejection.
SUGGESTED: Write the file first, then test and delete it, so the page's expected output is `TRUE`:
```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET path AS String = fs::pathJoin([fs::tempDirectory(), "é日😀.txt"])
  fs::writeText(path, "")
  io::print(toString(fs::fileExists(path)))
  fs::deleteFile(path)
END FUNC
```

### 5. Two sharp edges omitted: no `~` expansion, trailing slash
UNIT:      man-page:fs/fileExists
CLAIM:     "`path` is interpreted as UTF-8 bytes and passed to the host filesystem; it may be absolute or relative to the current working directory"
VERDICT:   incomplete
EVIDENCE:  Probe: `tilde -> FALSE` for `"~/.zshrc"` although `ls -la ~/.zshrc` shows the file exists; `trailing slash on regular -> FALSE` for `fx/reg.txt/` although `fx/reg.txt` reports TRUE. Neither is mentioned, and both are the first thing a shell user will try.
SUGGESTED: Append: "No shell expansion is applied: `~` is a literal character, not the home directory. A trailing `/` makes the path name a directory, so `fs::fileExists(\"notes.txt/\")` is `FALSE` even when `notes.txt` is a regular file."

### 6. Parameter description omits empty and NUL behaviour
UNIT:      man-page:fs/fileExists
CLAIM:     "The filesystem path to test, as UTF-8 bytes; absolute or relative to the current working directory."
VERDICT:   incomplete
EVIDENCE:  §2 of `.ai/man-content.md` requires every parameter description to state what happens at empty. The Description body covers it, but the Parameters table, which is what the package function table and quick readers see, does not. Probe: `empty -> FALSE`; `nul in path -> ERROR 77050002`.
SUGGESTED: "The filesystem path to test, absolute or relative to the current working directory. An empty path is `FALSE`; a path containing a NUL byte raises `ErrInvalidArgument`."
