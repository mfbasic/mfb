### 1. Implementation detail: `access`/`F_OK` sentence is a compiler fact, and false on Windows
UNIT:      man-page:fs/exists
CLAIM:     "The check is implemented with the host `access` call using the existence mode (`F_OK`, `0`); `access` returning `0` maps to `TRUE` and any nonzero result maps to `FALSE`."
VERDICT:   out-of-scope
EVIDENCE:  §3 of `.ai/man-content.md` bans codegen mechanics; the libc call name and mode constant matter only to someone reading `src/target/*/code.rs`. It is also not true of every target: `src/target/win_x86_64/code.rs:emit_path_exists` calls `GetFileAttributesW` and tests bit 31, never `access`. Only `src/target/macos_aarch64/code.rs:emit_path_exists` and `src/target/linux_common/code.rs:emit_path_exists` use `access(path, 0)`.
SUGGESTED: Delete the sentence. The preceding two sentences already state the developer-visible contract (any entry → TRUE, nothing → FALSE).

### 2. Symlink sentence justifies itself with `access` internals, and the Windows backend does not follow links
UNIT:      man-page:fs/exists
CLAIM:     "The final path component is followed when it is a symlink, because `access` dereferences the last component: a symlink pointing at an existing target reports `TRUE`, and a symlink whose target is missing reports `FALSE`."
VERDICT:   out-of-scope (the "because `access` dereferences" clause); UNVERIFIED on Windows
EVIDENCE:  macOS probe (`probe.mfb`, built via `mfb init proj_probe && mfb build proj_probe`): `symlink-ok: TRUE`, `symlink-dangling: FALSE`, so the behavior holds here. On Windows `emit_path_exists` calls `GetFileAttributesW`, which per Microsoft's documentation returns the attributes of the link itself rather than its target, so a dangling symlink would report TRUE there. I have no Windows host to run the probe; NEEDS-WINDOWS-PROBE.
SUGGESTED: "A symlink is followed: one pointing at an existing target reports `TRUE`, and one whose target is missing reports `FALSE`." If the Windows probe shows TRUE for a dangling link, add: "On Windows the link itself counts as an entry, so a dangling symlink reports `TRUE`."

### 3. "unreadable parent directory" names the wrong permission
UNIT:      man-page:fs/exists
CLAIM:     "A failed check — for example a missing path or an unreadable parent directory — is reported as `FALSE` rather than raised as an error."
VERDICT:   misleading
EVIDENCE:  Probe with `sb/nor` at mode 311 (no read, has search) and `sb/nox` at mode 644 (read, no search), each holding `inner.txt`: `noread-parent: TRUE`, `noexec-parent: FALSE`. Read permission on the parent is irrelevant; search (execute) permission is what decides.
SUGGESTED: "A failed check — for example a missing path, or a parent directory you have no permission to search — is reported as `FALSE` rather than raised as an error."

### 4. Trailing slash on a file is a sharp edge the page omits
UNIT:      man-page:fs/exists
CLAIM:     "It returns `TRUE` when an entry exists — a regular file, a directory, a symlink to an existing target, a socket, a FIFO, or a device node — and `FALSE` when nothing exists at `path`."
VERDICT:   incomplete
EVIDENCE:  Same probe: `file-trailing-slash: FALSE` for `sb/f.txt/` although `file: TRUE` for `sb/f.txt`; `dir-trailing-slash: TRUE` for `sb/d/`. An existing regular file named with a trailing separator reports FALSE, which is not "nothing exists at path".
SUGGESTED: Add: "A trailing path separator asks for a directory: `fs::exists("notes.txt/")` is `FALSE` even when the file `notes.txt` exists, while a directory reports `TRUE` with or without the slash."

### 5. The Unicode example cannot show what it claims
UNIT:      man-page:fs/exists
CLAIM:     "Unicode paths are accepted:" (second example)
VERDICT:   misleading
EVIDENCE:  Built and ran the example verbatim (`proj_ex2`): it printed `FALSE`. The file is never created, so the program prints `FALSE` whether or not Unicode names are accepted; the output does not distinguish the claim from its negation, and a reader comparing the output to "accepted" will assume the opposite. My probe that first created `sb/é日😀.txt` printed `unicode: TRUE`.
SUGGESTED: Create the file first so the printed result demonstrates acceptance:
```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET name AS String = fs::pathJoin([fs::tempDirectory(), "é日😀.txt"])
  fs::writeText(name, "")
  io::print(toString(fs::exists(name)))
  fs::deleteFile(name)
END FUNC
```
and state that it prints `TRUE`.

Everything else checked out. The first example builds and prints `found`. The NUL probe (`fs::exists("/tmp\u{0000}x")`) raised `7-705-0002 Argument value is not valid for the requested operation`, matching the ErrInvalidArgument row. Empty path printed `FALSE` with no error, FIFO and `/dev/null` printed `TRUE`, and a relative path resolved against the working directory printed `TRUE`. The page uses no banned memory vocabulary and teaches no ownership model; the only "Allocation" is the derived ErrOutOfMemory message, which is carved out.
