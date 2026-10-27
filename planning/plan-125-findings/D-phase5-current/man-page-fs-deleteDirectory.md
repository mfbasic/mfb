### 1. A trailing slash makes the symbolic-link guarantee false
UNIT:      man-page:fs/deleteDirectory
CLAIM:     "`fs::deleteDirectory` does not recurse and never removes a file or a symbolic link; use `fs::deleteFile` to remove a non-directory entry."
VERDICT:   misleading
EVIDENCE:  Probe `p_probe/src/main.mfb` under `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-deleteDirectory`, built with `mfb build -q p_probe` and run. Fixture: `mkdir target; ln -s …/target link`. Call `fs::deleteDirectory(link)` printed `symlink-to-dir: … TRUE invpath` (refused, as the page says). Call `fs::deleteDirectory(link & "/")` printed `symlink-to-dir-trailing-slash: ok`; afterwards `ls …/target` printed `No such file or directory` and `ls -la …/link` still showed the link, now dangling. The lowering is a bare host `rmdir` (`gen_directory/mod.rs:lower_fs_path_operation_helper`, `macos_aarch64/code.rs` maps `Rmdir => "_rmdir"`), and POSIX resolves a trailing slash through the link. So the sentence is literally true (the link is not removed) but the reader's takeaway, that a symlink is a safe no-op, is wrong: the directory the link points at is removed.
SUGGESTED: "`fs::deleteDirectory` does not recurse and never removes a file or a symbolic link itself; use `fs::deleteFile` to remove a non-directory entry. A symbolic link to a directory is refused with `ErrInvalidPath` — but only when the path ends at the link: a `path` that ends in a separator (`link/`) is resolved through the link, and the directory it points at is removed while the link is left behind."

### 2. The Errors paragraph omits the symbolic-link case it told the reader about
UNIT:      man-page:fs/deleteDirectory
CLAIM:     "`ErrInvalidPath` if `path` names a file or cannot name a directory — a name that is too long, a file used as a directory, or a loop of symbolic links."
VERDICT:   incomplete
EVIDENCE:  Same probe: `symlink-to-dir: FALSE notfound, FALSE busy, TRUE invpath …`. The description paragraph above says a symbolic link is never removed, but the error paragraph never says which error a symbolic link raises; "names a file" does not cover it under the page's own distinction between files and links. Mapping: `gen_shared.rs:emit_fs_host_error_mapping` sends `ENOTDIR` (20) to `invalid_path`.
SUGGESTED: "`ErrInvalidPath` if `path` names a file or a symbolic link, or cannot name a directory — a name that is too long, a file used as a directory, or a loop of symbolic links."

### 3. Windows removes a directory symbolic link outright
UNIT:      man-page:fs/deleteDirectory
CLAIM:     "`fs::deleteDirectory` does not recurse and never removes a file or a symbolic link"
VERDICT:   wrong (on Windows) — UNVERIFIED on this host
EVIDENCE:  `src/target/win_x86_64/code.rs:emit_fs_path_operation` maps `FsPathOperation::Rmdir => "RemoveDirectoryW"` with no reparse-point check. Microsoft's documented contract for `RemoveDirectoryW` is that it removes a directory symbolic link or junction itself, regardless of the target's state, and leaves the target untouched. That is the opposite of the page's promise and of the macOS/Linux behaviour probed above. I cannot run a Windows binary from this sandbox; this needs a Windows probe (`fs::deleteDirectory` on a `mklink /D` link, then check both link and target).
SUGGESTED: Either state the platform split plainly ("On Windows a directory symbolic link or junction is itself removed and its target is left alone; on macOS and Linux the link is refused with `ErrInvalidPath`"), or, if the intent is one contract, that is a code bug in the Windows lowering rather than a prose fix.

### 4. Unexplained `ErrWriteFailed` cases the developer will actually hit
UNIT:      man-page:fs/deleteDirectory
CLAIM:     "Any other refusal raises `ErrWriteFailed`."
VERDICT:   incomplete
EVIDENCE:  Same probe: `fs::deleteDirectory(".")` printed `dot: … TRUE writefailed; Write or flush operation failed.` The host answers `EINVAL` for `.` (and `EBUSY` for the current working directory or a mount point), neither of which is in the arm list in `gen_shared.rs:emit_fs_host_error_mapping`, so they fall to `FsFallback::Write`. "Write or flush operation failed" is a baffling message for "you asked to remove `.`"; the page should name the common causes so the reader can recognise them.
SUGGESTED: "Any other refusal raises `ErrWriteFailed`; the usual causes are asking to remove `.`, the current working directory, or a directory that is a mount point."

The two page examples both compile and run as the page implies: `p_ex1` printed `FALSE` and `p_ex2` printed `TRUE` (built with `mfb build -q p_ex1` / `p_ex2`, run from `build/`). Verified as stated by the same probe: missing path and missing parent raise `ErrPathNotFound`; non-empty raises `ErrResourceBusy` and leaves the directory in place; a regular file and a file used as a directory raise `ErrInvalidPath`; empty path and embedded NUL raise `ErrInvalidArgument`; a permission refusal (parent `chmod 000`) raises `ErrAccessDenied`; a relative path, a Unicode name and a trailing slash on a real directory all succeed. No banned vocabulary and no ownership-model sentence found on the rendered page.
