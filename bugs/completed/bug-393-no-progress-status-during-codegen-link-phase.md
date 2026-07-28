<!-- Bug document. See .claude/skills/write-bug/template.md for the schema. -->

# bug-393: codegen+link emits no live status — a long build looks like a hang

Last updated: 2026-07-27
Effort: medium (1h–2h)
Severity: LOW
Class: Footgun

Status: FIXED (a046d715d)
Regression Test: tests/cli_build_progress.rs (new — see Validation Plan)

STATUS: FIXED (a046d715d)

A lightweight `progress: &dyn Fn(&str)` sink was threaded through
`write_executable` — the trait method (`src/target.rs`), the `target::`
dispatcher, and all five backends (win_x86_64, macos_aarch64, linux_aarch64,
linux_x86_64, linux_riscv64). The CLI (`src/cli/build/mod.rs`) passes a closure
that calls the new `Reporter::progress`, which prints `codegen: <stage>` on
stderr at Verbose only and no-ops otherwise, so the level gate lives in one
place and emitted bytes never depend on verbosity. Each backend emits one line
per sub-stage boundary as the doc's Root Cause listed: `lowering module`,
`planning + regalloc`, `emitting native code`, `encoding image`, `linking
executable`.

Verified:
- New regression test `tests/cli_build_progress.rs` — RED before the fix
  (missing `codegen: lowering module`), GREEN after; asserts the ordered
  sub-stage lines under `-v`/`--verbose` and their absence at default/`-q`.
- Full `cargo test` suite green (0 failures across all binaries); the existing
  `cli_build_verbosity_output.rs` (including `artifact_bytes_identical_across_
  verbosity_levels`) still passes — no stdout `strip_prefix` test moved.
- Manually reproduced on the exact repro backend: a small project built for
  `windows-x86_64` with `-v` streams all five `codegen:` lines in pipeline
  order, followed by the `phase codegen+link <N>ms` total; default/`-q` emit
  none. The Windows PE (and the macOS `.out`) are byte-identical across
  `-q`/default/`-v`.

Deviations from the plan:
- The chosen wording is the `codegen: <stage>` prefix recommended in Open
  Decisions (distinct from the final `phase …` timing).
- Coarse granularity only (one line per sub-stage), as recommended; the
  per-function counter in `code::lower_module` remains a possible follow-up.
- The doc's own repro (`examples/browser/app` for `windows-x86_64`) could not be
  built as-is here because its local sub-packages (`display`/`dom`/`fetch`) are
  not installed in this checkout — it fails at *resolve*, before codegen, so it
  never reached the instrumented stage. The identical windows-x86_64 backend
  path was exercised with a small project instead; the fix is backend-wide, not
  program-specific.
- Package builds (`target::write_package`) remain silent — explicitly OUT OF
  SCOPE per Blast Radius; a follow-up can extend the same sink.

`mfb build` prints nothing between the front-end `phase` lines and the final
`Wrote executable to …`. On a large program the codegen+link stage can run for
over a minute (observed 69,892 ms building `examples/browser/app` for
`windows-x86_64`), during which the terminal is completely silent. Worse, the
`phase codegen+link <N>ms` line is a **post-hoc** timing — it is printed only
*after* the stage completes — so even at `-v` there is zero indication that the
compiler is alive and making progress. A user cannot distinguish a slow-but-
working build from a hang, and gets no signal about *which* part of codegen is
slow.

The single correct behavior a fix produces: while codegen+link is running under
`-v`/`--verbose`, the compiler emits incremental status as it crosses each named
sub-stage of `write_executable` (lower → plan/regalloc → code emit → encode →
link), so a long build is visibly progressing and the slow sub-stage is
identifiable. The default (Normal) and `-q` (Quiet) output must be unchanged,
and the emitted artifact bytes must be identical regardless of verbosity.

References:

- `src/docs/spec/tooling/07_cli-reference.md` — the `-v`/`-q` verbosity contract
  ("`verify`, `codegen+link` — as a lightweight build profiler").
- Found while answering a user question about the `phase codegen+link 69892ms`
  line during a `windows-x86_64` build of `examples/browser/app`.
