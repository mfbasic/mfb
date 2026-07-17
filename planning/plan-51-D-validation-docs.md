# plan-51-D: Validation, goldens, and doc sync

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-51-C

plan-51-A through C build the AppImage. This sub-plan proves it stays built: golden
coverage for the Linux app artifact shape, a runtime acceptance script for the GTK
boxes, and the spec/man sync that `.ai/specifications.md` obligates.

It is separated rather than folded into A–C for one reason: A–C each land with their
own unit and integration tests and are individually correct, but **none of them
leaves behind a regression net for the artifact as a whole**. Today `linux-app` has
zero golden coverage and no runtime acceptance script — the macOS side has both
(`tests/syntax/app/macos-app-mode-*` and `scripts/test-macapp.sh`). Without this
sub-plan the AppImage works on the day it ships and nothing notices when it stops.

The behavioral outcome: a Linux app-mode regression fails `scripts/test-accept.sh`
on the dev box, and a runtime regression fails `scripts/test-appimage.sh` on the GTK
boxes.

References (read first):

- `scripts/test-accept.sh:203-205,246-259,299-320,426-432` — the golden harness, and
  specifically the rule that **the presence of a `<pkg>.<target>.app.nir` golden is
  what triggers an `-app` build**.
- `scripts/test-macapp.sh` — the macOS runtime acceptance script this mirrors,
  including its watchdog and headless-mode design.
- `tests/syntax/app/macos-app-mode-{io,plumbing,term}/golden/` — the existing app
  golden fixtures.
- `tests/linux_app_mode.rs` — the cross-build artifact-shape tests, which A–C each
  amend.
- `src/target/macos_aarch64/app/mod.rs:71` — `MFB_MACAPP_HEADLESS`, the precedent for
  §4.2's headless question.
- `src/target/linux_gtk/mod.rs:10-23` — the threading/finish contract; the worker
  parks in `pause()` so the window stays open, and `_exit`s only when headless.
- `.ai/specifications.md` — the spec-currency obligation.
- `.ai/remote_systems.md` — the boxes.

## 1. Goal

- `tests/syntax/app/` carries `linux-x86_64` app goldens, so a change to Linux
  app-mode NIR/nplan/ncode shows up as golden churn on the dev box.
- `scripts/test-appimage.sh` builds a real AppImage, ships it to a GTK box, runs it,
  and checks the observable result — the Linux counterpart to
  `scripts/test-macapp.sh`.
- Every spec page that describes app-mode output, the artifact set, the CLI flags, or
  the Linux RUNPATH matches the compiler.
- The four `term::` man pages that say `mfb build --app` still describe a real thing.

### Non-goals (explicit constraints)

- **No new compiler behavior.** This sub-plan adds tests, a script, and prose. If it
  finds a bug, the fix belongs in A, B, or C — or a `bug-NN` — not here.
- **No GUI-pixel assertions.** `tests/gtk_term_utf8_grid.rs` already establishes the
  house position: assert on the code plan, not on rendered glyphs. The GTK VM has no
  reachable X server and `term::` crashes under headless broadwayd; glyph rendering
  stays manually verified.
- **No CI wiring.** The GTK boxes are developer infrastructure, not CI. §3.3 records
  why `test-appimage.sh` is opt-in rather than part of `test-accept.sh`.
- **No macOS golden churn.** Any movement in a `macos-aarch64` golden means A–C leaked
  and is a bug, not an expected update.
- **No riscv64 coverage.** App mode is unsupported there (plan-51-A §3.3).

## 2. Current State

### 2.1 Linux app mode has zero golden coverage

`scripts/test-accept.sh:246-259` decides per fixture whether to build `-app` by
looking for a golden:

> the presence of a `<pkg>.<target>.app.nir` golden file is what triggers
> `mfb build -q $target_arg -app …`

