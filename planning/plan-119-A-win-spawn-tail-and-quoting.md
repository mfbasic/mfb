# plan-119-A: Windows process backend — shared spawn tail + argv quoting fix

Last updated: 2026-09-01
Overall Effort: x-large (1d–3d — the whole plan-119 family: A + B + C)
Effort: medium (1h–2h)
Depends on: nothing

plan-119 makes `process::shell` and the four-argument `process::spawn`
(`process.spawnEnv`) work on Windows. Today both are compile-time rejected
(`native backend does not support runtime call '…'`) because `win_x86_64`
does not advertise them (`src/target/win_x86_64/mod.rs:289-307` lists every
other `process.` capability; the rejection is
`src/target/shared/validate/capabilities.rs:21`), and their Windows entry fns
are `unimplemented_on_windows` placeholders
(`func_shell.rs:233`, `func_spawn.rs:494` for the `process.spawnEnv` arm).

**The research spike proved Windows supports both mechanisms end-to-end** (box
2230, 2026-09-01, via the already-working one-argument `spawn` driving
`cmd.exe`): `spawn(["cmd.exe","/C","echo one & echo two"])` sequenced,
`exit 7` propagated (`rc-b=7`), `echo … | sort` piped, `> file` + `type`
redirected, and `send`/`close`/`receive` streamed stdin through cmd into
`sort` and back sorted — all through the existing `CreateProcessA` + pipes
tail. `CreateProcessA` natively accepts `lpEnvironment` and
`lpCurrentDirectory` (currently hard-NULL at `func_spawn.rs`, the `0x30`/`0x38`
stack slots), so spawnEnv needs no new OS mechanism either.

The spike also **confirmed a shipped correctness bug** this letter fixes: the
Windows spawn joins argv with bare spaces and NO quoting
(`func_spawn.rs:612-616`, the `no_space` separator branch), so the documented
"no splitting" contract is violated — proven on box 2230:
`spawn(["argdump.exe", "a b", "c"])` delivered the child `argc=3`,
`arg=[a]`, `arg=[b]` (an argv-dumping MFB probe). A silent wrong-value class
bug per `.ai/compiler.md`.

This letter: (1) fix the quoting, (2) factor the Windows spawn body's
pipes → `CreateProcessA` → record tail into a reusable helper (the Windows
twin of `gen_unix.rs:437 emit_spawn_tail`) so letters B and C emit only their
command-line/env prologue. No new surface is enabled here.

References:

- `src/codegen/builtins/process/func_spawn.rs:488-820` — the Windows spawn
  body this letter restructures (frame layout comment at :495 documents the
  slot map; the whole body runs at stack-adjust depth 1, no vregs — preserve
  that discipline, `finalize_frame` must not shift the Win64 stack args).
- `src/codegen/builtins/process/gen_windows.rs` — where the shared tail lands.
- `.ai/arch-abi.md` (Win64 traps: `movaps` alignment, arg-bank aliasing,
  c-result vs `return_register()`) — read before touching the body.
- Quoting rules: the MSVCRT/`CommandLineToArgvW` inverse (wrap when the arg is
  empty or contains space/tab/quote; double backslash runs before a quote and
  before the closing wrap-quote; escape `"` as `\"`).
- Box procedure: `.ai/remote_systems.md` (2230), `scripts/test-winapp.sh` as
  the ship+run pattern.

## Prerequisites

(For the family; B and C point here.)

| Must be true | Command | Status |
|---|---|---|
| Windows box reachable | `ssh -p 2230 test@127.0.0.1 "echo ok"` | MET (re-run 2026-09-01, prints `ok`) |
| Cross-build produces a runnable PE | `mfb build --target windows-x86_64` + box run | MET (re-run 2026-09-01: `argdump.exe` + `spawner.exe` both built and executed on 2230) |

## 1. Goal

- On box 2230, `process::spawn(["argdump.exe", "a b", "c"])` delivers the
  child exactly `argc=2`, `arg=[a b]`, `arg=[c]`; and
  `gen_windows.rs` exposes one `emit_win_spawn_tail(...)` that takes a built
  command-line slot (plus optional env-block/cwd slots for letter C) and emits
  pipes + SECURITY_ATTRIBUTES + STARTUPINFOA + CreateProcessA + handle
  hygiene + record stamping — with the one-argument spawn re-emitted through
  it byte-equivalently in structure (same calls, same slots).

### Non-goals (explicit constraints)

- No new capabilities advertised (shell/spawnEnv stay rejected until B/C).
- No behavior change for space-free argv (byte changes are expected from the
  restructure, but call sequence and record layout are identical).
- Stay on the ANSI APIs (`CreateProcessA`) — the whole Windows process
  backend is A-API today; a UTF-16 migration is a different plan.
- The tag-10 resource record layout (`RESOURCE_OFFSET_*`, `PROC_*`) is
  untouched.

