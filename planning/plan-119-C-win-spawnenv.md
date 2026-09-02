# plan-119-C: Four-argument `process::spawn` (spawnEnv) on Windows

Last updated: 2026-09-01
Effort: large (3h–1d)
Depends on: plan-119-A (tail helper + quoting — spawnEnv joins argv through the SAME quoted builder), plan-119-B (family order; also the capability-gate test re-point cascade)

Implement the four-argument `process::spawn` overload
(`args, cwd, env, envReplace` — routed as the `process.spawnEnv` alias) for
`windows-x86_64`. Its Windows arm is `unimplemented_on_windows`
(`func_spawn.rs`); the man page says the form is Unix-only.

**Corrected from the original text (plan-119-B, measured):** it is *not*
"compile-time rejected (capability absent)". `validate_capabilities` sees the
base call `process.spawn`, which `win_x86_64` advertises, so the alias never
faces the capability gate; the build instead fails at link time with

    error: native code internal relocation target
           '_mfb_rt_process_process_spawnEnv' is not defined

Adding `"process.spawnEnv"` to the capability list is therefore documentation of
intent rather than the thing that unblocks the call — what unblocks it is the
helper body. Add the row anyway (the list is the backend's advertised surface,
and `runtime_calls` is read by more than the validator), but do not expect it to
change any diagnostic.

No new OS mechanism is needed: `CreateProcessA` takes the two missing pieces
directly — `lpCurrentDirectory` (stack arg 8, the hard-NULL `0x38` slot) and
`lpEnvironment` (stack arg 7, `0x30`), an ANSI block of
`name=value\0…\0\0`. Unlike Unix (which applies the map in the fork child via
`unsetenv`/`setenv` loops + `chdir` — `gen_unix.rs:224-420`), Windows wants
the environment materialized as one block up front; the merge semantics must
be reproduced by *building* the block.

References:

- plan-119-A (`emit_win_spawn_tail` with the optional env/cwd slots this
  letter fills; quoting builder).
- `src/codegen/builtins/process/func_spawn.rs:140-170` — the overload's
  params/DESC (cwd: empty string = inherit; env: map; envReplace: TRUE = only
  `env`, FALSE = merge over inherited); `:180-486` the posix spawnEnv body
  (argv build + cwd C-string with the leading-NUL "skip chdir" convention).
- Win32: `GetEnvironmentStringsA`/`FreeEnvironmentStringsA` for the inherited
  block on merge.
- Collection layout for walking the `Map OF String TO String`:
  `COLLECTION_OFFSET_*`/`COLLECTION_ENTRY_*` (as the Windows spawn body
  already uses for the args list) — but note the map entry carries key AND
  value offsets/lengths; read the map-entry constants, not the list ones.
- `tests/rt_trapped_call_capability_gate.rs` — after this letter only
  `os.resourcePath` remains of its three named gaps; the vehicle must end on
  that.

## Prerequisites

Family gate in plan-119-A, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-119-A landed | `grep -n emit_win_spawn_tail src/codegen/builtins/process/gen_windows.rs` | MET — `gen_windows.rs:196` |
| plan-119-B landed | `grep -n '"process.shell"' src/target/win_x86_64/mod.rs` | MET — `win_x86_64/mod.rs:290` |

## 1. Goal

- On box 2230: `spawn(["cmd.exe","/C","cd"], "C:\\mfbspike", …)` prints the
  requested directory; `spawn(["cmd.exe","/C","set FOO"], "", map{FOO=bar},
  FALSE)` prints `FOO=bar` while inheriting the rest (probe `set PATH` too);
  with `envReplace=TRUE` the child sees ONLY the map (probe: `set` output
  contains the map entries and `PATH` reports not-defined). `mfb man process
  spawn` no longer says the form is Unix-only.

### Non-goals (explicit constraints)

- Overload routing (`abi_function_aliased`, arg-count selection at
  `builder_values`) is untouched — only the win helper arm gains a body.
