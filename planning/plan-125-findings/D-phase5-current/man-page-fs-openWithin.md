### 1. Example 3's caption promises a symlink refusal the code never exercises
UNIT:      man-page:fs/openWithin
CLAIM:     "Write beneath the root; a symlinked component makes the open fail:"
VERDICT:   misleading
EVIDENCE:  The block that follows creates `reports/`, opens `reports/today.txt` for `"write"`, writes `hello`, reads it back and cleans up. No symbolic link is created and no failure is shown. Compiled and run from `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-openWithin/ex3.mfb`; output was exactly `hello`, exit 0. The refusal itself is real (probe `probe.mfb`: `rel symlink dir component (inside root) -> ErrAccessDenied`, `rel symlink final (inside root) -> ErrAccessDenied`), but the example does not demonstrate it.
SUGGESTED: Either retitle the block "Write a new file beneath the root:" or add the demonstration, e.g. after the write: create `links` as a symbolic link to `reports` (the page has no `fs` member for that, so the honest fix is the retitle), or keep the caption and show a `TRAP(e)` printing `e.code = errorCode::ErrAccessDenied` on `openWithin(root, "links/today.txt")`.

### 2. Symlink refusal is stated as universal, but on Windows only an escape is refused
UNIT:      man-page:fs/openWithin
CLAIM:     "A relPath is therefore refused when it is absolute, contains a .. component, or traverses a symbolic link at any component."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/fs/gen_open.rs:lower_fs_open_within_helper` — the `PlatformFamily::Windows` arm takes the plain-open path and then calls `platform.emit_verify_within`, whose contract (`src/target/win_x86_64/code.rs:emit_verify_within`, "Returns 0 (contained) / 1 (refuse) ... fileFinal must start with rootFinal + '\'") accepts any open whose final path is still under the root. A link or junction that stays inside `root` is therefore followed and allowed on Windows, while macOS (probed: `rel symlink dir component (inside root) -> ErrAccessDenied`) and Linux refuse it. The page describes Linux and macOS by name and says nothing about Windows. UNVERIFIED by execution (no Windows host in this sandbox); source-backed only.
SUGGESTED: Add after the sentence: "On Windows the join is opened first and then checked against the root's resolved path: a link that stays inside root is followed, and only one that leads outside it is refused with ErrAccessDenied."

### 3. The seccomp clause overstates when the fallback engages
UNIT:      man-page:fs/openWithin
CLAIM:     "On a Linux kernel too old for openat2 (ENOSYS, pre-5.6, or a restrictive seccomp filter) the open falls back to O_NOFOLLOW"
VERDICT:   misleading
EVIDENCE:  `gen_open.rs:lower_fs_open_within_helper`, Linux branch: after `openat2` fails it reads errno and `compare_immediate(&openat2_errno, "38")` / `branch_ne(&open_error)` — only `ENOSYS` (38) takes the fallback. A seccomp filter that answers `EPERM` (the common `SCMP_ACT_ERRNO(EPERM)` configuration) goes to `open_error` and raises `ErrAccessDenied`; it does not fall back. UNVERIFIED by execution (macOS sandbox); read from the lowering.
SUGGESTED: "On a Linux kernel without openat2 (pre-5.6, or a seccomp filter that reports it as unimplemented) the open falls back to O_NOFOLLOW, ... A filter that refuses openat2 with a permission error instead raises ErrAccessDenied."

### 4. The joined-path limit is a fixed byte count, not "what the host allows"
UNIT:      man-page:fs/openWithin
CLAIM:     "ErrInvalidArgument means root or relPath is empty or contains a NUL byte, relPath is absolute or has a .. component, mode is not one of the names above, or the joined path is longer than the host allows."
VERDICT:   misleading
EVIDENCE:  `gen_open.rs:lower_fs_open_within_helper`: `PATH_MAX_PLUS_NUL = 4097`; `need = rlen + rel_len + 2; branch_hi(&invalid)` when `need > 4097`, before any host call. The bound is the canonical root plus `/` plus `relPath` exceeding 4096 bytes, checked by the function itself. Probe `probe2.mfb` t2 (25 components of 200 bytes): `joined > PATH_MAX -> InvalidArgument: TRUE`. A single over-long *name* is a different case and a different code (probe: `rel long name 300 -> ErrInvalidPath`), which the page gets right.
SUGGESTED: "...or the canonical root joined with relPath is longer than 4096 bytes."

### 5. `root` may be relative, and the page does not say what it is relative to
UNIT:      man-page:fs/openWithin
CLAIM:     "relPath is always interpreted relative to root, never to the process working directory."
VERDICT:   incomplete
EVIDENCE:  Probe `probe.mfb`, run from the scratch directory: `try("root relative cwd", "root", "a.txt", "read")` printed `root relative cwd -> ok`. `realpath` resolves a relative `root` against the working directory, so the working directory does affect the result through `root`. The sentence as written invites the reading that the working directory never matters.
SUGGESTED: "relPath is always interpreted relative to root, never to the process working directory; a relative root is itself resolved against the working directory before use."

### 6. `./` and a trailing `/` on relPath are unaddressed sharp edges
UNIT:      man-page:fs/openWithin
CLAIM:     "The path to open, relative to root. Rejected if it is empty, absolute, contains a .. component, or traverses a symbolic link at any component."
VERDICT:   incomplete
EVIDENCE:  Probe `probe.mfb`: `rel dot slash -> ok` (`./a.txt` opens), `rel dot only -> ErrInvalidPath` (`.` names the root directory itself), `rel trailing slash on file -> ErrInvalidPath` (`a.txt/`), `rel ..b (not dotdot) -> ErrPathNotFound` and `rel ... (three dots) -> ErrPathNotFound` (neither is a `..` component, so they are looked up as names). Only an exact `..` component is rejected; `.` components are passed through to the host.
SUGGESTED: Append to the parameter description: "A `.` component is passed through unchanged; only an exact `..` component is rejected, so `..b` and `...` are ordinary names. A relPath that names a directory, including `.`, raises ErrInvalidPath."

### 7. Error-precedence: `mode` is checked after `root` and `relPath`
UNIT:      man-page:fs/openWithin
CLAIM:     "The mode string is matched exactly, byte for byte, and is case sensitive; any other value is rejected before the file is touched."
VERDICT:   incomplete
EVIDENCE:  `gen_open.rs:lower_fs_open_within_helper` canonicalizes `root` with `realpath` and validates `relPath` before the mode literal compare. Probe `probe2.mfb` t3: `openWithin(R & "/nope", "a.txt", "bogus")` raises `ErrPathNotFound`, not `ErrInvalidArgument`; t4: empty relPath with bad mode raises `ErrInvalidArgument`. "Before the file is touched" is true, but the root directory *is* resolved first, and a bad mode with a missing root reports the root's error.
SUGGESTED: "...any other value is rejected before the file is touched, though root is resolved first: a root that does not exist reports ErrPathNotFound even when mode is also invalid."

### 8. `"r"` named where both spellings apply
UNIT:      man-page:fs/openWithin
CLAIM:     "Any other refusal raises ErrReadFailed for "r" and ErrWriteFailed for a mode that writes."
VERDICT:   misleading
EVIDENCE:  `gen_open.rs:lower_fs_open_within_helper` maps `"r"` and `"read"` (and the omitted default) to the same `flags.read`, and `FsFallback::ByAccess { read_only: flags.read }` keys on that flag value, not on the spelling.
SUGGESTED: "Any other refusal raises ErrReadFailed for a read open (mode omitted, "read" or "r") and ErrWriteFailed for a mode that writes."

Everything else on the page checked out against the binary: all three examples compile and run from `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-openWithin/proj` with outputs `hi`, `TRUE`, `hello`; `probe.mfb` confirmed empty root/relPath, absolute, `..` at start/middle/end, every bad or case-variant mode, and embedded NUL in root, relPath or mode all raise `ErrInvalidArgument`; missing root and missing file raise `ErrPathNotFound`; root-is-file, file-used-as-directory, directory in read and write mode, and a 300-byte name raise `ErrInvalidPath`; any symlink component raises `ErrAccessDenied`; a symlinked root and a trailing slash on root are accepted. `probe2.mfb` confirmed write truncates (`ab`), readWrite preserves (`Zb`), append appends (`Zb!`), omitted mode reads, and `ls -l` under umask 022 showed the three created files as `-rw-------` (0600). No banned memory vocabulary and no borrow/ownership framing was found on the rendered page.
