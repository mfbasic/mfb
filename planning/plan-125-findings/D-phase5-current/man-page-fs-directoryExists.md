### 1. Mode-bit arithmetic is compiler mechanics, not developer fact
UNIT:      man-page:fs/directoryExists
CLAIM:     "The check masks the entry's mode with the file-type bits (61440) and compares against the directory type (16384), so only directories qualify."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/fs/gen_exists.rs:lower_fs_kind_exists_helper` and `src/target/macos_aarch64/code.rs:emit_stat_is_kind` (the `S_IFMT` mask and `FS_MODE_DIRECTORY`). This is a description of the lowering, and it is only true of the POSIX backends: `src/target/win_x86_64/code.rs:emit_stat_is_kind` tests the `FILE_ATTRIBUTE_DIRECTORY` bit of a `GetFileAttributesW` result and never touches a mode word. A developer gains nothing from the two magic numbers; the preceding sentence already says only directories qualify.
SUGGESTED: Delete the sentence. The sentence before it ("It returns `TRUE` only when `path` exists and the resolved entry is a directory; it returns `FALSE` for a missing path, a regular file, or any other non-directory entry ...") already carries the whole developer-visible fact.

### 2. "stat rather than lstat" is a justification by host call, and it is not the host call on Windows
UNIT:      man-page:fs/directoryExists
CLAIM:     "The final path component is followed when it is a symlink, because the host `stat` call is used rather than `lstat`: a symlink pointing at a directory reports `TRUE`, and a symlink whose target is missing or non-directory reports `FALSE`."
VERDICT:   misleading
EVIDENCE:  The developer-visible half is verified on macOS: probe `p_probe` under the scratch dir printed `link_dir => TRUE`, `link_dangling => FALSE`, `link_file => FALSE` (fixtures: `ln -s dir fx/link_dir; ln -s nowhere fx/link_dangling; ln -s file fx/link_file`). The "because `stat` not `lstat`" clause is implementation vocabulary (§3), and it is false as stated for the Windows build: `src/target/win_x86_64/code.rs:emit_path_stat` calls `GetFileAttributesW`, which reports the reparse point's own attributes and does not resolve the link target. Whether a dangling directory symlink reports `TRUE` on Windows is therefore UNVERIFIED (no Windows host in this sandbox); the sentence as written promises `FALSE` on every platform on the strength of a syscall that platform does not use.
SUGGESTED: "A symlink in the final position is followed: a link to a directory reports `TRUE`, and a link whose target is missing or is not a directory reports `FALSE`." Then either confirm the Windows behaviour on a Windows host or add "On Windows a directory symlink reports `TRUE` whether or not its target exists" if that is what a probe shows.

### 3. "stats" / "A failed stat" leak the syscall name; "unreadable parent" names the wrong permission
UNIT:      man-page:fs/directoryExists
CLAIM:     "A failed `stat` — for example a missing path or an unreadable parent directory — is reported as `FALSE` rather than raised as an error."
VERDICT:   misleading
EVIDENCE:  `p_probe2`: with `chmod 000 fx/ro`, `fx/ro/inner => FALSE`; with `chmod 311 fx/ro` (search permission but no read permission), `fx/ro/inner => TRUE`. So a parent that is merely unreadable does not make the check fail; a parent you may not traverse does. Also `fx/ro` itself with mode 000 reports `TRUE`, which the page does not say. The first description sentence, "`fs::directoryExists` stats `path`", has the same jargon problem.
SUGGESTED: Opening sentence: "`fs::directoryExists` looks `path` up on the filesystem and reports whether it is a directory." This sentence: "When the lookup fails — the path is missing, or a directory on the way to it cannot be traversed — the result is `FALSE`; no error is raised. A directory you have no permission to read still reports `TRUE`, because only its existence is tested."

### 4. No cross-reference to the siblings a developer will reach for by mistake
UNIT:      man-page:fs/directoryExists
CLAIM:     (absence) The page never names `fs::exists` or `fs::fileExists`, so the derived See also section is empty.
VERDICT:   incomplete
EVIDENCE:  Rendered page has no "See also" section (`mfb man fs directoryExists`). `.ai/man-content.md` §2 requires "Cross-references to the sibling a developer would otherwise reach for by mistake." `fs::exists` returns `TRUE` for a regular file as well (`gen_exists.rs:lower_fs_exists_helper` uses `access`), which is exactly the confusion a reader of this page is trying to avoid.
SUGGESTED: Add to the description: "Use `fs::exists` to accept any kind of entry, or `fs::fileExists` to require a regular file."

### 5. Second example demonstrates nothing: it prints FALSE for a path that does not exist
UNIT:      man-page:fs/directoryExists
CLAIM:     "Unicode paths are accepted:" followed by the `é日😀` example.
VERDICT:   misleading
EVIDENCE:  Built as `p_ex2` and run: output is `FALSE`. The example never creates the directory, so `FALSE` is indistinguishable from "the name was rejected". The behaviour it is captioned to show is true (my probe with an existing `fx/é日😀` printed `unicode => TRUE`), but the example as printed cannot show it. The first example compiles and runs as described (`p_ex1` printed `FALSE` then `found, 0 entries`, on two consecutive runs).
SUGGESTED:
```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET dir AS String = fs::pathJoin([fs::tempDirectory(), "é日😀"])
  fs::createDirectory(dir)
  io::print(toString(fs::directoryExists(dir)))
  fs::deleteDirectory(dir)
END FUNC
```
which prints `TRUE`.

Everything else on the page checked out against the binary. The remaining claims and the probe outputs that back them, all from `p_probe` and `p_nul` under the scratch directory: a regular file, a FIFO, `/dev/null`, a missing path and the empty string all print `FALSE`; a directory, `.`, `fx/dir/` with a trailing slash and `fs::tempDirectory()` print `TRUE`; `fx/file/` with a trailing slash prints `FALSE`; a path with an embedded NUL raises code `77050002` (`ErrInvalidArgument`), matching the Errors table and `gen_exists.rs`'s `invalid` label. The only `ErrOutOfMemory` site is the path-copy allocation at the top of the helper, consistent with the page. The banned-vocabulary grep over the rendered page hits only the derived message "Allocation failed." in the Errors table, which is the §4.4 carve-out. No sentence teaches a borrow or ownership model.
