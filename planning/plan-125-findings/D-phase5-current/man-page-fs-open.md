### 1. "portable mode names" implies non-portable ones exist
UNIT:      man-page:fs/open
CLAIM:     "The portable mode names are `"read"` or `"r"`, `"write"` or `"w"`, `"readWrite"` or `"rw"`, and `"append"` or `"a"`."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/fs/gen_open.rs:lower_fs_open_helper` matches exactly eight literals (`r`, `read`, `w`, `write`, `rw`, `readWrite`, `a`, `append`) and branches everything else to `ErrInvalidArgument`; there is no platform-specific set. Probe `probe1.mfb` under `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-open`: `"r+"`, `"readwrite"`, `"READ"`, `"Read"`, `"r "` and `""` all printed `err 77050002`. The word "portable" tells a reader that `"r+"` or `"wb"` might work on some host; nothing does.
SUGGESTED: "The mode names are `"read"` or `"r"`, `"write"` or `"w"`, `"readWrite"` or `"rw"`, and `"append"` or `"a"`; there are no others."

### 2. Reading a write-only handle is a sharp edge the page omits
UNIT:      man-page:fs/open
CLAIM:     "`"write"` opens the file for writing, creating it when it does not exist and truncating it to empty when it does. ... `"append"` opens the file for writing with every write directed to the end of the file"
VERDICT:   incomplete
EVIDENCE:  Only `"read"` is described as "for reading only"; the page never says what a `"write"` or `"append"` handle does when read, nor what a `"read"` handle does when written. Probe `probe2.mfb` (cwd `/tmp/mfbprobe`): on a `"w"` handle `fs::readAllBytes` returned 0 bytes with no error, `fs::eof` returned `TRUE`, and `fs::readLine` raised `77020001 Read operation failed`; on an `"a"` handle `fs::readAll` returned `[]` with no error. Probe `probe1.mfb`: `fs::writeAll` on an `"r"` handle raised `77020002 Write or flush operation failed`. The flag sets in `gen_open.rs:open_flag_set` are `O_RDONLY` / `O_WRONLY|...` / `O_RDWR|...` / `O_WRONLY|O_APPEND|...`, so this is by design. Side note for the main thread, not a page defect: the whole-file readers swallow the write-only refusal and report an empty file while `readLine` raises, which is an inconsistency in the sibling readers worth a bug.
SUGGESTED: Add after the mode list: "A `"read"` handle refuses writes with `ErrWriteFailed`. A `"write"` or `"append"` handle cannot be read: `fs::readLine` raises `ErrReadFailed`, and the whole-file readers report it as empty. Use `"readWrite"` when you need both."

### 3. "append" wins over seek, and the page does not say so
UNIT:      man-page:fs/open
CLAIM:     "`"append"` opens the file for writing with every write directed to the end of the file, creating it when it does not exist."
VERDICT:   incomplete
EVIDENCE:  Probe `probe1.mfb`: opened `keep.txt` (contents `keep`) with `"a"`, called `fs::seek(ap, 0)`, wrote `Z`, closed; file read back as `keepZ`. True to the claim, but a developer who seeks then writes will be surprised and the page is the place that defines the mode.
SUGGESTED: "`"append"` opens the file for writing with every write directed to the end of the file, even after `fs::seek`, creating it when it does not exist."

### 4. Symlink paragraph understates both this function and the alternative it points at
UNIT:      man-page:fs/open
CLAIM:     "The final path component is followed when it is a symlink, so opening through a symlink opens its target. To refuse a symlinked final component, use `fs::openFileNoFollow` instead."
VERDICT:   misleading
EVIDENCE:  `fs::open` follows a symlink at every component, not only the last one (plain `open` with no `O_NOFOLLOW` bit in `gen_open.rs:open_flag_set` for `no_follow = false`); `probe1.mfb` `symlink r` printed `ok` through `link.txt`. And `fs::openFileNoFollow`'s own page, rendered with `mfb man fs openFileNoFollow`, says it "refuses to traverse a symbolic link at any component of path, not just the terminal name" (Linux `openat2 RESOLVE_NO_SYMLINKS`, macOS `O_NOFOLLOW_ANY`, per the same file). This page sends a reader to the sibling for a narrower guarantee than the sibling actually gives, and the two pages disagree.
SUGGESTED: "Symbolic links anywhere in `path` are followed, so opening through a symlink opens its target. To refuse a path that passes through any symbolic link, use `fs::openFileNoFollow` instead."

### 5. "File record cannot be built ... never leaks a host fd" is compiler-side mechanics
UNIT:      man-page:fs/open
CLAIM:     "If the `File` record cannot be built after the file is opened, the file is closed before the error is reported, so a failed open never leaks a host fd."
VERDICT:   out-of-scope
EVIDENCE:  This describes the `arena_alloc` failure branch after `emit_open_file` in `gen_open.rs:lower_fs_open_helper` ("The File-record alloc failed after `open` succeeded: close the fd before reporting OOM"). "`File` record cannot be built", "host fd" and "leaks" are all the implementation's vocabulary (§3: no codegen mechanics, no Rust-side item concepts). The only developer-visible fact is that a failed `fs::open` hands back nothing that needs closing, and that is already implied by the function raising instead of returning. I could not trigger the branch (`ErrOutOfMemory`: UNVERIFIED, no way to exhaust memory deterministically from a probe).
SUGGESTED: Delete the sentence. If anything is kept: "When `fs::open` raises, there is nothing to close."

### 6. The NUL-byte rule is justified with a C-level reason
UNIT:      man-page:fs/open
CLAIM:     "The string must not be empty and must not contain an embedded NUL byte, because the host `open` call requires a NUL-terminated path."
VERDICT:   out-of-scope
EVIDENCE:  The rule itself is verified (`probe1.mfb`: `empty path` and `nul path` built with `strings::fromScalars([toScalar(0)])` both printed `err 77050002`), and the rejection is the `emit_cstring_copy(..., reject_nul = true, ...)` branch in `gen_open.rs`. The "because" clause explains the C calling convention, which only matters to someone reading the lowering (§1 test). The developer needs the rule and the error, not the reason.
SUGGESTED: "The string must not be empty and must not contain a NUL byte; either raises `ErrInvalidArgument`."

### 7. Example 3 prints a trailing blank line the prose does not prepare the reader for
UNIT:      man-page:fs/open
CLAIM:     "Open a file for appending so each write lands at the end:" (the third example, which ends with `io::print(fs::readText(path))` after writing `"started\n"`)
VERDICT:   incomplete
EVIDENCE:  Built and run via `runone.sh ex3.mfb` under the scratch directory: stdout was `started` followed by an empty line (the file content already ends in a newline and `io::print` adds another), exit 0. Examples 1 and 2 ran clean: example 1 printed nothing and removed its directory; example 2 printed `hello`. All three compiled. Not broken, but a reader comparing their terminal to "each write lands at the end" sees a blank line the page never mentions, and the example makes only one write, so it does not demonstrate appending at all.
SUGGESTED: Make the example append twice and print without the extra newline so the output shows the claim, e.g. write `"started\n"`, close, reopen with `"a"`, write `"stopped\n"`, close, then print the file; or write the lines without trailing `\n` and let `io::print` supply it.

Verified and not reported: every row of the Errors table against a live condition on macOS at HEAD (`probe1.mfb`: missing file in any mode and missing parent in `w`/`a`/`rw` give `77030001`; a directory in all four modes, a symlink loop, a 300-byte name and a file used as a directory give `77030002`; a `chmod 000` file in `r` and `w` gives `77030003`; `probe2.mfb` under `ulimit -n 64`: descriptor exhaustion gives `77020001` for `"r"` and `77020002` for `"a"`, matching the `ByAccess` fallback in `gen_shared.rs:emit_fs_host_error_mapping`). Created files are `-rw-------` for `w`, `rw` and `a` under umask 022 (`stat -f %Sp` on `rwnew.txt`, `anew.txt`, `wnew.txt`). `"readWrite"` preserved `keep` and read it back before writing; `"write"` truncated it to empty. Scope-exit close holds: 500 opens with no `fs::close` under a 64-descriptor limit all succeeded, and a recursion holding one open handle per frame hit the limit at depth 62. `ErrOutOfMemory` and the Windows permission wording are UNVERIFIED (no deterministic trigger, no Windows host). No banned memory word appears on the rendered page; the "leaks a host fd" sentence in finding 5 is the §4-style mental-model leak that no grep catches.
