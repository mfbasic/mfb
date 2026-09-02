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
- VERIFIED on box 2230 through the implemented arm
  (`scripts/test-winprocess.sh`, e1–e6, 47/47 assertions ok):
  - (a) an UNSORTED ANSI block is accepted — every case below passes one.
  - cwd: `spawn(["cmd.exe","/C","cd"], "C:\\Windows", …)` → `e1:C:\Windows`;
    the empty string inherits → `e2:C:\mfbproc` (the harness's own directory).
  - replace, map `{MFBPROBE:=one}` → the child's whole `set` output is
    `COMSPEC=…`, `MFBPROBE=one`, `PATHEXT=…`, `PROMPT=$P$G` — no `PATH`, no
    `SystemRoot`, nothing else inherited.
  - (b) **a replace block lacking `SystemRoot` runs fine**, and the three
    variables that do appear are `cmd.exe`'s own: it synthesizes
    `COMSPEC`/`PATHEXT`/`PROMPT` after it starts. The `ComSpec` spelling in the
    merge case versus `COMSPEC` here is the tell — the merged child inherits the
    parent's mixed-case spelling, the replaced one gets cmd's uppercase
    synthesis. Nothing is injected on our side; documented in the DESC rather
    than papered over, exactly as the Open Decision recommended.
  - replace with an EMPTY map: the two-NUL block is accepted (`e4:rc=0`) and
    `MFBPROBE` is absent.
  - merge: `e5:MFBPROBE=two`, `e5:MFBOTHER=three`, and the inherited set
    survives (`e5:SystemRoot=C:\WINDOWS`, `e5:Path=…`).
  - merge with a case-variant key `path`: `e6:path=MFB-OVERRIDE` is present and
    `e6:Path=` is **absent** — one entry, the map's. A byte-exact skip would
    have handed the child both.

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

- [x] Verify the map-entry layout constants (key/value offset+length names)
      against a decoded map block; record them here. Recorded in Measured
      populations; the working precedent is `gen_unix.rs`'s
      `emit_child_apply_env`, which walks `0..capacity` and skips entries whose
      `FLAGS & USED` is 0 — a Map is sparse, unlike the List the spawn argv
      walk iterates `0..count`.
- [x] Implement the win spawnEnv arm: 4-arg capture, quoted argv (A's
      builder), cwd buffer (empty→NULL), replace-mode block build; call
      `emit_win_spawn_tail(CMD, env, cwd)`.
- [x] Add `"process.spawnEnv"` to `src/target/win_x86_64/mod.rs`.
- [x] Re-point `tests/rt_trapped_call_capability_gate.rs` to
      `os.resourcePath` as its remaining real gap (its header already names
      it); assertions unweakened. Done in plan-119-B, which had to re-point it
      anyway and discovered `process.spawnEnv` was never a valid vehicle.
- [x] Box probes in `scripts/test-winprocess.sh`: cwd (`cmd /C cd`), replace
      (`cmd /C set` → contains map entries; `PATH` absent), empty-map replace,
      empty cwd inherits. e1–e4; transcripts in Verified properties.
- [x] `cli_process_windows_build.rs`: spawnEnv program's nplan imports
      (existing set — replace mode needs no new Win32 import). The test also
      exercises the three-argument `send`/`sendBytes` and the stream-selected
      `poll`/`receive`/`receiveBytes`, because the same missing force-emit
      broke all five (see Corrections).
- [x] Added: fix the force-emit exclusion in
      `src/codegen/engine/builder/mod.rs`. Without it `process.spawnEnv`'s
      helper body is never emitted on Windows and the build dies at link time —
      and the same exclusion had been silently breaking `sendTimeout`,
      `sendBytesTimeout`, `pollFrom`, `receiveFrom` and `receiveBytesFrom`
      since plan-90-D. Reproduced on `main` first. See Corrections.
- [x] Added: delete `unimplemented_on_windows` from `gen_windows.rs`. With
      `shell` and `spawnEnv` implemented it has no callers, and the module doc
      that described "unreachable placeholder arms" was describing something
      that no longer exists.

Acceptance: box probes pass; full `cargo test --no-fail-fast` green.
Commit: —

### Phase 2 — merge-mode env (the risk concentrate)

- [x] Implement the merge walk (GetEnvironmentStringsA, case-folded skip,
      copy + append + terminator, FreeEnvironmentStringsA); nplan test gains
      the two new imports.
- [x] Box probes: merged child sees an inherited var (`SystemRoot`), the
      override wins for a case-variant collision (`set path=X` map key `PATH`
      → exactly one entry, value from the map), and a fresh key appends.
      e5–e6; transcripts in Verified properties.

Acceptance: box probes pass, including the case-collision single-entry
assertion; full suite green.
Commit: —

### Phase 3 — docs

- [x] `func_spawn.rs` DESC: delete the Unix-only paragraph; document per-OS
      mechanics (fork+setenv vs CreateProcess env block), the
      case-insensitive merge rule on Windows, and the replace-mode "only your
      map, including on Windows" consequence observed in Phase 1. The `env`
      parameter's own `desc` said "each key/value set with `setenv`", which is
      Unix-specific and now false as a description of the contract — replaced
      with the case-insensitivity rule a caller can actually act on.
- [x] ~~`planning/todo.md`: retire the note's spawn half.~~ — moot, same
      evidence as plan-119-B: the note exists only in the shared main
      checkout's uncommitted `planning/todo.md`, not on this branch
      (`git show main:planning/todo.md | grep -c` → 0). Reported instead.
- [x] Render gates: `mfb man process spawn`, `scripts/man-census.sh
      --memory-scope`, `scripts/man-run-examples.sh process --run` (host runs
      the Unix path — examples must remain host-runnable). 18/18 ran;
      memory-scope unchanged at 8 pre-existing `canvas` hits.
- [x] Added: the byte-identity `process` cover fixture now builds for
      `windows-x86_64` (it calls `shell` and the four-argument `spawn`, which
      is exactly why it could not before), so it finally gets a
      `windows-x86_64.ncodesum` golden — the drift sentinel plan-119-A's
      census found missing. The census is now 133 goldens, 0 diffs.

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

- **The premise "compile-time rejected (capability absent)" was false, and what
  it hid was a wider shipped defect.** Corrected in §1 already; the root cause is
  that `validate_capabilities` sees the base `process.spawn`, so the alias never
  faces the gate. Chasing that down led to the real bug:
  `src/codegen/engine/builder/mod.rs` refused to force-emit the synthesized
  `process` overload helpers on Windows —

      let process_synth = platform.family() != PlatformFamily::Windows;

  — on the stated premise that they "are stubs there". That premise was true only
  for `spawnEnv`. `sendTimeout`, `sendBytesTimeout`, `pollFrom`, `receiveFrom`
  and `receiveBytesFrom` have always had real Windows bodies (each `*_win` entry
  fn branches on the runtime-call name exactly as its posix twin does) and
  `win_x86_64` has advertised all five since plan-90-D — so all five were
  advertised, passed validation, and then failed to link. Reproduced on `main`
  with an attribution binary before touching anything:

      $ /tmp/p119-head/target/release/mfb build --target windows-x86_64 synth
      error: native code internal relocation target
             '_mfb_rt_process_process_sendTimeout' is not defined

  The exclusion is deleted. This is outside the letter's stated scope and is
  fixed here anyway: it is a bug found while doing the work, it is in the exact
  seam this letter had to touch, and `spawnEnv` cannot work without the fix.
- **The gate-test re-point happened in plan-119-B, not here.** Letter B had to
  move the vehicle off `process.shell` the moment it implemented it, and
  discovered `process.spawnEnv` was not a usable intermediate. It went straight
  to `os.resourcePath`, so this letter's task was already satisfied.
- **The two phases landed as one commit.** They stayed separate units of work and
  of box verification — cwd and replace mode were written and run on the box
  before the merge walk existed — but they are not separately committable. A
  commit containing Phase 1 alone would build and advertise a four-argument
  `spawn` whose `envReplace = FALSE` branch silently drops the caller's
  environment, which is precisely the silent-wrong-value class this plan family
  exists to remove. Both `Commit:` lines carry that one hash.
- **The `planning/todo.md` task had no target on this branch** — same evidence as
  plan-119-B's correction: the note lives only in the shared main checkout's
  uncommitted `planning/todo.md`, which belongs to a peer session.
- **A golden the family made possible was added.** plan-119-A's census found the
  Windows `process` backend had no byte-identity coverage at all, because the
  cover fixture calls `shell` and the four-argument `spawn`. With both
  implemented the fixture builds for `windows-x86_64`, so
  `tests/byte-identity/process/golden/process_codegen_cover_rt.windows-x86_64.ncodesum`
  now exists and the census is 133 goldens with 0 diffs.

## Summary

The mechanism is a solved API (`CreateProcessA` already takes both pointers,
sitting zeroed in the shipped code); the engineering risk is one hand-emitted
merge walk with case-folded matching, isolated to its own phase behind
flat-walk Phase 1, and every semantic claim is pinned by on-box `set`/`cd`
transcripts rather than compile-side proxies.
