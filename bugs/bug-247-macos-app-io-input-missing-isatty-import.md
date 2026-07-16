# bug-247: macOS `-app` build of a program using `io::input` fails with "runtime helper requires _isatty import"

Last updated: 2026-07-15
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: <tests/* filename — to be added in Phase 1>

Building any macOS app-mode executable (`mfb build -app`, `NativeBuildMode::MacApp`)
for a program that calls `io::input` aborts codegen with:

```
error: runtime helper requires _isatty import
```

App-mode `io.input` is lowered to `emit_app_io_input_helper`, which *composes*
the unchanged console `io.readLine` body (`_mfb_rt_io_io_readLine`) to read the
window input pipe. That readLine body probes the tty via `isatty` (→ `_isatty`
on macOS) and `tcgetattr` (→ `_tcgetattr`), but the macOS plan only declares
those two symbols in the import row for programs that call `io.readLine`/
`readChar`/`readByte` *directly*. A program that calls only `io.input` never
triggers that row, and the macOS `app_mode_imports()` list never force-declares
the terminal-probe symbols, so `emit_libsystem_call` cannot find `_isatty` in
the platform-imports map and hard-errors. **The single correct behavior a fix
produces: a macOS `-app` build of a program that calls `io::input` (and no
`io::readLine`) links and runs, prompting and reading a line from the app
window, with no missing-import error.**

References:

- `plan-04-macos-app.md` §5.4 (app-mode `io.input` composes `io.write` +
  `io.readLine`), §6.5 (`app_mode_imports`).
- Sibling backend that already handles this correctly:
  `src/target/linux_gtk/mod.rs:749` (force-declares `read`/`isatty`/`tcgetattr`/
  `tcsetattr` in app-mode imports with an explanatory comment).
- Found while running `../../target/debug/mfb build -app` in `examples/audio`
  (its `main.mfb` calls `io::input("Device >")`).

## Failing Reproduction

```
cd examples/audio
../../target/debug/mfb build -app
```

`examples/audio/src/main.mfb` calls `io::input` (device selection prompt) and
never calls `io::readLine`.

- Observed: `error: runtime helper requires _isatty import` (build aborts, no
  `.app` produced).
- Expected: the build succeeds and produces the `.app` bundle; at runtime the
  program prompts in the window and reads the typed line.

Minimal reproduction (no audio dependency):

```
' minimal.mfb
LET name AS String = io::input("Name > ")
io::print("Hi " & name)
```

```
mfb build -app minimal.mfb   # fails identically on macOS-aarch64
```

Contrast cases that work today (bound the bug):

- Console build of the same program (`mfb build`, no `-app`): the `io.input`
  console body restores cooked mode with `tcsetattr` only and does **not** call
  `isatty`/`tcgetattr`, so its import row (`_read`,`_write`,`_fsync`,`___error`,
  `_tcsetattr`) is sufficient — builds fine.
- macOS `-app` build of a program that ALSO calls `io::readLine` directly: the
  `io.readLine` import row runs and declares `_isatty`/`_tcgetattr` — builds
  fine.
- Linux GTK `-app` build of the same `io::input`-only program: immune — its
  `app_mode_imports()` already force-declares `isatty`/`tcgetattr` (and
  `read`/`tcsetattr`) precisely for this composition (mod.rs:743-750).

| Environment | Config | Result |
| --- | --- | --- |
| macOS-aarch64 | `-app`, program uses only `io::input` | fails ✗ |
| macOS-aarch64 | console (no `-app`), `io::input` | works ✓ |
| macOS-aarch64 | `-app`, program also uses `io::readLine` directly | works ✓ |
| linux-x86_64 GTK | `-app`, program uses only `io::input` | works ✓ |

## Root Cause

Two layers disagree about what the app-mode `io.input` build actually emits.

- **Code layer forces the readLine body.**
  `src/target/shared/code/mod.rs:1063-1073` — when `build_mode.is_app()` and
  `_mfb_rt_io_io_input` is used, it appends `_mfb_rt_io_io_write` and
  `_mfb_rt_io_io_readLine` to `runtime_symbols` so their bodies are emitted.
  The composed `_mfb_rt_io_io_readLine` body is produced by
  `lower_io_read_line_helper` (io_helpers.rs:1673); because `with_prompt` is
  false for the `io.readLine` spec, it runs `emit_configure_stdin_terminal`
  (io_helpers.rs:1812-1825), which emits an `isatty` libc call
  (io_helpers.rs:823) → `_isatty` on macOS, followed by `tcgetattr` →
  `_tcgetattr`.

- **Plan layer only declares imports for calls present in the NirOps.**
  Import collection is driven per runtime-call target
  (`platform_imports_for_runtime_call`, symbols.rs:441-449). A program that
  calls only `io::input` triggers only the `io.input` import row in
  `src/target/macos_aarch64/plan.rs:257-316`. That row's `spec.call ==
  "io.input"` branch (lines 263-288) declares `_read`,`_write`,`_fsync`,
  `___error`,`_tcsetattr` — but **not** `_isatty` or `_tcgetattr`. Those live
  in the `else` branch (lines 289-315), reached only by `io.readLine`/
  `readChar`/`readByte`. Nothing force-declares them for the composed body:
  macOS `app_mode_imports()` (plan.rs:81-145) lists AppKit/Obj-C/libSystem
  symbols but omits the terminal probes.

Result: `_isatty` (and `_tcgetattr`) are referenced by emitted code but absent
from the `platform_imports` map, so `emit_libsystem_call`
(macos_aarch64/code.rs:783-786) returns `Err("runtime helper requires _isatty
import")`. `_isatty` is emitted before `_tcgetattr`, so it surfaces first.

The console build is immune because its `io.input` body genuinely does not call
`isatty`/`tcgetattr`. The Linux GTK app build is immune because its
`app_mode_imports()` already force-declares these symbols for exactly this
composition.

## Goal

- A macOS `-app` build of a program that calls `io::input` but not
  `io::readLine` completes codegen without the missing-import error and links
  against `_isatty`/`_tcgetattr` (and any other terminal-probe symbols the
  composed readLine body references).

### Non-goals (must NOT change)

- The console-mode `io.input` import row: it must keep NOT importing `_isatty`/
  `_tcgetattr` (the console body doesn't call them; adding them there would be
  dead imports).
- The runtime semantics of app-mode `io.input`/`io.readLine` (still reads the
  window input pipe; the terminal probes are no-ops on the non-tty pipe).
- The set of emitted helper bodies (mod.rs:1063-1073 composition is correct;
  the bug is in imports, not emission).
- Tempting wrong fix, explicitly forbidden: making `emit_configure_stdin_terminal`
  skip the `isatty` call in app mode to dodge the import. The readLine body is
  the *shared console* helper and must stay byte-identical; the fix is to
  declare the imports the emitted code actually uses, mirroring the Linux GTK
  backend — not to alter the shared body or mask the probe.

## Blast Radius

Backends whose app mode composes the console readLine body from `io.input`:

- `src/target/macos_aarch64/plan.rs:app_mode_imports` — **fixed by this bug**
  (missing `_isatty`/`_tcgetattr`; `_read`/`_write`/`_tcsetattr`/`___error`
  already arrive via the `io.input` row but should be considered for
  robustness).
- `src/target/linux_gtk/mod.rs:app_mode_imports` (~line 743-750) — **unaffected**,
  already force-declares `read`/`isatty`/`tcgetattr`/`tcsetattr` for this exact
  composition.
- Other native backends (`linux_aarch64`, `linux_x86_64`, `linux_riscv64`) —
  **unaffected**: app mode on those targets is GTK (handled above); the plain
  console backends never take the app-mode `io.input` composition path.

Related but distinct (out of scope): app-mode `io.readChar`/`io.readByte`
composition, if any, on macOS — audit in Phase 1 to confirm they either arrive
through their own import rows or need the same treatment.

## Fix Design

Mirror the Linux GTK backend: force-declare the terminal-probe symbols the
composed console readLine body references in macOS `app_mode_imports()`
(`src/target/macos_aarch64/plan.rs`). At minimum `_isatty` and `_tcgetattr`;
for parity and robustness against future changes to the shared body, also
declare `_read` and `_tcsetattr` (harmless duplicates — `push_platform_import`
de-dups). Add a comment matching the GTK one explaining that app-mode
`io::input` delegates to the console readLine body, whose terminal probes are
no-ops on the fd-0 pipe but whose symbols must still bind.

Rejected alternative: teaching the code-side composition (mod.rs:1063-1073) to
also inject the readLine import row into the plan. That crosses the
code/plan layer boundary (imports are a plan-layer concern computed before
codegen) and is more invasive than the localized `app_mode_imports` list the
GTK backend already established as the pattern.

Expected generated-output shift: the macOS app-mode import table / MachO load
commands gain `_isatty`/`_tcgetattr` (and possibly `_read`/`_tcsetattr` if not
already present) for app builds that use `io::input`. No console-build output
changes.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a test that builds a macOS app-mode program calling only `io::input`
      and asserts codegen succeeds (no "requires _isatty import" error),
      following the existing app-mode / import-planning test conventions.
      Confirm it fails against current behavior.
- [ ] Audit macOS app-mode `io.readChar`/`io.readByte` composition (if reachable)
      and confirm each terminal-probe symbol used by every force-composed body is
      accounted for. Write the verdict per site into the Blast Radius list.

Acceptance: the new test fails with the documented `_isatty` error; the audit
list is complete with a verdict per site.
Commit: —

### Phase 2 — the fix

- [ ] Add `_isatty` and `_tcgetattr` (and, for parity, `_read`/`_tcsetattr`) to
      `src/target/macos_aarch64/plan.rs:app_mode_imports` with an explanatory
      comment mirroring `linux_gtk/mod.rs`.

Acceptance: the Phase 1 test passes; console builds are unchanged; nothing in
Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate any app-mode import/golden snapshots the fix shifts; confirm
      the delta is only the added macOS app-mode terminal-probe imports.
- [ ] Run the project's full acceptance suite.
- [ ] Re-run the original reproduction (`examples/audio` + minimal case) with
      `mfb build -app` on macOS-aarch64 and confirm it builds and runs the
      prompt/read.

Acceptance: full suite green; expected-output deltas are exactly the added
imports; the reproduction builds and runs on macOS where it previously failed.
Commit: —

## Validation Plan

- Regression test(s): the app-mode `io::input`-only build test from Phase 1.
- Runtime proof: `mfb build -app` in `examples/audio` produces a `.app` that,
  when launched, prompts "Device >" and accepts a typed line.
- Doc sync: none expected — `plan-04-macos-app.md` §6.5 already describes the
  composition; the fix only completes the import list to match it.
- Full suite: the project's acceptance / artifact-gate commands.

## Summary

Localized import-planning gap: macOS app mode force-emits the console
`io.readLine` body to implement `io::input`, but only declares that body's
`isatty`/`tcgetattr` imports when `io.readLine` is called directly. The Linux
GTK backend already solved the identical composition by force-declaring the
probe symbols in `app_mode_imports`; the fix ports that one-list change to
macOS. Real risk is near zero (add symbols to an import list); the shared
readLine body and all runtime semantics stay untouched.
