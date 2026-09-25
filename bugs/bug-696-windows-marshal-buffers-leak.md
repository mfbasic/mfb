# bug-696: every Windows `os::getEnv*`/`hasEnv` call leaks 256 KiB and every Windows path call leaks 64 KiB — the backend's UTF-16 marshalling buffers are never freed

Last updated: 2026-09-25
Effort: large (3h–1d)
Severity: HIGH
Class: Memory-safety (leak)

Status: Open
Regression Test: `scripts/test-winleak.sh` (box 2230); `marshal_ownership_tests` in `src/target/win_x86_64/code.rs`

The Windows x86-64 backend marshals UTF-8 strings to and from UTF-16 through
scratch buffers it allocates from the arena, and it never frees any of them.
`os::getEnvOr` and `os::hasEnv` allocate three per call (64 KiB wide name,
64 KiB wide value, 128 KiB UTF-8 value), and a path operation
(`fs::fileExists`) allocates a 64 KiB wide path. A program that reads an
environment variable or touches a file in a loop grows without bound: 100
`getEnvOr` calls hold 26 MB.

It is silent and Windows-only: output is correct, every other target frees its
(far smaller) marshalled copies (bug-574, bug-575), and only the Windows
`--debug` report shows it.

**Correct behavior after the fix:** each marshalling buffer is freed once the
call that needed it has returned and its result has been copied out, so the
programs below end with the same `alloc_calls - free_calls` whatever the loop
count, as they do on macOS and Linux.

References:

- bug-574 / bug-575 (`bugs/completed/`): the same leak class for runtime-helper
  and `tls::` marshalling, fixed on the POSIX side.
- Found while cross-checking bug-689 on Win11 (box 2230): the bug-689 probe
  ended with 262,144 bytes live on Windows only — its single `os::getEnvOr`.

## Failing Reproduction

`bugs/repro/bug-696-windows-marshal-buffers-leak.mfb`, built
`mfb build -q --debug -target windows-x86_64` at bug-689's tree (`aa644f908` +
bug-689) and run on box 2230 (Win11 x86-64) with `set W=<mode>&& p.exe`:

| W | 100 calls of | `alloc_calls` | `free_calls` | `live_bytes` |
| --- | --- | --- | --- | --- |
| none | nothing | 7 | 4 | 262,144 |
| env | `os::getEnvOr("PATH", "")` | 407 | 104 | **26,476,544** |
| has | `os::hasEnv("PATH")` | 307 | 4 | **26,476,544** |
| fs | `fs::fileExists(…)` | 207 | 104 | **6,815,744** |