A fixture carries either console or app goldens for a given extension, never both.
The three existing app fixtures — `tests/syntax/app/macos-app-mode-{io,plumbing,term}/`
— each carry `<pkg>.macos-aarch64.app.{nir,nplan,ncode}` plus `build.log`. **There are
no `linux-*.app.*` goldens anywhere.** So today, every Linux app-mode codegen change
is invisible to `scripts/test-accept.sh`.

The mechanism is the good news: dropping a `<pkg>.linux-x86_64.app.nir` into an
existing fixture lights up Linux app goldens with no harness change.
`scripts/sync-goldens.sh <exe> <name-glob>` is filter-aware and takes ~4s.

### 2.2 There is no Linux runtime acceptance script

`scripts/test-macapp.sh` (macOS only) builds a real `.app`, runs
`Contents/MacOS/<name>` under `MFB_MACAPP_HEADLESS=1` behind a 15s perl watchdog, and
checks exit codes and stdout. Real-window GUI cases (System Events keystroke
injection) are opt-in via `MFB_MACAPP_GUI=1`.

**There is no Linux equivalent.** `tests/linux_app_mode.rs` cross-builds and inspects
artifacts but never executes — the dev/CI host is macOS. So Linux app mode has, today,
no automated execution anywhere.

### 2.3 The spec describes an artifact set that will be wrong

`src/docs/spec/architecture/08_artifacts.md:46-49`:

| `build/<name>.out` | `mfb build` executable (macOS) | Native executable (Mach-O). |
| `build/<name>-glibc.out` | `mfb build` executable (Linux) | Native executable (ELF, glibc). |
| `build/<name>-musl.out` | `mfb build` executable (Linux) | Native executable (ELF, musl). |
| `build/<name>.app` | `mfb build --app` (macOS) | Application bundle. |

**Linux `--app` output is not in the table at all** — `build/<name>.out` for a Linux
app build has never been documented. plan-51-A/C do not merely change a row; they fill
a hole that predates them.

`src/docs/spec/tooling/07_cli-reference.md:130` lists `--app` with no `--app-debug`
sibling; `:147-151` describes the gates but not the output shape.
`src/docs/spec/app/02_linux-runtime.md` describes the GTK runtime whose app id and
title stop being constants in plan-51-A §4.5.

## 3. Design Overview

Three independent pieces:

1. **Golden coverage** (§4.1) — add `linux-x86_64` app goldens to an existing app
   fixture. Harness change: none.
2. **A runtime acceptance script** (§4.2) — `scripts/test-appimage.sh`, modeled on
   `test-macapp.sh` but shipping the artifact to a remote box, because the dev host
   cannot execute Linux binaries and **cannot emulate them either** (plan-51-C §4.6).
3. **Doc sync** (§4.3) — the pages `.ai/specifications.md` obligates.

The risk here is not correctness but **false confidence**. A golden that pins the
wrong thing, or an acceptance script that reports success because the app crashed
after printing, is worse than no coverage: it converts an unknown into a wrong known.
§4.2's exit-code and marker design is about that.

### 3.1 Which fixture gets Linux goldens

`tests/syntax/app/macos-app-mode-plumbing` is the right one. It exercises the app
entry, the worker thread, and the io seam without depending on `term::`'s grid — the
part most likely to differ between AppKit and GTK for reasons that are not
regressions. Adding `<pkg>.linux-x86_64.app.{nir,nplan,ncode}` there gives the
broadest coverage per golden.

The fixture name says `macos-app-mode-*`, which becomes a lie once it carries Linux
goldens. Rename to `app-mode-plumbing` — the harness keys off the golden filename's
target component, not the directory name, so the rename is free. Doing it now is
cheaper than explaining it forever.

Rejected: **a new `linux-app-mode-*` fixture.** It would duplicate the source for no
gain and would let the two drift, which is exactly what a shared fixture with
per-target goldens prevents.

### 3.2 Why x86_64 and not aarch64 goldens

