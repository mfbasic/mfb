# plan-47-D: the Win32 filesystem surface

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: plan-47-C (the IAT mechanism and a runnable `.exe`). Strongly prefers
plan-47-S (see §Open Decisions 1). Feature-wide precondition: master §Prerequisites.
Produces: the `fs::*` Windows implementations and `fs.*` in `runtime_calls`.

Implements `fs::*` over Win32 — `CreateFileW`/`ReadFile`/`WriteFile`/`GetFileAttributesW`/
`FindFirstFileW`/`FindNextFileW`/`MoveFileExW`/`GetTempPathW` — plus the UTF-8↔UTF-16
path marshaling every one of them needs.

The single behavioral outcome: a program that creates, writes, reads back, lists,
renames and deletes files produces byte-identical stdout on Windows and linux-x86_64,
including for a path containing non-ASCII characters.

**This is the most method-shaped surface in plan-47** — 17 of its 20 trait methods are
genuine per-OS implementations. It is the only surface where the master's "just add
methods to the Windows `CodegenPlatform`" framing is close to true. But it is not
entirely true: see §2.2.

References (read first):

- `src/target/shared/code/fs_helpers_io.rs`, `fs_helpers_paths.rs`, `fs_helpers_atomic.rs`
  — the shared lowering. `open_flag_set` at `fs_helpers_io.rs:2738`, and the comment at
  `:2739-2743` documenting that its wrong arm has already shipped once.
- `src/target/linux_common/code.rs:302` and `src/target/macos_aarch64/code.rs:38` — the
  two existing implementations of the 17 methods.
- `planning/plan-47-S-raise-the-posix-seam.md` §4.2 — which removes `dirent_name_offset`,
  `dirent_name_length_offset` and `stat_mode_offset` and replaces them with
  `emit_read_dir_entry`/`emit_stat_is_dir`.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-47-C has landed | `ls src/target/win_x86_64/code.rs` | **NOT MET** |
| plan-47-S has landed (else see Open Decisions 1) | `rg -n 'fn emit_read_dir_entry' src/` | **NOT MET** |
| The Win11 box answers | `ssh -p 2230 test@127.0.0.1 true` | **UNVERIFIED — run it** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every row before continuing and again before deciding to stop. If you stop, report all
> three statuses.

## 1. Goal

- 17 `CodegenPlatform` fs methods implemented over Win32 (§2.1).
- UTF-8 ↔ UTF-16 path marshaling, with a documented policy for paths that are not valid
  Unicode.
- `fs.*` advertised in `runtime_calls`; before this lands, an `fs::` program is rejected
  at compile time by 47-C.
- Runtime proof: create/write/read/list/rename/delete round-trips byte-identically
  against linux-x86_64, including a non-ASCII path.

### Non-goals (explicit constraints)

- **No symlink creation, no permissions/mode surface.** Windows `st_mode` has no POSIX
  analog; `emit_stat_is_dir` (from 47-S) answers the only question shared lowering asks.
- **No path-separator translation in the language.** MFBASIC paths stay as the program
  wrote them; Win32 accepts `/` in most APIs. Do not silently rewrite user paths.
- **Do not edit `open_flag_set`'s POSIX arms.** 47-P made it exhaustive; D fills the
  Windows arm only.
- **No `environ`.** `emit_environ_pointer` has no Windows analog
  (`GetEnvironmentStringsW` is the replacement) and belongs with the `os::` surface, not
  here.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| fs-related `CodegenPlatform` methods | **20** | `awk '/pub\(crate\) trait CodegenPlatform/,0' src/target/shared/code/types.rs \| awk '/^}/{exit} /^    fn /{sub(/^    fn /,""); sub(/[(<].*/,""); print}' \| grep -cE 'file\|dir\|path\|stat\|realpath\|rename\|mkstemp\|temp\|environ\|current'` |
