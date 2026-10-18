# bug-886: macOS `os::executablePath` / `os::appResourcePath` return the unresolved invocation path — symlinks are not followed and `.`/`..` components survive, so a symlinked program cannot find its own resources

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed (bs24-agent-i)
Coordinator note: repro measured by the review agent only; the coordinator did not re-run it.
Regression Test: tests/rt-behavior/os/func_os_executablePath_resolved_rt (to add)

## Cause (coordinator, bug-staging-24)

- Rule: `os::executablePath` / `os::appResourcePath` return the absolute, symlink-resolved path with no `.`/`..` components (`mfb spec stdlib os`), as Linux already returns from `/proc/self/exe`.
- Where: `src/codegen/builtins/os/gen_paths.rs:emit_executable_path_into` (shared by both, and by the `appResourcePath` self-update prefix in `string_self_update.rs:emit_app_resource_prefix`) returns `_NSGetExecutablePath`'s bytes as-is on macOS.
- Fix: the macOS arm canonicalizes with `realpath()` after `_NSGetExecutablePath`, in the one shared helper, so both members and the cached prefix follow.

On macOS, `os::executablePath()` returns the path `_NSGetExecutablePath` reports,
which is the path the program was **launched by**, not the file it is. Two cases show
the difference. Run through a symlink, it returns the symlink's path. Run as
`../../x/build/prog`, it returns `<cwd>/../../x/build/prog`. `os::appResourcePath`
derives its base from the same bytes. Under a symlink it therefore points into the
symlink's directory, where the resources do not exist. This is the normal install
pattern (`/usr/local/bin/tool -> /opt/tool/build/tool`, Homebrew links). The program
then fails to open bundled resources, and nothing reports an error until the open.

Correct behavior: both calls return the absolute, symlink-resolved path with no `.`
or `..` components, as the spec states. That is `realpath()` of the
`_NSGetExecutablePath` result, which matches what Linux already gets from
`/proc/self/exe`. Then `appResourcePath` finds resources however the program was
launched.

References:

- src/docs/spec/stdlib/14_os.md §"The argument vector": "`os::prog()` … can therefore differ from `os::executablePath()`: the former preserves a relative path, symlink, or launcher spelling, while the latter is the resolved absolute executable path."
- src/docs/spec/stdlib/14_os.md §"Build resources": "It is an absolute path with no `..` segments, so it opens with `fs::open` regardless of the working directory".

## Failing Reproduction

```
mkdir -p /tmp/hardening/B06/osres/src /tmp/hardening/B06/osres/res /tmp/hardening/B06/bin
echo hello > /tmp/hardening/B06/osres/res/data.txt
cd /tmp/hardening/B06/osres
cat > project.json <<'J'
{ "name": "osres", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }], "entry": "main", "targets": ["native"], "resources": [{ "src": "res/data.txt", "dst": "./" }] }
J
cat > src/main.mfb <<'M'
IMPORT io
IMPORT os
IMPORT fs
FUNC main AS Integer
  LET p = os::appResourcePath("data.txt")
  io::print(p & " exists=" & toString(fs::fileExists(p)))
  RETURN 0
END FUNC
M
/tmp/hardening/mfb build -q
./build/osres.out
ln -sf /tmp/hardening/B06/osres/build/osres.out /tmp/hardening/B06/bin/osres
/tmp/hardening/B06/bin/osres
```

- Observed:
  ```
  /private/tmp/hardening/B06/osres/build/data.txt exists=TRUE
  /tmp/hardening/B06/bin/data.txt exists=FALSE
  ```
- Expected: both lines `/private/tmp/hardening/B06/osres/build/data.txt exists=TRUE`.

`..` components (program `/tmp/hardening/B06/os2`, which prints `os::prog`, `os::executablePath`, and `s = os::appResourcePath("")`):

```
cd /tmp/hardening/B06/os2/build && ../../os2/build/os2.out
prog [../../os2/build/os2.out] exe [/private/tmp/hardening/B06/os2/build/../../os2/build/os2.out]
selfarp [/private/tmp/hardening/B06/os2/build/../../os2/build]
cd /tmp/hardening/B06/os2/src && ../build/./os2.out
prog [../build/./os2.out] exe [/private/tmp/hardening/B06/os2/src/../build/./os2.out]
selfarp [/private/tmp/hardening/B06/os2/src/../build/.]
/tmp/hardening/B06/os2link        # symlink to os2/build/os2.out
prog [/tmp/hardening/B06/os2link] exe [/tmp/hardening/B06/os2link]
selfarp [/tmp/hardening/B06]
```

| Environment | Result |
| --- | --- |
| macOS 15 aarch64 | fails ✗ (measured) |
| Linux | not run — `/proc/self/exe` is kernel-resolved, so likely ✓ |
| Windows | not run — `GetModuleFileNameW` returns the loaded image path; `.lnk` is not a symlink, but an NTFS symlink would need checking |

## Root Cause

`src/codegen/builtins/os/gen_paths.rs:emit_executable_path_into` (shared by
`os::executablePath` and `os::appResourcePath`) uses `_NSGetExecutablePath(buf,
&size)` on macOS and returns its bytes as-is. Apple documents that this path "may be
a symbolic link" and need not be canonical. The macOS arm needs a `realpath(buf,
out)` after it, which libc provides with no extra link dependency. The `appResourcePath`
self-update arm `src/codegen/collection/assign/string_self_update.rs:emit_app_resource_prefix`
caches the same prefix, so it is fixed by the same change.

## Goal

- On macOS, `os::executablePath()` equals `realpath` of the image, and `os::appResourcePath(rel)` is `<realpath dir>/<rel>`, for symlinked and relative invocations.

### Non-goals (must NOT change)

- `os::prog()` keeps the raw `argv[0]`.
- The `.app` bundle offset (`Contents/MacOS` → `Contents/Resources`) logic.

## Blast Radius

- `os::executablePath`, `os::appResourcePath` (copying and in-place arms) — fixed by this bug.
- `os::appDataPath` & co. — unaffected (HOME/passwd based; measured).

## Fix

`emit_executable_path_into` (`src/codegen/builtins/os/gen_paths.rs`), macOS arm: `_NSGetExecutablePath`
now writes the raw launch path into the upper part of the existing 4096-byte frame buffer and
`platform.emit_realpath` canonicalizes it into the first 1024 bytes (PATH_MAX), which is the returned
buffer; a NULL `realpath` takes the same `fail` exit. Frame size is unchanged. `os.executablePath` /
`os.appResourcePath` macOS imports gain `DARWIN_REALPATH_SYMBOL` (`target/macos_aarch64/plan.rs`).
The self-update prefix calls the `os.executablePath` helper, so it follows. Spec `stdlib os` and the
`executablePath` man prose updated.

Test: `tests/runtime/rt_executable_path_resolved.rs` — RED on main's compiler (symlink launch returned
the link path), GREEN after (direct, symlinked and `../x/./prog` launches).
Expected golden movement: byte-identity `os` and `general` artifacts for macos-aarch64 only (helper body
and import list changed; Linux/Windows arms untouched), and any fixture whose golden embeds the
macOS `os.executablePath`/`appResourcePath` helper.
Artifact gate: byte-identity/os/os_codegen_cover_rt.macos-aarch64.ncode is the only DIFF (1/7); general clean; 5 accept fixtures clean.