One target's goldens, not both. The shared lowering is target-neutral by construction
(plan-34-D: zero physical registers in every shared stream), so a second target's
goldens would mostly re-pin the same bytes and double the churn on every codegen
change. `linux-x86_64` is the better single choice: it is the arch with the GTK box
that most users have, and plan-00-H notes x86 needs a different `__libc_start_main`
trampoline than aarch64, so it is the one with backend-specific app code to pin.

`tests/linux_app_mode.rs` already cross-builds `linux-aarch64` and asserts on its
artifacts, so aarch64 is not uncovered — it is covered by assertions rather than
goldens, which is the right trade for the second target.

### 3.3 Why the acceptance script is opt-in

`scripts/test-accept.sh` runs on the dev box and must stay hermetic and fast.
`test-appimage.sh` needs SSH to a specific box that may be off. Making it part of the
default acceptance run would mean a red bar for infrastructure reasons, which trains
people to ignore red bars.

`scripts/test-macapp.sh` is already a separate script for the same reason, and its
GUI cases are further gated behind `MFB_MACAPP_GUI=1`. This mirrors that shape.

## 4. Detailed Design

### 4.1 Golden coverage

- Rename `tests/syntax/app/macos-app-mode-plumbing` → `tests/syntax/app/app-mode-plumbing`
  (§3.1); the existing `<pkg>.macos-aarch64.app.*` goldens move unchanged.
- Generate `<pkg>.linux-x86_64.app.{nir,nplan,ncode}` + `build.log` with
  `scripts/sync-goldens.sh`.
- Leave `macos-app-mode-io` and `macos-app-mode-term` macOS-only. The io seam and the
  `term::` grid are the two most platform-divergent surfaces; pinning them on Linux
  buys churn, not signal, and `tests/gtk_term_utf8_grid.rs` already covers the grid's
  code plan.

⚠️ **The `build.log` is the sensitive one.** plan-36 notes `mfb build -q` was
specifically chosen to keep golden churn at zero, and ~439 `build.log` goldens (47% of
935) exist. A Linux app `build.log` records `Wrote executable to …`, which is exactly
what plan-51-A and plan-51-C each change (`.out` → `.AppDir` → `.AppImage`). Adding
this golden **before** A and C land would create churn those sub-plans then have to
re-sync. Hence the ordering: this sub-plan depends on plan-51-C, and the golden is
generated once, against the final shape.

### 4.2 `scripts/test-appimage.sh`

Modeled on `scripts/test-macapp.sh`, with one structural difference: the artifact has
to travel.

```text
scripts/test-appimage.sh [--box <port>] [--gui]

  1. mfb build --app -target linux-x86_64  (or -aarch64, per box)
  2. scp build/<name>.AppImage to the box
  3. ssh: chmod is already 0755; run it
  4. assert: exit code, stdout markers
  5. ssh: rm the artifact
```

Default boxes: `2228` (Ubuntu x86_64 GTK) for x86_64, `2226` (Debian 12 GTK) for
aarch64. Both from `.ai/remote_systems.md`; both are the only boxes with GTK4, and app
mode is glibc-only so the musl boxes are out by construction.

**Headless.** macOS has `MFB_MACAPP_HEADLESS=1` (`macos_aarch64/app/mod.rs:71`) which
skips the window and event loop. The GTK backend has an fd fallback when headless and
`_exit`s the worker rather than parking it in `pause()`
(`linux_gtk/mod.rs:10-23`) — so an equivalent path exists, but it is reached by GTK
failing to open a display rather than by an explicit env var. **This needs to be
checked against the code, not assumed**: if `_mfb_gtkapp_main` aborts on a missing
`$DISPLAY` before reaching the fallback, the script needs `xvfb-run` on the box
instead. §5 Phase 2 resolves it by testing, and the answer determines whether the
script needs a display at all.

**The watchdog is not optional.** `test-macapp.sh` uses a 15s perl watchdog because a
GUI app that fails to start does not exit — it hangs. A hung `ssh` in an acceptance
script is a wedged terminal. Same 15s bound, applied to the remote command.