| — genuine per-OS emitters this sub-plan writes | **17** | the 20 minus `dirent_name_offset`, `dirent_name_length_offset`, `stat_mode_offset` |
| — POSIX layout constants (removed by 47-S) | **3** | same three |
| `platform.target()` sites in fs shared lowering | 17 | `rg -c 'platform\.target\(\)' src/target/shared/code/fs_helpers_*.rs` |
| — of those, branch-shaped (converted by 47-P) | **5** | `rg -c 'platform\.target\(\)\s*(==\|\.starts_with\|\.contains)' src/target/shared/code/fs_helpers_*.rs` |
| `open_flag_set` call sites | 6 | `rg -c 'open_flag_set\(' src/target/shared/code/` |

The 17 emitters: `emit_open_file`, `emit_read_file`, `emit_close_file`, `emit_sync_file`,
`emit_seek_file`, `emit_rename_path`, `emit_mkstemps`, `emit_temp_directory`,
`emit_opendir`, `emit_readdir`, `emit_closedir`, `emit_realpath`, `emit_path_exists`,
`emit_path_stat`, `emit_fs_path_operation`, `emit_current_directory`,
`emit_environ_pointer` (stubbed — see non-goals).

### 2.2 Where D is *not* method-shaped

Five branch sites in fs shared lowering, which 47-P converts to exhaustive matches and D
must then answer:

| Site | Decision | Windows answer |
|---|---|---|
| `fs_helpers_io.rs:2744` (`open_flag_set`) | `O_*` bit values | `CreateFileW`'s `dwDesiredAccess`/`dwCreationDisposition` are a **different shape entirely**, not different bits — see §3.1 |
| `fs_helpers_paths.rs:922`, `:1039` | dirent `d_namlen` vs strlen | removed by 47-S's `emit_read_dir_entry` |
| `fs_helpers_io.rs:599`, `:938` | `openat2(RESOLVE_NO_SYMLINKS)` nofollow | Windows needs its own whole-path symlink refusal (§3.2) |
| `fs_helpers_io.rs:33` (`write_uses_raw_syscall`) | raw syscall vs libc | `false` for Windows — correct, but it routes Windows onto the **libc-errno EINTR retry path**, which is meaningless on Win32 (§3.3) |

### 2.3 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| 17 of the 20 fs methods are genuine emitters | **CONFIRMED** | the other 3 are the layout constants 47-S removes |
| `open_flag_set`'s wrong arm has shipped before | **CONFIRMED** | `fs_helpers_io.rs:2739-2743` documents linux-x86_64 receiving the macOS bits |
| Windows takes the libc-errno EINTR path | **CONFIRMED** | `write_uses_raw_syscall` is `target == "linux-x86_64"`, so Windows gets `false` |
| Windows has neither `___error` nor `__errno_location` | **CONFIRMED** | it has `GetLastError`; 47-C routes `emit_errno` there |
| `GetFileAttributesW` answers is-dir | **CONFIRMED** | `FILE_ATTRIBUTE_DIRECTORY` in the returned bitmask |
| Round-trip byte-identity on Windows | **UNVERIFIED — this is the acceptance criterion** | proven on the Win11 box |

## 3. Design Overview

Three genuinely new things, then 17 mechanical translations.

### 3.1 Open flags are a different *shape*, not different values

POSIX packs mode into one `O_*` bitmask. `CreateFileW` takes **three separate
parameters**: `dwDesiredAccess` (`GENERIC_READ`/`GENERIC_WRITE`), `dwShareMode`, and
`dwCreationDisposition` (`CREATE_ALWAYS`/`OPEN_EXISTING`/`OPEN_ALWAYS`/`TRUNCATE_EXISTING`).

So `open_flag_set` returning an integer is the wrong seam for Windows, exactly as
`termios_*` was for the console. The Windows arm must return a small struct
(`{access, share, disposition, flags}`) rather than a bitmask, and `emit_open_file`
consumes it. **This is a seam change inside the fs group** — small, but it is the reason
D is not purely additive.

### 3.2 Symlink refusal

`openat2(RESOLVE_NO_SYMLINKS)` has no Win32 equivalent. The closest is
`FILE_FLAG_OPEN_REPARSE_POINT` on `CreateFileW`, which opens the *reparse point itself*
rather than refusing — a different semantic. Decide explicitly (§Open Decisions 2) and
document it; silently degrading a security-relevant nofollow is the worst outcome.

