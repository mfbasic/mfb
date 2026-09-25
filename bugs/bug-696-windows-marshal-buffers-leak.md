# bug-696: every Windows `os::getEnv*`/`hasEnv` call leaks 256 KiB and every Windows path call leaks 64 KiB — the backend's UTF-16 marshalling buffers are never freed

Last updated: 2026-09-24
Effort: large (3h–1d)
Severity: HIGH
Class: Memory-safety (leak)

Status: Open
Regression Test: none yet — a Windows-executed runtime test (Phase 1)

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

- [ ] A Windows-executed runtime check of the reproduction (the project runs
      Windows proofs on box 2230; see `.ai/testing-gates.md`).
- [ ] The per-site audit above.

Commit: —

### Phase 2 — the fix

Commit: —

### Phase 3 — goldens + validation

- [ ] Regenerate the `windows-x86_64` goldens; confirm only the added frees.
- [ ] Full suite; the reproduction on 2230.

Commit: —

## Validation Plan

- Runtime proof: the table above, on 2230, before and after.
- Full suite + `artifact-gate.sh <mfb> all`.

## Summary

Contained to one backend file and two consumers; the work is the per-site
ownership audit and Windows-only verification.
