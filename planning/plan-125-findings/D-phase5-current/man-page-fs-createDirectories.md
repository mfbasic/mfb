### 1. `ErrPathNotFound` is not a race-only error
UNIT:      man-page:fs/createDirectories
CLAIM:     "`ErrPathNotFound` means a directory could not be created because its parent is missing — which, since the walk creates every parent, happens only when something removes one while the call runs."
VERDICT:   wrong
EVIDENCE:  A prefix that is a symbolic link pointing at nothing raises it with no concurrency at all. The prefix walk (`gen_directory/create.rs:lower_fs_create_directories_helper`, the `prefix_ok` arm) treats `mkdir` answering `EEXIST` as "accepted" without looking at what exists, so a dangling link passes as a prefix; the next `mkdir` under it answers `ENOENT`, which `gen_shared.rs:emit_fs_host_error_mapping` maps to `ErrPathNotFound`. Probe (`probe.mfb` prints `e.code` and `e.message` from a `TRAP`, path from `$P`), fixture `ln -s $F/nowhere $F/dangling`:
```
--- dangling-prefix: P=.../fx/dangling/sub
ERR 77030001 Filesystem path does not exist.
--- dangling-prefix-mid: P=.../fx/dangling/a/b
ERR 77030001 Filesystem path does not exist.
```
SUGGESTED: "`ErrPathNotFound` means a directory could not be created because its parent is missing: a prefix of `path` is a symbolic link that points at nothing, or something removed a directory the walk had just created while the call was running."

### 2. Symbolic links at the final component are a sharp edge the page omits
UNIT:      man-page:fs/createDirectories
CLAIM:     "A final `path` that exists but is not a directory — a regular file — raises `ErrAlreadyExists`, as `fs::createDirectory` does."
VERDICT:   incomplete
EVIDENCE:  The final-component check is `mkdir` → `EEXIST` → `stat` (which follows links) → "is it a directory?" (`create.rs`, the `emit_path_stat`/`emit_stat_is_kind` block). So a symbolic link to a directory succeeds quietly, while a link that points at nothing, or a link loop, raises `ErrAlreadyExists` even though nothing usable is there. Probe output:
```
--- linkdir-final: P=.../fx/linkdir        (ln -s $F/realdir)
after: dirExists=TRUE
--- dangling-final: P=.../fx/dangling      (ln -s $F/nowhere)
ERR 77050005 Create operation conflicts with an existing item.
--- loop-final: P=.../fx/loopA             (loopA -> loopB -> loopA)
ERR 77050005 Create operation conflicts with an existing item.
```
A prefix that is a link to a directory is also followed (`linkdir-prefix` → `after: dirExists=TRUE`). The page never says links are followed.
SUGGESTED: "Symbolic links are followed: a final `path`, or a prefix, that is a link to a directory counts as that directory. A final `path` that exists but is not a directory — a regular file, or a link that leads nowhere — raises `ErrAlreadyExists`, as `fs::createDirectory` does."

### 3. `ErrInvalidPath` conditions are under-listed
UNIT:      man-page:fs/createDirectories
CLAIM:     "`ErrInvalidPath` that `path` cannot name a directory: a prefix of it is a file, or a name in it is too long."
VERDICT:   incomplete
EVIDENCE:  `emit_fs_host_error_mapping` also sends `ELOOP` (symbolic-link loop) and `EILSEQ` to `ErrInvalidPath`, and "a prefix is a file" is really "a prefix is anything that is not a directory" (a device node too). Probe:
```
--- loop-prefix: P=.../fx/loopA/sub
ERR 77030002 Filesystem path string is invalid for the host platform.
--- devnull: P=/dev/null/x
ERR 77030002 Filesystem path string is invalid for the host platform.
--- file-prefix: P=.../fx/file/sub
ERR 77030002 ...
--- long: P=.../fx/aaaa(300 a's)
ERR 77030002 ...
```
SUGGESTED: "`ErrInvalidPath` that `path` cannot name a directory: a prefix of it exists but is not a directory, a link in it loops back on itself, or a name in it is too long."

### 4. Windows separator and drive/share claims could not be run here
UNIT:      man-page:fs/createDirectories
CLAIM:     "On Windows `\` separates components too, and a leading drive (`C:`) or network share (`\\server\share`) is skipped the same way, so `fs::createDirectories("C:\data\logs\2024")` … work."
VERDICT:   UNVERIFIED
EVIDENCE:  No Windows host in this sandbox. The lowering agrees with the sentence by reading: `emit_windows_create_directories_start` skips `X:` plus one separator, or `\\server\share` plus one separator, or one leading separator, and the scan loop also branches on byte 92 (`\`) when `platform.family() == Windows`. The "already exists" test there is `ERROR_ALREADY_EXISTS` (183). Not a finding against the text; recorded so the main thread knows it rests on code reading, not a run.
SUGGESTED: none; needs a Windows run.

Everything else on the page checked out against the binary. Probes under `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-createDirectories` (`run.sh`, `ex1.mfb`, `ex2.mfb`, `nul.mfb`, `probe.mfb` + bash fixtures in `fx/`): both examples build and run, exit 0, the second printing `TRUE` and both cleaning up after themselves; empty path and a path with an embedded NUL (built with `strings::fromScalars([`\0`])`, length 25) both raise 77050002 `ErrInvalidArgument`; a regular file at the final component raises 77050005 `ErrAlreadyExists`; a file as a prefix raises 77030002 `ErrInvalidPath`; a 0500 parent raises 77030003 `ErrAccessDenied`; a read-only HFS+ image (`hdiutil attach -readonly`, Python confirms errno 30 `EROFS`) raises 77020002 `ErrWriteFailed`, confirming the "every other refusal" fallback; the walk leaves already-created prefixes behind on failure (`pp/n1/n2` existed after the over-long final name failed); re-running on an existing directory succeeds; trailing `/`, doubled `//`, `.`/`..` components, a relative path (created under the working directory) and a Unicode name all succeed; and the mode claim holds: with `umask 022` every created level is `755`, with `umask 077` every level is `700`, matching the constant `493` (`0o755`) passed to `mkdir` in `src/target/macos_aarch64/code.rs`. No internals or banned memory vocabulary on the page, and no sentence teaches an ownership model.