- Possibly related root-cause of the *duration* (not this bug's scope):
  arena transient-churn quadratic behavior (see memory `arena-transient-churn-
  quadratic-graphemes`). This bug is about *visibility*, not the runtime.

## Failing Reproduction

```
cd examples/browser/app
../../../target/debug/mfb build -v --target windows-x86_64
```

- Observed (verbatim, `-v`):

  ```
  Building browser (executable) for windows-x86_64
  phase parse 5ms
  phase resolve 21ms
  phase verify 26ms
  phase codegen+link 69892ms        <- printed only AFTER ~70s of silence
  Wrote executable to ./build/browser.exe
  ```

  For ~70 seconds after `phase verify`, the process emits nothing.

- Expected (`-v`): incremental sub-stage lines during codegen+link, e.g.

  ```
  phase verify 26ms
  codegen: lowering module
  codegen: planning + regalloc
  codegen: emitting native code
  codegen: encoding image
  codegen: linking executable
  phase codegen+link 69892ms
  Wrote executable to ./build/browser.exe
  ```

  (Exact wording TBD — see Open Decisions.)

Contrast cases that are correct today and must stay unchanged:

- Default `mfb build` (no `-v`): prints only `Building …` + `Wrote …`. No new
  lines.
- `mfb build -q`: prints only the `Wrote …` artifact line. No new lines.
- The `Wrote executable to …` line stays on **stdout**; all progress stays on
  **stderr** (integration tests `strip_prefix` the stdout artifact line).

## Root Cause

Not a miscompile — a missing-instrumentation gap. In
`src/cli/build/mod.rs`, everything from `let codegen_start = Instant::now();`
(mod.rs:535) through `reporter.phase("codegen+link", codegen_start.elapsed())`
(mod.rs:643) is timed as one opaque block, and `Reporter::phase`
(`src/cli/build/mod.rs:72`) prints exactly once, at the end. The bulk of the
block is a single call to `target::write_executable(…)` (mod.rs:536).

Inside the backend, `write_executable` is already a clean linear pipeline of
named sub-stages that each *could* report but currently do not. For the Windows
backend (`src/target/win_x86_64/mod.rs:264-282`):

1. `lower_validated_module` — IR → neutral module (MIR)
2. `plan::lower_module` + `validate` — native plan / register allocation
3. `os::windows::validate_native_object_plan`
4. `code::lower_module` + `validate` — machine-code emission (per-function over
   the whole program + every `uses` package; usually the hot stage)
5. `crate::arch::x86_64::encode::encode` — assemble image bytes
6. `os::windows::write_linked_executable` — link + write the PE

No `Reporter` (or any progress sink) is threaded into `write_executable`, so
none of these boundaries are observable. The stage is silent by construction.

## Goal

- Under `-v`/`--verbose`, `mfb build` emits at least one status line per
  `write_executable` sub-stage as it is entered, on stderr, so a multi-second
  codegen+link is visibly progressing and the slow sub-stage is named.
- Default and `-q` output are byte-for-byte unchanged from today.
- Emitted artifact bytes are identical across `-q`/default/`-v` (verbosity never
  reaches codegen output, only the CLI's own prints).

### Non-goals (must NOT change)

- **Not** reducing the 70s duration. That is a separate performance
  investigation (candidate: arena transient-churn quadratic). A "fix" that
  merely speeds up the browser example without adding progress output does not
  close this bug.
- **Not** changing artifact bytes, the `Wrote …` stdout line, its channel, or
  its format — integration tests depend on it.
- **Not** adding output at default/Quiet verbosity.
- **Not** rewriting the existing `phase parse/resolve/verify/codegen+link`
  lines' format; the post-hoc `codegen+link` timing line stays (it is the total).
- Tempting wrong fix to forbid: making the new test assert on default output, or
  loosening an existing `strip_prefix` test to "make room" — the new lines are
  stderr + verbose-only precisely so no existing stdout test moves.

## Blast Radius

`write_executable` is implemented per backend; a progress sink must be threaded
into each (or into a shared helper they all call). Found via
`grep -rn "fn write_executable" src/`:

- `src/cli/build/mod.rs:536` (call site) + `:72` (`Reporter::phase`) — fixed by
  this bug; the reporter/callback originates here.
- `src/target.rs:114` (trait method) + `:276` (dispatcher) — signature change;
  fixed by this bug.
- `src/target/win_x86_64/mod.rs:264` — the reproduced backend; fixed by this bug.
- `src/target/macos_aarch64/mod.rs:284` — same pipeline shape; fixed by this bug
  (so `-v` is consistent across hosts).
- `src/target/linux_aarch64/mod.rs:184` — fixed by this bug.
- `src/target/linux_x86_64/mod.rs:189` — fixed by this bug.
- `src/target/linux_riscv64/mod.rs` (`write_executable`) — fixed by this bug.
- Package builds (`target::write_package`, mod.rs:705) — latent, same silence,
  OUT OF SCOPE: package codegen has not been observed slow; keep this bug scoped
  to executables. Note it here so a follow-up can extend the same sink.
- `write_nir` / `write_native_plan` / `write_native_object_plan` /
  `write_native_code_plan` — unaffected: these are artifact-dump paths, not the
  full build, and are not the slow path.

## Fix Design

Thread a lightweight progress callback (not the whole `Reporter`) into
`write_executable`. A `&dyn Fn(&str)` (or a small `Progress` newtype wrapping the
`Reporter`) keeps the backend decoupled from CLI verbosity: the CLI passes a
closure that calls `reporter` at Verbose and is a no-op otherwise, so backends
just call `progress("emitting native code")` unconditionally and the gating
lives in one place (mirrors how `Reporter::phase` already gates a single print).

Granularity: start with **coarse** — one line per sub-stage boundary listed in
Root Cause. This is zero hot-path cost (a handful of calls per build) and
already distinguishes "hung" from "working" and names the slow stage. A
finer per-function counter inside `code::lower_module` (stage 4, the usual hot
one) is a possible follow-up but needs a callback through the emit loop and a
throttle so it does not spam — deferred; see Open Decisions.

Rejected alternatives:

- Passing `&Reporter`/`Verbosity` into every backend — leaks CLI concerns into
  codegen and repeats the gate at each call site. The closure keeps the gate in
  the CLI.
- A background "still working…" heartbeat timer — needs a thread, gives no
  sub-stage information, and would print at default verbosity or not at all.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/cli_build_progress.rs`: build a small fixture project with
      `-v` and assert the stderr contains the sub-stage lines in order; add a
      companion assertion that default and `-q` stderr do NOT contain them and
      stdout is unchanged. Confirm it fails today (no sub-stage lines emitted).
- [x] Confirm the blast-radius list above against `grep -rn "fn
      write_executable" src/` at HEAD; record any backend missed. (All five
      backends confirmed; the `os::*::link::write_executable` matches are the
      unrelated low-level linker fn, correctly untouched.)

Acceptance: the new test fails only because the sub-stage lines are absent (the
default/`-q`/stdout assertions already pass); audit list complete.
Commit: a046d715d

### Phase 2 — the fix

- [x] Add a progress-callback parameter to the `write_executable` trait method
      (`src/target.rs:114`) and dispatcher (`:276`); the CLI (`src/cli/build/
      mod.rs:536`) passes a closure that reports at Verbose, no-ops otherwise.
- [x] Emit one `progress(...)` call at each sub-stage boundary in all five
      backends (win_x86_64, macos_aarch64, linux_aarch64, linux_x86_64,
      linux_riscv64), on stderr, wording agreed in Open Decisions.

Acceptance: Phase 1 test passes; default/`-q` output unchanged; artifact bytes
unchanged; nothing in Non-goals changed.
Commit: a046d715d

### Phase 3 — full validation

- [x] Run the full `cargo test` suite; confirm no stdout-`strip_prefix`
      integration test moved. (0 failures; `cli_build_verbosity_output.rs`
      unchanged.)
- [x] Rebuild for `windows-x86_64` with `-v` and confirm live sub-stage lines
      appear, with the `phase codegen+link <N>ms` total still printed after.
      (The `examples/browser/app` repro needs its local sub-packages installed —
      it fails at resolve before codegen — so the identical windows-x86_64
      backend path was exercised with a small project; all five lines stream in
      order.)
- [x] Diff a built artifact `-q` vs `-v` to confirm byte-identical output.
      (Windows PE and macOS `.out` both byte-identical across `-q`/default/`-v`.)

Acceptance: full suite green; artifact bytes identical across verbosities; the
original reproduction now shows live progress.
Commit: a046d715d

## Validation Plan

- Regression test: `tests/cli_build_progress.rs` — `-v` shows ordered sub-stage
  lines on stderr; default/`-q` do not; stdout artifact line unchanged.
- Runtime proof: the browser-example `-v` build shows incremental codegen lines
  instead of ~70s of silence.
- Doc sync: update `src/docs/spec/tooling/07_cli-reference.md` `-v` description
  to mention the codegen sub-stage progress lines.
- Full suite: `cargo test` + the codegen artifact gate (byte-identity unchanged,
  since verbosity never touches emitted bytes).

## Open Decisions

- Exact line wording/prefix — `codegen: <stage>` vs. extending the `phase …`
  vocabulary. Recommend a distinct `codegen: …` prefix so it reads as live
  progress, not a final timing. (§Fix Design)
- Coarse-only now vs. also a per-function counter in `code::lower_module`.
  Recommend coarse now; file the counter as a follow-up if stage 4 dominates.
  (§Fix Design)

## Summary

Low-risk visibility fix: the codegen+link stage is already a clean linear
pipeline of named sub-stages, but no progress sink is threaded into
`write_executable`, so a minute-plus build is indistinguishable from a hang and
the slow stage is unnamed. The engineering care is entirely in *not* perturbing
existing output — new lines are stderr + verbose-only, artifact bytes and the
stdout `Wrote …` line stay byte-identical — not in the mechanism, which is a
callback threaded through five backends.