**Assert on markers and exit code, not on the absence of a crash.** The GTK finish
contract parks the worker in `pause()` so the window stays open after the program
ends — meaning a successful *program* does not produce a successful *process exit* in
windowed mode. The script must assert the program's own stdout marker, and treat the
watchdog firing as expected in `--gui` mode and as failure in headless mode. Getting
this backwards yields a script that passes when the app crashes immediately (no
window → no hang → clean exit).

**Cases:**

| case | asserts |
| --- | --- |
| hello | stdout marker + exit 0 |
| vendored library | the lib loads from inside the image, no `LD_LIBRARY_PATH` |
| `--appimage-extract-and-run` | the no-FUSE path works |
| `--appimage-extract` | the extracted tree matches a `--app-debug` AppDir |
| `--gui` (opt-in) | a real window appears; watchdog fires as expected |

### 4.3 Doc sync

`.ai/specifications.md` forbids the spec contradicting the compiler. Owed:

| page | change |
| --- | --- |
| `src/docs/spec/architecture/08_artifacts.md:46-49` | add `build/<name>.AppImage` (`mfb build --app`, Linux) and `build/<name>.AppDir` (`--app-debug`). **Linux `--app` output is absent today** (§2.3) — this fills a pre-existing hole. Prose at `:54-55` names only the macOS bundle. |
| `src/docs/spec/tooling/07_cli-reference.md:130` | `--app-debug` row in the flag table |
| `src/docs/spec/tooling/07_cli-reference.md:147-151` | output shape per target; `--app-debug` implies `--app`; it is accepted-but-inert on macOS (plan-51-C §4.7) |
| `src/docs/spec/tooling/01_project-manifest.md` | `icon` now applies to Linux app builds, and the 1024×1024 rule is shared |
| `src/docs/spec/app/02_linux-runtime.md` | the artifact is an AppImage; the app id is `dev.mfbasic.<name>` and the title is the project name, neither a constant |
| `src/docs/spec/app/spec.md` | app-mode overview: the two output shapes |
| `src/docs/spec/linker/07_linux-aarch64.md`, `08_linux-x86_64.md` | the app-mode `DT_RUNPATH` is `$ORIGIN/../lib`, not `$ORIGIN/vendor` |
| `src/docs/spec/language/17_native-libraries.md` | vendored libs land in `usr/lib/` inside the AppImage for app builds |
| `src/docs/man/builtins/term/{sync,moveTo,terminalSize}.txt`, `term/package.md` | each says "app mode (`mfb build --app`)"; verify still accurate — they describe behavior, not artifacts, so likely no change. **Check, do not assume.** |

`.ai/compiler.md` and `.ai/remote_systems.md` gain a line each: the GTK boxes are the
app-mode proof surface, and AppImages cannot be tested under emulation.

Man pages follow `.ai/man_template.md` exactly, and the authoring rules live in
`scripts/update_man.sh` — use the driver, do not hand-edit, if any page actually
changes.

## Compatibility / Format Impact

None. Tests, a script, and prose. The one observable change is a fixture directory
rename (`macos-app-mode-plumbing` → `app-mode-plumbing`), which is internal to
`tests/` and which the harness tolerates because it keys off golden filenames rather
than directory names (`scripts/test-accept.sh:246-259`).

## Phases

### Phase 1 — Doc sync

Lands first: it is pure prose against already-shipped behavior (A–C are done), and it
is the obligation most likely to be dropped if left last.

- [ ] Update every page in §4.3's table.
- [ ] Verify the four `term::` man pages against the shipped behavior; run
      `scripts/update_man.sh` if any changes.
- [ ] Add the GTK-box / no-emulation note to `.ai/compiler.md` and
      `.ai/remote_systems.md`.

Acceptance: `mfb spec` renders; no page describes `build/<name>.out` as a Linux app
output or `dev.mfbasic.app` as the GTK app id; the artifact table lists both new
artifacts.
Commit: —

### Phase 2 — Runtime acceptance script