- Unix semantics unchanged; the shared DESC describes both platforms.
- ANSI APIs, consistent with the rest of the backend (UTF-16 env is a
  follow-up with `CREATE_UNICODE_ENVIRONMENT`, out of scope).
- Sorting the env block is NOT required (`CreateProcess` accepts unsorted
  ANSI blocks) and is not attempted.

## 2. Current State

- Posix spawnEnv body: captures 4 args from `x0..x3` before any libc call,
  builds cwd C-string (empty → leading-NUL sentinel → child skips `chdir`),
  builds argv, then `emit_spawn_tail` with env-map/replace-flag handling in
  the fork child (`gen_unix.rs:224` doc: `unsetenv` each inherited name when
  replacing — portable `clearenv` — then `setenv` each USED map entry).
- Windows arm: `unimplemented_on_windows("spawn")`.
- Windows capability list: no `process.spawnEnv`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Posix spawnEnv body | `func_spawn.rs:180-486` (306 lines) | read |
| Map-entry layout constants exist for codegen walks | yes, VERIFIED | `src/codegen/error/constants/error_constants.rs:974-989`: `COLLECTION_HEADER_SIZE=40`, `COLLECTION_ENTRY_SIZE=40`, entry fields `FLAGS=0`, `KEY_OFFSET=8`, `KEY_LENGTH=16`, `VALUE_OFFSET=24`, `VALUE_LENGTH=32`, `COLLECTION_ENTRY_FLAG_USED=1`. A Map is walked `0..capacity` skipping entries whose `FLAGS & USED` is 0 — NOT `0..count` as a List is (`gen_unix.rs:emit_child_apply_env` is the working precedent). |
| CreateProcess env/cwd slots | `0x30` / `0x38` | `func_spawn.rs:724-727` |

### Verified properties

- The tail accepts env/cwd (plan-119-A design) — its two slots exist and are
  currently zeroed; passing pointers is the entire integration.
- UNVERIFIED (Phase 1 box probes, before the emission is written):
  (a) `CreateProcessA` + ANSI `lpEnvironment` block with an UNSORTED block —
  documented OK, prove with a hand-run probe if any doubt;
  (b) child behavior with a replace-block lacking `SystemRoot` — expected
  parity with Unix's cleared environment (caller's responsibility), but
  OBSERVE `cmd.exe /C set` under replace to document what actually happens.

## 3. Design Overview

The win spawnEnv body = argv build (A's quoted builder) + two prologues + the
shared tail:

1. **cwd**: copy to a NUL-terminated buffer; empty string → pass NULL (the
   Windows twin of the leading-NUL skip). Slot → tail's `lpCurrentDirectory`.
2. **env block**:
   - `envReplace = TRUE`: size pass over the map (Σ klen+1+vlen+1, +1
     terminator, minimum 2 for an empty map — a block of two NULs), alloc,
     emit `key=value\0` per entry, final `\0`.
   - `envReplace = FALSE` (merge): `GetEnvironmentStringsA` → first pass
     measures the inherited block, skipping any entry whose name matches a
     map key **case-insensitively** (Windows env names are case-insensitive;
     an exact-byte compare would hand the child both `PATH` and `Path`) —
     uppercase-fold both sides byte-wise (ASCII fold; names beyond ASCII are
     out of contract, note in DESC); second pass copies the survivors, then
     appends every map entry, then the terminator;
     `FreeEnvironmentStringsA`.
   Slot → tail's `lpEnvironment` (no `CREATE_UNICODE_ENVIRONMENT`, block is
   ANSI).

**Correctness risk concentrates in the merge**: a hand-emitted two-pass walk
over two variable-length NUL-delimited structures with case-folded matching,
at depth-1 frame discipline. It is the largest hand-emission in the family —
schedule it behind the replace path (which is a single flat walk), and pin it
with box probes that print the whole `set` output. A wrong block is a
*silently* wrong child environment, exactly the class `.ai/compiler.md` warns
about — the box assertions must check presence AND absence.

Byte-identity NOT the gate; new emission only. Gates: box probes + compile
tests.

