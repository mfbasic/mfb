### 1. Permission-bits sentence is false on Windows
UNIT:      man-page:fs/createDirectory
CLAIM:     "The new directory is requested with permission bits 0755 (rwxr-xr-x), which the host masks with the process umask in the usual way, so the directory's actual mode is 0755 with the umask bits cleared."
VERDICT:   misleading
EVIDENCE:  True on macOS and Linux: `src/target/macos_aarch64/code.rs` and `src/target/linux_common/code.rs` pass immediate 493 (0o755) as the mkdir mode under `FsPathOperation::Mkdir`; probe `probe2.mfb` under `umask 022` produced `drwxr-xr-x fx/modedir` and under `umask 077` produced `drwx------ fx/modedir` (`stat -f '%Sp %N'`). False on Windows: `src/target/win_x86_64/code.rs:2766` maps Mkdir to `CreateDirectoryW` with `lpSecurityAttributes = NULL` (comment at the call, `abi::mfb_arg(1)` set to 0); there is no mode and no umask. Not runnable here, verified by code read only.
SUGGESTED: "On macOS and Linux the new directory is requested with permission bits 0755 (rwxr-xr-x), which the host masks with the process umask in the usual way, so its actual mode is 0755 with the umask bits cleared. On Windows the directory inherits its parent's security attributes; there are no permission bits to request."

### 2. An existing symbolic link, even a broken one, also raises ErrAlreadyExists, and directoryExists does not see it
UNIT:      man-page:fs/createDirectory
CLAIM:     "if anything already exists at path, including an existing directory, the call fails with ErrAlreadyExists rather than succeeding quietly."
VERDICT:   incomplete
EVIDENCE:  Fixture `fx/loop1 -> loop2 -> loop1` (a symlink whose target never resolves). `probe.mfb` case `symlink-loop-final` on path `fx/loop1` printed `code=77050005` (ErrAlreadyExists) and then `exists=FALSE` from `fs::directoryExists` on the same path. The page's second example guards with `directoryExists`, which returns FALSE here, so that guard does not prevent the raise. Mapping: `gen_shared.rs:emit_fs_host_error_mapping` arm `("17", &already_exists)` (EEXIST) for `op.creates()`.
SUGGESTED: "if anything already exists at path — a directory, a file, or a symbolic link, even one whose target is missing — the call fails with ErrAlreadyExists rather than succeeding quietly. A guard with fs::directoryExists does not cover the last case: it reports FALSE for a link with no target, and createDirectory still refuses the name."

### 3. ErrInvalidPath list omits the Windows reserved-character case
UNIT:      man-page:fs/createDirectory
CLAIM:     "ErrInvalidPath if the path cannot name a directory — a name that is too long, a file used as a directory, or a loop of symbolic links."
VERDICT:   incomplete
EVIDENCE:  `gen_shared.rs:emit_fs_host_error_mapping`, Windows arm: `("123", &invalid_path) // ERROR_INVALID_NAME` and `("1921", &invalid_path) // ERROR_CANT_RESOLVE_FILENAME`. A name containing a character Windows forbids (`?`, `*`, `<`, `>`, `|`, `"`, or a stray `:`) raises ErrInvalidPath there, and nothing on the page says so. POSIX arms verified by probe: `file-as-parent` → 77030002, `too-long` (300-byte component) → 77030002, `symlink-loop` → 77030002. Windows path not runnable here; code read only.
SUGGESTED: "ErrInvalidPath if the path cannot name a directory — a component that is too long, a file used as a directory, a loop of symbolic links, or, on Windows, a name containing a character the host forbids."

### 4. Second example repeats its own guard verbatim with no explanation
UNIT:      man-page:fs/createDirectory
CLAIM:     "Guard against re-creating a directory that already exists:" followed by two identical `IF NOT fs::directoryExists(cache) THEN fs::createDirectory(cache) END IF` blocks.
VERDICT:   misleading
EVIDENCE:  `ex2.mfb` built and ran, printing `TRUE`, exit 0, so the example is not broken. But the second IF block is a byte-for-byte copy of the first, and the prose never says the repetition is the point (that the guard makes a second attempt harmless). A reader hunting for the difference between the two blocks finds none.
SUGGESTED: Either delete the duplicate block, or keep it and introduce it: "Run the guard twice to show the second pass is a no-op — the directory is already there, so createDirectory is never called again:". Also state the expected output: "This prints TRUE."

No other findings. Every other claim was checked against the binary. `probe.mfb` and `probe2.mfb` (under `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-createDirectory`) exercised: existing directory and existing file → 77050005; missing parent and `nope/../x` → 77030001; parent with mode 000 → 77030003; file as parent, 300-byte name, symlink loop → 77030002; read-only root (`/mfb-probe-rofs`) → 77020002 ErrWriteFailed; empty path and a path with an embedded NUL (`strings::fromScalars([toScalar(0)])`) → 77050002 ErrInvalidArgument, with nothing created; a Unicode name, a relative name resolved against `fs::currentDirectory`, and a trailing-slash name all succeeded. Both page examples compiled and ran. The banned-word sweep (`scripts/man-census.sh --banned-list` against the rendered page) hit only the derived Errors row "Allocation failed.", which is the §4.4 carve-out.