(`none` is the program's own one `getEnvOr("W", …)`: 3 blocks, 256 KiB.)

- Observed: 3 blocks / 262,144 B live per `getEnvOr`/`hasEnv`, 1 block /
  65,536 B per `fileExists`.
- Expected: the same live count for 1 call and 100.

## Root Cause

`src/target/win_x86_64/code.rs` allocates through `arena_alloc_to_slot`,
`emit_marshal_path` and five direct `branch_link(ARENA_ALLOC_SYMBOL)` sites —
over 25 allocations across its emitters (`grep -n
"arena_alloc_to_slot(from\|emit_marshal_path(\|branch_link(crate::codegen::error::constants::ARENA_ALLOC_SYMBOL)"`)
— and contains no `_mfb_arena_free` call at all. `emit_env_get` is the measured case:
it allocates `WNAME_SLOT`, `WVAL_SLOT` and `U8VAL_SLOT`, returns the UTF-8
buffer pointer as the `getenv`-style result, and the shared consumers
(`lower_get_env` in `src/codegen/builtins/os/gen_env.rs`, `lower_has_env` in
`func_has_env.rs`) copy or test it and return — none of the three is released.
The comment on `arena_alloc_to_slot` explains why the OOM tag is not checked,
not who frees the block; nobody does.

## Goal

- The reproduction's `env`, `has` and `fs` rows show the same `live_bytes` as a
  1-iteration run.

### Non-goals (must NOT change)

- The marshalling itself (UTF-16 conversion, the 32,767-character env limit,
  the `getenv`-style NUL-or-pointer contract `emit_env_get` hands its callers).
- Every non-Windows target.

## Blast Radius

To audit site by site — every allocation in
`src/target/win_x86_64/code.rs`: `emit_env_get` (3 buffers), `emit_env_set`,
`emit_marshal_path` (every path-taking `fs::` operation), `emit_os_wide_string`,
`emit_dir_path_query`, `emit_realpath`, `emit_mkstemps`, `emit_opendir`,
`emit_readdir`/`emit_read_dir_entry`, `emit_build_argv_utf8` (once per process —
likely intended to live) and the rest. For each: who reads the buffer after
the emitter returns, and where the free belongs (in the emitter, or in the
consumer after it has copied the result).

## Fix Design

Free each buffer at the first point nothing reads it: the wide inputs right
after the Win32 call, a UTF-8 result after the consumer's copy (for
`emit_env_get`, the consumer's `build_string_from_cstr` / presence test). The
consumers are vreg-allocated helpers, so a pointer kept across their calls is
preserved. Right-size where it is cheap (a path needs `2 × (len + 1)` bytes, not
64 KiB), which also removes most of the peak.

## Phases

### Phase 1 — failing test + audit

- [x] A Windows-executed runtime check of the reproduction:
      `scripts/test-winleak.sh <mfb>` runs 23 modes (one per seam, and one per
      shared consumer of a returned buffer) with 1 and 25 calls on box 2230 and
      requires equal `arena.0.live_bytes`. RED at `e4e8b0a85`: all 23 leaked,
      from 8,192 B/call (`appDataPath`) to 393,216 B/call (`openWithin`), and
      `environ` produced no report at all (see Finding 1). Plus two static tests
      in `src/target/win_x86_64/code.rs` (`marshal_ownership_tests`), a
      path-sensitive alloc-minus-free walk over each seam's instructions: RED on
      13 scratch-only seams and 6 result-returning ones.
- [x] The per-site audit — every allocation in `code.rs`, who reads it last, and
      where it is now freed:

| Seam | Buffer(s) | Last reader | Freed |
| --- | --- | --- | --- |
| `emit_marshal_path` (exists, stat, fs op, open, rename ×2, opendir, realpath) | wide path, now `2 × (len + 1)` | the seam's Win32 call | by each seam, after the call |
| `emit_dir_path_query` (currentDirectory, tempDirectory) | 64 KiB wide out | the seam's conversion | the seam |
| `emit_realpath` | wide in + 64 KiB wide out | the seam | the seam (the input pointer moves off `[0x38]`, which the conversion's 8th stack argument reuses) |
| `emit_verify_nofollow` / `emit_verify_within` | 3 wide buffers each | the seam | the seam (`verify_within`'s frame grew 0x70 → 0x80 for the answer slot) |
| `emit_mkstemps` | wide template, sized once | the retry loop | the seam, on both exits |
| `emit_env_set` | wide name, wide value | `SetEnvironmentVariableW` | the seam; the value on its own arm |
| `emit_env_get` | wide name, 64 KiB wide value; UTF-8 value | seam; **consumer** | seam; `lower_get_env` / `lower_has_env` via `emit_release_marshalled` |
| `emit_os_wide_string` (host, user, exe) | 4 KiB wide; UTF-8 answer | seam; **consumer** | seam; `lower_os_wide_string_windows`, `lower_app_resource_path` |
| `emit_known_folder_query` | UTF-8 answer (wide is `CoTaskMem`) | **consumer** | `lower_host_dir`'s Windows `Join` arm; the seam on a failed conversion |
| `emit_environ_pointer` | pointer array + one string per entry → now ONE block | **consumer** | `lower_environ` |
| `emit_opendir` | wide pattern; `DIR` | seam; `readdir` loop | seam; `emit_closedir` (and `emit_opendir` when the search fails) |
| `emit_build_argv_utf8` | argv array + one UTF-8 string per argument | `os::args`, for the process's life | never — each string now sized to its argument instead of 128 KiB |

Every result buffer is now sized to its content (a measuring
`WideCharToMultiByte`) and every buffer carries an 8-byte size header
(`MARSHAL_HEADER`), so a consumer frees it without tracking the size.

Commit: `05296eca3` (the `marshal_ownership_tests`), `ab490d531`
(`scripts/test-winleak.sh`)

### Phase 2 — the fix

- [x] Seams free their scratch; results are one sized block the consumer
      releases (`CodegenPlatform::emit_release_marshalled`, called from
      `emit_release_marshalled_result` at each consumer's single exit with the
      result registers carried across).
- [x] Finding 1 (below) fixed by the `emit_environ_pointer` rewrite.
- [x] Finding 2 fixed in `emit_env_get` (+ the `SetLastError` import).
- [x] Finding 3 fixed in `lower_fs_list_directory_helper`.
- [x] Finding 4: the spec's `getEnvOr` sentence corrected.

Commit: `05296eca3` (seams, consumers, Findings 1–2), `8eb5f7031` (Finding 3),
`de1e8ff38` (spec, Finding 4)

### Phase 3 — goldens + validation

- [x] Regenerate the goldens the change moves; confirm only the intended change.
      `artifact-gate.sh <mfb> all`: 9 of 2142 moved, each localized per function
      against a compiler built at `e4e8b0a85` — `fs::listDirectory` alone on
      macOS/Linux; on Windows only the helpers that inline the reworked seams
      (`fs` path members, `os` env/introspection members and `_main`'s argv
      build, and the `fs` helpers `http`/`thread`/`app_mouse_surface` embed).
      After merging `main` (`505e9930e`, then `d55df2748`): 0 diffs over 2142.
- [x] The reproduction on 2230 now reads, for every mode, `alloc_calls ==
      free_calls` and `live_bytes 0` (`none` 7/7, `env` 407/407, `has` 307/307,
      `fs` 207/207 — before: 26,476,544 / 26,476,544 / 6,815,744 live).
      `test-winleak.sh`, `test-winprocess.sh` and `test-winapp.sh` pass on 2230.
- [ ] Full suite — **partial**. The first full `cargo test --no-fail-fast` (pre-
      `main`-merge) completed: every target green except
      `codegen_helper_scratch_release` (fixed in `d264ddf05`: the listDirectory
      row moved as intended, and the `os` table was already red at `e4e8b0a85`)
      and `rt_compress_bounds::decode_time_is_linear_in_output_size`, a timing
      ratio that also fails at `e4e8b0a85` under load (2 of 3 there, 0 of 3 here
      in alternating runs) and whose program compiles byte-identically before and
      after. After the second `main` merge (`d55df2748`, which folded bug-694's
      `emit_realpath` rewrite into these seams) the full run was stopped at the
      user's request; the fs/os/Windows subset ran instead, all green: 18 test
      targets (205 tests: every `rt_fs_*`, `rt_scope_drop_leaks`,
      `rt_debug_arena`, `codegen_helper_scratch_release`, the
      `codegen_win64_*`/`cli_windows_*`/`cli_process_windows_build` targets,
      `rt_process_spawn_env_replace_bogus_environ`,
      `inplace_self_update_census`) and the `--bin mfb` filters `win_x86_64`
      (40), `fs_declared_errors` (3, bug-694's C6 regression included),
      `builtins::os`, `platform_failure`, `platform_hooks`, `field_kind_census`;
      plus `test-winleak.sh` and a `canonicalPath` probe on 2230 (a missing path
      still raises `ErrNotFound` through the frees).

Commit: `d264ddf05` (scratch-release table), `427d04a4a` (9 `.ncodesum`),
`505e9930e` and `d55df2748` (merges of `main`)

## Findings along the way

1. **`os::environ` crashed every Windows program that called it** — measured at
   `e4e8b0a85`: one call exits `-1073741819` (`0xC0000005`) on 2230, before any
   output. The old `emit_environ_pointer` kept its loop state in the fixed
   `SCRATCH` pool, whose Win64 realizations include the callee-saved
   `rbx`/`rsi`/`rdi`, and staged `_mfb_arena_alloc`'s alignment argument in
   `SCRATCH[1]` instead of `ARG[1]`. Which of the two faulted was not isolated;
   the rewrite removes both (only `ARG[0..3]` and frame slots), and `environ`
   now runs 25 times with the right values (`test-winleak.sh`'s `environ` and
   `values` rows).
2. **A Windows variable set to `""` read as unset** — measured on 2230:
   `os::setEnv("X", "")` then `os::hasEnv("X")` → `FALSE`, `os::getEnvOr("X",
   "FALLBACK")` → `"FALLBACK"`, while `os::environ()` listed it and macOS
   answers `TRUE` / `""`. `GetEnvironmentVariableW` returns 0 for both; only
   `GetLastError() == ERROR_ENVVAR_NOT_FOUND` means unset, and the call does not
   clear the last error on success. Now cleared first and checked.
3. **`fs::listDirectory` leaked its List on every target** when the directory
   went away between its two scans: the second `opendir` failing jumped to the
   error tail after the List was allocated. The List now rides the same
   `done`-exit release as the marshalled path. Not reproduced at runtime (it
   needs the directory removed between two consecutive `opendir`s); fixed by
   construction.
4. **`mfb spec stdlib os` said `os::getEnvOr` cannot tell unset from `""`** —
   wrong: it returns `""`, not the fallback (measured on macOS). Corrected.

## Validation Plan

- Runtime proof: the table above, on 2230, before and after.
- Full suite + `artifact-gate.sh <mfb> all`.

## Summary

Contained to one backend file and two consumers; the work is the per-site
ownership audit and Windows-only verification.