Rejected: mutating the parent's environment around `CreateProcess`
(`SetEnvironmentVariableA` + restore) — racy against other threads and
observable from the parent; block-building is the documented mechanism.
Rejected: requiring sorted blocks (not needed for ANSI CreateProcess).

## Phases

### Phase 1 — cwd + replace-mode env (flat walks only)

- [ ] Verify the map-entry layout constants (key/value offset+length names)
      against a decoded map block; record them here.
- [ ] Implement the win spawnEnv arm: 4-arg capture, quoted argv (A's
      builder), cwd buffer (empty→NULL), replace-mode block build; call
      `emit_win_spawn_tail(CMD, env, cwd)`.
- [ ] Add `"process.spawnEnv"` to `src/target/win_x86_64/mod.rs`.
- [ ] Re-point `tests/rt_trapped_call_capability_gate.rs` to
      `os.resourcePath` as its remaining real gap (its header already names
      it); assertions unweakened.
- [ ] Box probes in `scripts/test-winprocess.sh`: cwd (`cmd /C cd`), replace
      (`cmd /C set` → contains map entries; `PATH` absent), empty-map replace,
      empty cwd inherits.
- [ ] `cli_process_windows_build.rs`: spawnEnv program's nplan imports
      (existing set — replace mode needs no new Win32 import).

Acceptance: box probes pass; full `cargo test --no-fail-fast` green.
Commit: —

### Phase 2 — merge-mode env (the risk concentrate)

- [ ] Implement the merge walk (GetEnvironmentStringsA, case-folded skip,
      copy + append + terminator, FreeEnvironmentStringsA); nplan test gains
      the two new imports.
- [ ] Box probes: merged child sees an inherited var (`SystemRoot`), the
      override wins for a case-variant collision (`set path=X` map key `PATH`
      → exactly one entry, value from the map), and a fresh key appends.

Acceptance: box probes pass, including the case-collision single-entry
assertion; full suite green.
Commit: —

### Phase 3 — docs

- [ ] `func_spawn.rs` DESC: delete the Unix-only paragraph; document per-OS
      mechanics (fork+setenv vs CreateProcess env block), the
      case-insensitive merge rule on Windows, and the replace-mode "only your
      map, including on Windows" consequence observed in Phase 1.
- [ ] `planning/todo.md`: retire the note's spawn half.
- [ ] Render gates: `mfb man process spawn`, `scripts/man-census.sh
      --memory-scope`, `scripts/man-run-examples.sh process --run` (host runs
      the Unix path — examples must remain host-runnable).

Acceptance: rendered page correct; man gates green; family-standard suite
green (full cargo test, test-accept, artifact-gate, fmt, check
--all-targets).
Commit: —

## Validation Plan

- Tests: compile (`cli_process_windows_build.rs` incl. the two merge-mode
  imports), runtime (`scripts/test-winprocess.sh` spawnEnv matrix on 2230 —
  presence AND absence assertions), the re-pointed capability-gate test run
  against a windows target.
- Runtime proof: the box `set`/`cd` transcripts recorded in this doc.
- Doc sync: Phase 3.
- Acceptance: family-standard gate set (plan-119-A Validation).

## Open Decisions

- **Merge case-folding scope** — ASCII-only fold (recommended: matches the
  practical contract, documented in DESC) vs locale-aware
  (`CompareStringOrdinal` per entry: more calls, more emission, for env names
  that essentially never exceed ASCII). §3.
- Whether replace-mode should implicitly preserve `SystemRoot` — recommended:
  NO (parity with Unix's fully-cleared environment; document the observed
  child behavior instead). Revisit only if Phase 1's observation shows cmd
  itself cannot run without it, in which case document THAT rather than
  silently injecting.

## Corrections

*(fill during execution)*

## Summary

The mechanism is a solved API (`CreateProcessA` already takes both pointers,
sitting zeroed in the shipped code); the engineering risk is one hand-emitted
merge walk with case-folded matching, isolated to its own phase behind
flat-walk Phase 1, and every semantic claim is pinned by on-box `set`/`cd`
transcripts rather than compile-side proxies.