### 3.3 EINTR retry is meaningless

The shared retry construct reads `errno == EINTR` after a short read/write. Win32 calls
do not set `errno` and are not interrupted this way. The Windows arms must not emit the
retry loop — but the loop is in shared code, so this is one more decision 47-P's
exhaustive match will surface. Treat a short `ReadFile`/`WriteFile` as a real short
transfer and loop on *progress*, not on an error code.

### 3.4 Path marshaling

Every path-taking Win32 call is the `W` (UTF-16) variant. Each takes a UTF-8 MFBASIC
string → `MultiByteToWideChar` → stack or arena buffer → call. The `A` variants are
codepage-dependent and must not be used.

**Where design uncertainty concentrates:** §3.1's shape change and §3.4's allocation
question — where does the UTF-16 buffer live for a path of unbounded length, inside a
codegen path that cannot easily allocate? Phase 1 answers both on one method
(`emit_path_exists`, the simplest path-taking call) before the other 16 are written.

**Where correctness risk concentrates:** the directory walk. `FindFirstFileW`/
`FindNextFileW` have a different lifecycle from `opendir`/`readdir`/`closedir` —
`FindFirstFileW` *returns the first entry* rather than just a handle, so a naive
translation either skips or double-reports the first file. Test a directory with exactly
one entry.

**Rejected alternative:** *use the POSIX-compat layer in the UCRT (`_open`, `_read`).*
Rejected: it is a CRT dependency (hard non-goal), it re-introduces the errno model this
plan is removing, and it cannot express `FILE_FLAG_OPEN_REPARSE_POINT`.

## 4. Detailed Design