- [ ] Determine whether the GTK app reaches its headless fd fallback without a
      display, or needs `xvfb-run` (§4.2). Test on box 2228; the answer shapes the
      script.
- [ ] Add `scripts/test-appimage.sh` per §4.2: build, ship, run, assert, clean up,
      with a 15s watchdog and `--box` / `--gui` flags.
- [ ] Add the vendored-library, extract-and-run, and extract-vs-AppDir cases.

Acceptance: `scripts/test-appimage.sh` passes against box 2228 and box 2226; it
**fails** when pointed at a deliberately broken AppImage (truncate the squashfs by one
byte) rather than passing or hanging.
Commit: —

### Phase 3 — Golden coverage (last: it pins the final shape)

- [ ] Rename `tests/syntax/app/macos-app-mode-plumbing` → `app-mode-plumbing` (§3.1).
- [ ] Generate `<pkg>.linux-x86_64.app.{nir,nplan,ncode}` + `build.log` with
      `scripts/sync-goldens.sh`.
- [ ] Confirm the `macos-aarch64` goldens in that fixture are byte-identical
      post-rename.

Acceptance: `scripts/test-accept.sh` green with the new goldens; reverting any
plan-51-A §4.5 codegen change (the GTK app id) fails a golden rather than passing
silently.
Commit: —

## Validation Plan

- **Tests:** the goldens themselves are the test. The negative case is Phase 3's
  acceptance criterion — a golden that does not fail when the thing it pins changes is
  not coverage. Verify by reverting the app-id change and confirming red.
- **Runtime proof:** Phase 2's script is the runtime proof, and its own acceptance
  criterion is that it fails on a corrupted artifact. A green acceptance script that
  cannot go red is the specific failure this sub-plan exists to prevent (§3).
- **Doc sync:** Phase 1 is the doc sync.
- **Acceptance:** `scripts/test-accept.sh` (expect new `linux-x86_64.app.*` goldens
  and zero movement in any `macos-aarch64` golden — macOS churn means A–C leaked),
  `scripts/artifact-gate.sh`, `cargo fmt` with the second pass in `repository/`.

## Open Decisions

- **One golden target (`linux-x86_64`) vs. both** — recommend one, per §3.2. The
  shared streams are target-neutral (plan-34-D), so the second target mostly re-pins
  identical bytes while doubling churn. Revisit if a Linux-app bug ever lands that
  `linux-x86_64` goldens missed and `linux-aarch64` goldens would have caught.
- **Rename the fixture vs. leave `macos-app-mode-plumbing` carrying Linux goldens** —
  recommend renaming. The harness does not care, and a directory named `macos-*` full
  of `linux-*` goldens is a lie a future reader has to decode.
- **Fold `test-appimage.sh` into `test-accept.sh`** — recommend no, per §3.3. It needs
  a box that may be off, and a suite that goes red for infrastructure reasons trains
  people to ignore red.

## Summary

The risk here is **false confidence**, not correctness. Two specific traps:

**The acceptance script can pass on a broken app.** The GTK finish contract parks the
worker in `pause()` so a successful program does not exit — meaning "no hang" is a
symptom of *failure*, not success, in windowed mode. A script that checks for a clean
exit passes exactly when the app crashed before opening a window. §4.2 assert on the
program's own stdout marker and treats the watchdog's behavior as mode-dependent;
Phase 2's acceptance criterion is that the script goes red on a one-byte-truncated
AppImage, because a green light that cannot go red is worse than none.

**The goldens can pin nothing.** Phase 3's criterion is that reverting plan-51-A's
app-id change turns the suite red. If it does not, the golden covers bytes that do not
move.

Beyond that this is bookkeeping — but it is the bookkeeping that decides whether
plan-51 is a feature that works today or one that keeps working. Linux app mode
currently has no golden coverage and no automated execution anywhere; that is the gap,
and it predates this plan.

Left untouched: all compiler behavior. If this sub-plan finds a bug, the fix goes to
A, B, C, or a `bug-NN` — never here.