## 2. Current State

- One-argument spawn works on Windows (full body at `func_spawn.rs:488-820`):
  sums arg lengths, joins with `' '` (no quoting), three `CreatePipe`s with
  inheritable SA, strips parent-end inheritance via `SetHandleInformation`,
  zeroed STARTUPINFOA with `STARTF_USESTDHANDLES`, `CreateProcessA(NULL, cmd,
  NULL, NULL, TRUE, 0, NULL, NULL, &si, &pi)`, closes child ends + hThread,
  stamps the record (pid in `PROC_STATUS`).
- `process.spawnEnv` arm: `unimplemented_on_windows("spawn")`
  (`func_spawn.rs:494`); shell: ditto (`func_shell.rs:233-240`).
- The 4-arg overload routes as `Body::abi_function_aliased(lower_spawn,
  &["spawnEnv"])` (`func_spawn.rs:139,169`), selected by argument count at
  codegen (comment at :112). Both overloads share `lower_spawn`, which
  branches win/posix (`:100`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Windows spawn body size | 334 lines | `func_spawn.rs:488-820` |
| Capability rows for `process.` on win | 19 (no shell/spawnEnv) | `grep -c '"process\.' src/target/win_x86_64/mod.rs` → 19 |
| Quoting defect repro | argc=3 for `["argdump.exe","a b","c"]` | box 2230 run, 2026-09-01 (probe PEs from `mfb build --target windows-x86_64`) |
| Windows `.ncodesum` fixtures that will churn | **0** | full 132-golden census (`tests/byte-identity/*/golden/*.ncodesum`, each rebuilt for its own target and sha-compared): `unchanged: 132`, zero DIFF. `tests/byte-identity/process` has no `windows-x86_64.ncodesum` at all — the fixture calls `shell` and 4-arg `spawn`, so it cannot build for Windows until B and C land. See Corrections. |

### Verified properties

- The body runs at stack-adjust depth 1 with hand-picked slots precisely so
  `finalize_frame` does not shift the six outgoing stack args — read the
  comment block at `func_spawn.rs:495-520`. The refactor must keep the tail
  inside one `subtract_stack(FRAME)` bracket owned by the caller, or pass the
  frame constants through.
- CRLF: `process::receive` keeps a child's trailing `\r` (no CR handling in
  `func_receive.rs` — `grep -n '\\\\r' → 0`; observed on-box:
  `l1=[one \r] len=6`). Pre-existing, not this letter's scope — recorded so
  B's examples/doc phrasing accounts for it.

## 3. Design Overview

1. **`emit_win_spawn_tail`** (new, `gen_windows.rs`): parameters = symbol,
   frame constants (or the existing constants moved to `gen_windows.rs` and
   shared), the `CMD` slot, `Option<env-block slot>`, `Option<cwd slot>`,
   fail labels. Body = everything from the SECURITY_ATTRIBUTES setup
   (`func_spawn.rs:659`) through record stamping (`:797`), with `0x30`/`0x38`
   loaded from the optional slots instead of hard zeros. The unix twin
   (`emit_spawn_tail`, `gen_unix.rs:437`) is the naming/shape precedent.
2. **Quoting** in the cmdline builder: per-arg, emit the CRT algorithm —
   pass-through when the arg is non-empty and has no space/tab/quote;
   otherwise wrap in `"`, doubling backslash runs before any `"` and before
   the closing quote, and emitting `\"` for embedded quotes. Length pre-scan
   (`sum_loop`) must compute the worst-case quoted length (2 + 2×len bound is
   acceptable — allocate conservatively rather than double-scan).

**Correctness risk**: the depth-1 frame discipline (a spilled vreg or shifted
offset breaks the Win64 stack args silently — `.ai/arch-abi.md`'s exact trap
class). Mitigation: keep the no-vreg style; prove on-box, and the
`cli_process_windows_build.rs` nplan assertions catch missing imports.

Byte-identity is NOT the gate (windows spawn bytes change by design — quoting
+ code motion). Windows `.ncodesum` goldens churn and are re-synced; the gate
is behavioral: the on-box argv probe + the compile tests.

Rejected: quoting only when needed per-arg with exact re-alloc (double scan
for no measured win); switching to `CreateProcessW` here (worthy, separate
plan — every sibling helper is A-API).

## Phases

### Phase 1 — refactor to the shared tail (no quoting change yet)

- [x] `gen_windows.rs`: add `emit_win_spawn_tail` (+ move/share the frame
      constants); re-emit the 1-arg spawn body through it. The joiner came out
      too — `emit_win_build_cmdline` — since letter C needs the same argv build.
      The 334-line body is now 80 lines of prologue + two shared calls.
- [x] Census + re-sync the churned windows `.ncodesum`/goldens
      (`regen-ncodesum.sh`; expect ONLY windows-target, process-using
      fixtures — prove the delta list matches the census). **0 churned**, so
      nothing to re-sync; the census is in Measured populations and the
      *absence* of Windows process coverage is filed in Corrections.
