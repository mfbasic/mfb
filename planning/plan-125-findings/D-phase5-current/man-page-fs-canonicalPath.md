Probes are done. Writing up the findings.

### 1. Buffer-size sentence is a compiler fact and misdescribes where the result lives
UNIT:      man-page:fs/canonicalPath
CLAIM:     "The result is returned as a String in the host resolution buffer sized to hold up to PATH_MAX bytes plus the terminating NUL (4097)."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/fs/gen_canonical.rs:lower_fs_canonical_path_helper` — the 4097-byte buffer is the helper's own scratch, released at `done`; the returned String is `result`, a separate value. `PATH_MAX`, "terminating NUL" and the buffer size are implementation details (§3). The only developer-visible consequence is that a resolved path longer than the host limit raises `ErrInvalidPath`: probe `long-path` (a 50-component path of 100-byte names) printed `code 77030002`.
SUGGESTED: Delete the sentence. Fold the one visible fact into the error paragraph: "a path whose resolved form is longer than the host allows raises ErrInvalidPath."

### 2. "requires a NUL-terminated path" explains the rule via the C host call
UNIT:      man-page:fs/canonicalPath
CLAIM:     "The string must not be empty and must not contain an embedded NUL byte, because the host call requires a NUL-terminated path; either condition raises ErrInvalidArgument before any host call is made."
VERDICT:   out-of-scope
EVIDENCE:  Behavior confirmed: probe `empty` and probe `nul` (`"/tmp" & strings::fromScalars([`\0`]) & "x"`) both printed `code 77050002`. The *why* clause ("the host call requires a NUL-terminated path", "before any host call is made") is a C-string mental model (§1, §3); the developer needs the rule, not the calling convention.
SUGGESTED: "An empty path, or one containing a NUL character, raises ErrInvalidArgument."

### 3. "host realpath call" is not what happens on Windows
UNIT:      man-page:fs/canonicalPath
CLAIM:     "Resolution is performed by the host realpath call, which collapses . and .. components, removes redundant separators, and follows every symbolic link encountered along the way"
VERDICT:   misleading
EVIDENCE:  `src/target/win_x86_64/code.rs:emit_realpath` comment: "Open it (CreateFileW, OPEN_EXISTING…) and ask GetFinalPathNameByHandleW". Naming the POSIX function is both an internals leak (§3) and false on one supported host. The behavior itself verified on macOS: probe `double-slash` (`top & "//d//"`) → `…/mfb-cp-probe/d`; probe `link-file` (symlink to a file) → `…/target.txt`; probe `loop` → `code 77030002`. Windows output form UNVERIFIED (no Windows host here).
SUGGESTED: "Resolution collapses . and .. components, removes redundant separators, and follows every symbolic link along the way, including one at the final component, so the returned path names the real file or directory."

### 4. "does not open … any file" is untrue on Windows
UNIT:      man-page:fs/canonicalPath
CLAIM:     "The function reads filesystem metadata only; it does not open, create, or modify any file and has no other side effects."
VERDICT:   misleading
EVIDENCE:  `src/target/win_x86_64/code.rs:emit_realpath` opens the entry with CreateFileW to resolve it. Not developer-visible on macOS/Linux (verified no file created or changed: probe runs cleaned up their fixtures via `deleteFile`/`deleteDirectory` with no error). UNVERIFIED on Windows whether the open is observable (e.g. sharing-violation on an exclusively locked file).
SUGGESTED: "The function never creates or modifies anything; it only looks the path up." Drop "does not open".

### 5. "a missing component raises an error" leaves the code to a later paragraph
UNIT:      man-page:fs/canonicalPath
CLAIM:     "every component named by path, including the final one, must exist on the filesystem; a missing component raises an error."
VERDICT:   incomplete
EVIDENCE:  Probe `missing` (`top/nope`) and `missing-parent` (`top/nope/x`) both printed `code 77030001` (ErrPathNotFound); `unicode-missing` likewise. The code is stated three paragraphs later; the sentence that introduces the rule should name it.
SUGGESTED: "…must exist on the filesystem; a missing component raises ErrPathNotFound."

### 6. A trailing separator is accepted and dropped, and the page does not say so
UNIT:      man-page:fs/canonicalPath
CLAIM:     "A relative path is resolved against the current working directory; an absolute path is canonicalized in place."
VERDICT:   incomplete
EVIDENCE:  Probe `trailing-slash-dir` (`top/d/`) → `…/mfb-cp-probe/d`; probe3 `top & "/"` equals `canonicalPath(top & "/.")` → `TRUE`. The result never carries a trailing separator. Also "canonicalized in place" reads as mutation; the function returns a new value and `path` is unchanged (`LET` binding, probes compile with an immutable `path` argument).
SUGGESTED: "A relative path is resolved against the current working directory. The result is always absolute, never ends in a separator, and is a new value; path itself is not changed."

### 7. Error paragraph: EPERM and macOS trailing-slash-on-file not covered
UNIT:      man-page:fs/canonicalPath
CLAIM:     "ErrInvalidPath if path cannot name an entry — a file used as a directory, a name that is too long, or a loop of symbolic links."
VERDICT:   incomplete
EVIDENCE:  Verified rows: `file-as-dir` (`top/f.txt/x`) → 77030002; `long-name` (300-byte component) → 77030002; `loop` → 77030002; `noperm` (dir chmod 000) → 77030003 ErrAccessDenied. `gen_shared.rs:emit_fs_host_error_mapping` also maps `EILSEQ` (a byte sequence the host cannot use as a name) to ErrInvalidPath, which the page omits. Note that macOS accepted `top/f.txt/` (probe `trailing-slash-file` printed the file path), so "a file used as a directory" means an extra component after a file, not a trailing slash; the page should not imply otherwise.
SUGGESTED: "ErrInvalidPath if path cannot name an entry: a component after a file, a name the host cannot use or that is too long, or a loop of symbolic links."

Example verification (not a finding): both page examples built with `mfb build` and ran. Example 1 printed `/private/var/folders/…/T/mfb-canonicalPath-1/report.txt`, which also shows the page's claim about symlink following in action since `/var` → `/private/var` on macOS. Example 2 printed `TRUE`. Probe sources and outputs are under `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-canonicalPath/{probe,probe2,probe3}.mfb`.