| Method | Win32 |
|---|---|
| `emit_open_file` | `CreateFileW` (§3.1's struct) |
| `emit_read_file` | `ReadFile` |
| `emit_close_file` | `CloseHandle` |
| `emit_sync_file` | `FlushFileBuffers` |
| `emit_seek_file` | `SetFilePointerEx` |
| `emit_rename_path` | `MoveFileExW` (`MOVEFILE_REPLACE_EXISTING`) |
| `emit_path_exists` / `emit_stat_is_dir` | `GetFileAttributesW` |
| `emit_opendir`/`emit_readdir`/`emit_closedir` | `FindFirstFileW`/`FindNextFileW`/`FindClose` (§ risk) |
| `emit_realpath` | `GetFullPathNameW` |
| `emit_temp_directory` | `GetTempPathW` |
| `emit_mkstemps` | `GetTempFileNameW` + `CreateFileW(CREATE_NEW)` |
| `emit_current_directory` | `GetCurrentDirectoryW` |
| `emit_fs_path_operation` | `DeleteFileW` / `RemoveDirectoryW` / `CreateDirectoryW` |

## Compatibility / Format Impact

- **New:** `fs.*` in the Windows `runtime_calls`; kernel32 gains ~14 imports.
- **Changed (shared):** `open_flag_set`'s return shape (§3.1); the EINTR retry gains a
  Windows arm (§3.3). Both must leave the four existing targets byte-identical.
- **Unchanged:** every other backend's fs behavior; the `fs::` language surface.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — spike: path marshaling and the open-flag shape

- [ ] Implement `emit_path_exists` end to end: UTF-8 → `MultiByteToWideChar` →
      `GetFileAttributesW`. Decide and document where the UTF-16 buffer lives.
- [ ] Change `open_flag_set` to return the §3.1 struct; POSIX arms reproduce today's
      bitmask byte-identically.
- [ ] Runtime: a program testing existence of an ASCII path and a non-ASCII path.

Acceptance: `scripts/artifact-gate.sh` 0 diffs on the four existing targets after the
`open_flag_set` reshape; the existence program is correct on Windows for both paths. If
the UTF-16 buffer has nowhere to live, stop and redesign §3.4.
Commit: —

### Phase 2 — file I/O

- [ ] `emit_open_file`, `emit_read_file`, `emit_close_file`, `emit_seek_file`,
      `emit_sync_file`, and the §3.3 progress-loop.
- [ ] Runtime: write a file, read it back, byte-compare against linux-x86_64.

Acceptance: a write/read round-trip is byte-identical across the two targets, including
a short-read case.
Commit: —

### Phase 3 — paths and directories (highest correctness risk)

- [ ] `emit_realpath`, `emit_temp_directory`, `emit_current_directory`,
      `emit_rename_path`, `emit_mkstemps`, `emit_fs_path_operation`.
- [ ] `emit_opendir`/`emit_readdir`/`emit_closedir` over `FindFirstFileW`.
- [ ] Tests: a directory with **exactly one entry**, one with zero, and one with a
      non-ASCII filename — the first-entry lifecycle (§3) is where this breaks.
- [ ] Decide and document §3.2 symlink refusal.

Acceptance: directory listings match linux-x86_64 exactly for the zero-, one- and
many-entry cases and for a non-ASCII filename; the symlink decision is written down.
Commit: —

### Phase 4 — advertise and prove (largest blast radius last)

- [ ] Add `fs.*` to `runtime_calls`; remove the compile-time rejection.
- [ ] Runtime: the full fs acceptance fixture set on Windows.

Acceptance: every `fs::` fixture that passes on linux-x86_64 produces byte-identical
stdout on Windows.
Commit: —

## Validation Plan

- Tests: per phase. The zero/one/many directory cases are mandatory — `FindFirstFileW`'s
  return-the-first-entry lifecycle is the specific trap.
- Coverage check: the two shared edits (§3.1, §3.3) are byte-identity-gated, and
  `linux-riscv64` has zero goldens (master §Prerequisites row 3). Seed them first.
- Runtime proof: the Win11 box, **byte-comparing** against linux-x86_64 output — not
  "looks plausible". A wrong UTF-16 conversion produces plausible-looking text.
- Doc sync: none expected; `fs::` semantics are unchanged. If a Windows limitation forces
  a documented behavior difference (§3.2), that is a spec change and must be recorded.
- Acceptance: the full suite plus `scripts/artifact-gate.sh` 0 diffs.

## Open Decisions

1. **Land D before or after 47-S.** Recommended **after**: D consumes
   `dirent_name_offset`/`stat_mode_offset`, which S deletes. Landing D first means
   writing those three against a seam that is about to change, then rewriting them.
   If D must go first, budget the rewrite explicitly rather than discovering it.
2. **Symlink refusal semantics** (§3.2). Recommended: implement nofollow via
   `FILE_FLAG_OPEN_REPARSE_POINT` + an explicit reparse-point check that *fails* the
   open, matching POSIX refusal semantics. Alternative: reject nofollow opens on Windows
   with a clear diagnostic. Do **not** silently ignore the flag — it is security-relevant.
3. **Non-Unicode paths.** Windows paths are UTF-16 and may contain unpaired surrogates
   that are not valid UTF-8. Recommended: reject with a clear error at the marshaling
   boundary rather than lossily converting.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **D is 17 emitters, not 20.** Three of the fs-related trait methods
  (`dirent_name_offset`, `dirent_name_length_offset`, `stat_mode_offset`) are POSIX
  layout constants that 47-S removes.
- 2026-07-20 — **D is not purely additive**, contrary to the master's framing. Five
  branch sites in fs shared lowering need Windows answers, and `open_flag_set`'s return
  *shape* has to change (§3.1) because `CreateFileW` takes three parameters where POSIX
  takes one bitmask.

## Summary

The engineering risk is in two places: the `FindFirstFileW` lifecycle, which returns the
first entry and so breaks a naive `opendir`/`readdir` translation (test the one-entry
case), and UTF-16 marshaling, where a wrong conversion produces plausible-looking output
that only a byte-comparison catches.

The design work is §3.1 — open flags are a different shape on Windows, not different
values — which is a small echo of the same lesson 47-S learns at scale.

What is left untouched: `fs::` language semantics, every other backend's fs behavior, and
the permissions/symlink-creation surface, which Windows does not model the POSIX way and
which this sub-plan explicitly does not invent.