- [x] Ship the spike's probe program (spawn `cmd.exe /C …` matrix) to box
      2230 and re-run: identical output to the recorded spike baseline. Landed
      as `scripts/test-winprocess.sh` (Phase 2's home too) rather than a
      throwaway: `seq:one`/`seq:two`, `exit:rc=7`, `cat:filed`,
      `pipe:apple`/`pipe:banana`, `stdin:apple`/`stdin:banana` — 9/9 ok.
- [x] Added: a standalone argv probe (`argdump.exe` driven by a spawner) run
      BEFORE and AFTER the refactor — byte-for-byte identical output
      (`A:argc=3 / arg=[a] / arg=[b]`), proving the code motion changed no
      behavior. It is the Phase 2 fixture too.

Acceptance: `cargo test --no-fail-fast` green (incl.
`cli_process_windows_build.rs` nplan assertions);
box run reproduces the spike baseline byte-for-byte (`rc-b=7`, sorted
`apple`/`banana`, …).
Commit: —

### Phase 2 — argv quoting

- [ ] Implement the CRT quoting in the cmdline build loop (worst-case length
      pre-scan; wrap/escape emit).
- [ ] Add the argdump quoting probe as a scripted box check: new
      `scripts/test-winprocess.sh` (sibling of `test-winapp.sh`: builds a
      console argdump.exe + a spawner, ships both, asserts `argc=2`,
      `arg=[a b]`, plus a quote-containing and an empty argument case).
- [ ] Re-sync churned windows goldens (same census discipline).
- [ ] Doc sync: `func_spawn.rs` DESC — the "no quoting … interpreted" promise
      now holds on Windows too; note that the joined line is re-parsed by the
      child's CRT using the standard rules.

Acceptance: `scripts/test-winprocess.sh target/release/mfb` passes on box
2230 (argc=2 / `a b` / `c`, quote and empty-arg cases); full
`cargo test --no-fail-fast`, `scripts/test-accept.sh`,
`scripts/artifact-gate.sh all` with re-synced goldens; both-root fmt +
`cargo check --all-targets`.
Commit: —

## Validation Plan

- Tests: `cli_process_windows_build.rs` (existing nplan pins);
  `scripts/test-winprocess.sh` (new, the runtime truth — cargo test never
  executes a PE, per the repo's own lesson in `test-winapp.sh`'s header).
- Runtime proof: the box runs above, recorded in this doc.
- Doc sync: `func_spawn.rs` DESC (Phase 2).
- Acceptance: family-standard: full cargo test, test-accept, artifact-gate
  (re-synced), fmt both roots, `cargo check --all-targets`.

## Open Decisions

- Whether `scripts/test-winprocess.sh` folds into `test-winapp.sh` or stands
  alone — recommended: standalone (console entry path, not app; the two entry
  paths differ by exactly 8 bytes of alignment per the arch lore and deserve
  separate scripts).

## Corrections

- **"Windows `.ncodesum` fixtures that will churn: UNMEASURED" → measured 0.**
  Every one of the 132 `tests/byte-identity/*/golden/*.ncodesum` goldens was
  rebuilt for its own target and sha-compared: zero differ. The reason is not
  that the refactor was byte-identical (it is not — slots moved and label names
  changed); it is that **the Windows `process` backend has no byte-identity
  coverage at all.** `tests/byte-identity/process/golden/` holds
  `linux-{x86_64,aarch64,riscv64}` and `macos-aarch64` sums and no
  `windows-x86_64` one, because the fixture calls `process::shell` and the
  four-argument `process::spawn` unconditionally and both are compile-time
  rejected on Windows today. The drift sentinel this plan family would most
  like to have is the one the family's own subject makes possible — so a new
  task lands in **plan-119-C Phase 3**, once B and C have made the fixture
  buildable for `windows-x86_64`.
- **Phase 1 also factored out the command-line joiner, not just the tail.**
  The plan's §3 scoped `emit_win_spawn_tail` alone and left the joiner inside
  `func_spawn.rs`. But letter C builds the *same* quoted argv (its §3 says so
  explicitly: "argv build (A's quoted builder)"), so leaving it in
  `func_spawn.rs` would have forced C to either duplicate it or reach across
  files into a `func_*.rs`. It is now `emit_win_build_cmdline` in
  `gen_windows.rs`, beside the tail, with its own scratch-slot block
  (`WIN_CMD_*`) and `WIN_CMDLINE_SCRATCH_END` for callers that need more.

## Summary

Pure enabling work: a proven-buggy joiner gets the real quoting algorithm
(box-verified repro → box-verified fix), and the 334-line spawn body becomes
a reusable tail so B and C are each a small prologue. Risk lives entirely in
the depth-1 Win64 frame discipline during the refactor.
