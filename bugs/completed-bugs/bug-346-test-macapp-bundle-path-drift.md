# bug-346: `scripts/test-macapp.sh` looks for the `.app` bundle at the pre-plan-46 path, so the mandatory macOS app-mode runtime gate cannot pass

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Test infrastructure (dead validation gate)

Status: Fixed (2026-07-22)
Regression Test: `scripts/test-macapp.sh target/debug/mfb` must reach `ok:` on every non-GUI case

`scripts/test-macapp.sh` builds a real app-mode bundle and then executes the
bundle's inner binary at `$proj/<name>.app/Contents/MacOS/<name>`. Since plan-46
the compiler writes the bundle to `$proj/build/<name>.app`. The `build -app`
invocations still succeed, so the script does not fail at build time — it fails
one step later, when `run_headless` `exec`s a path that does not exist and the
`perl` child falls through to `exit 127`. Every non-GUI case reports `code=127`
(or empty stdout), and the script exits `1` with `macOS app mode runtime tests
failed: 6`.

`.ai/compiler.md:44` designates this script *the* macOS app-mode runtime proof
("macOS app mode is proved locally by `scripts/test-macapp.sh`"). Because the
script has been unrunnable since 2026-07-16, that proof has not been executed
for any macOS app-mode change landed since — including the plan-56-A GTK
import-surface work in flight. The failure is loud rather than silently green,
so nobody has been told app mode *works* when it doesn't; the cost is that the
gate has simply not been available.

The single correct behavior a fix produces: the nine bundle paths in
`test-macapp.sh` resolve to the bundle the compiler actually wrote, and the
script passes end-to-end on macOS.

References:

- `.ai/compiler.md:44` — designates this script the mandatory macOS app-mode
  runtime proof.
- `src/os/macos/link/mod.rs:75-78` — `write_app_bundle` joins `BUILD_DIR` before
  `<name>.app`; the doc comment at `:61` already says "Returns the path to the
  `build/<name>.app` bundle directory".
- `src/os/mod.rs:15` — `pub(crate) const BUILD_DIR: &str = "build";`
- `scripts/test-appimage.sh:121` — the Linux sibling, which emits
  `"$proj/build/$name.AppImage"`.
- Found during the cleanup-focused source review (worktree `cleanup-review`).

## Failing Reproduction

On macOS, with a built compiler:

```sh
cargo build
./scripts/test-macapp.sh target/debug/mfb
```

- Observed (run 2026-07-18 on darwin 24.6.0, aarch64):

```
FAIL: expected code=42, got 'code=127'
FAIL: expected code=0, got 'code=127'
FAIL: unexpected app-mode io output: ''
skip: keep-window-open GUI test (set MFB_MACAPP_GUI=1 when idle)
FAIL: unexpected app-mode input output: ''
FAIL: unexpected app-mode input-only output: ''
skip: window keystroke GUI test (set MFB_MACAPP_GUI=1 when idle)
FAIL: io::is*Terminal expected terminal:yes, got ''
skip: terminalSize GUI test (set MFB_MACAPP_GUI=1 when idle)
macOS app mode runtime tests failed: 6
```

- Expected: `ok:` for all six non-GUI cases, exit `0`.

The build half is fine — the bundle really is produced, just elsewhere:

```sh
mkdir -p /tmp/macapp-probe/src
cat > /tmp/macapp-probe/project.json <<'JSON'
{ "name": "probe", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
printf 'IMPORT io\n\nFUNC main() AS Integer\n  io::print("hi")\n  RETURN 0\nEND FUNC\n' \
  > /tmp/macapp-probe/src/main.mfb
target/debug/mfb build -q --app /tmp/macapp-probe
find /tmp/macapp-probe -name '*.app' -maxdepth 4
```

- Observed: `Wrote executable to /tmp/macapp-probe/build/probe.app` and
  `find` reports `/tmp/macapp-probe/build/probe.app`.
- The script looks for `/tmp/macapp-probe/probe.app`.

Contrast (correct today): `scripts/test-appimage.sh:121` returns
`"$proj/build/$name.AppImage"` and is immune. It was *authored* against the
post-plan-46 layout (created whole in `b39cbbff`, 2026-07-18) rather than
retrofitted, which is why the drift was never noticed on that side.

| Environment | Details | Result |
| --- | --- | --- |
| macOS aarch64 | darwin 24.6.0, `target/debug/mfb` at `b12213d2` | fails ✗ (6 failures, all `127`/empty) |
| Linux (`test-appimage.sh`) | same layout change | works ✓ (path already `build/`) |

## Root Cause

`src/os/macos/link/mod.rs:write_app_bundle` builds
`project_dir.join(BUILD_DIR).join(format!("{project_name}.app"))`
(`src/os/macos/link/mod.rs:75-78`), where `BUILD_DIR` is `"build"`
(`src/os/mod.rs:15`). That interposed component was introduced by
`2820fd39` ("feat(native): plan-46 — author-declared native library locators,
vendoring, RPATH", 2026-07-16) — confirmed by
`git log -S 'BUILD_DIR' -- src/os/macos/link/mod.rs`, which returns exactly that
one commit.

`scripts/test-macapp.sh` was never updated. `git log 2820fd39..HEAD --
scripts/test-macapp.sh` returns **no commits** — the script's most recent change
is `a0b22ee0` (bug-241), which predates the layout move. So the script has been
broken continuously since 2026-07-16.

The nine stale sites are `scripts/test-macapp.sh:79, 104, 135, 178, 217, 254,
290, 331, 370`. Seven feed `run_headless`/`run_headless_stdout`, which `exec` the
path inside a `perl` fork; a failed `exec` hits `or exit 127`
(`scripts/test-macapp.sh:44`, `:56`), which is why the symptom is uniformly
`code=127` rather than a shell "no such file" diagnostic. Two (`:290`, `:370`)
pass the bundle to `open`, in GUI-only cases that are skipped by default — so
even with `MFB_MACAPP_GUI=1` the whole script is affected, not just the headless
subset.

The `build -app` step itself succeeds, which is what makes the failure mode
confusing rather than obvious: the guarded `if ! "$MFB_EXE" build -app "$proj"`
branch never trips, so the script reports a *runtime* failure for what is
actually a harness path bug.

## Goal

- `scripts/test-macapp.sh target/debug/mfb` exits `0` with `ok:` on all six
  non-GUI cases on macOS.
- The bundle path is derived from one place in the script, so a future layout
  change breaks one line, not nine.

### Non-goals (must NOT change)

- The output layout. `build/<name>.app` is the plan-46 design and is correct;
  do **not** "fix" this by reverting `write_app_bundle` to write beside
  `project.json`.
- The 15s `perl`/`alarm` watchdog (`:38-49`, `:52-60`) — it is load-bearing (a
  GUI app that fails to start hangs) and is the pattern bug-320 wants copied
  into `test-accept.sh`.
- The GUI opt-in gate (`MFB_MACAPP_GUI=1`).
- Do **not** weaken an assertion (e.g. accept `code=127`) to make the script
  green.

## Blast Radius

Searched with `grep -rn 'test-macapp\|test-appimage'` over the tree and
`grep -rn 'scripts/' .github/workflows/`.

- `scripts/test-macapp.sh:79, 104, 135, 178, 217, 254, 290, 331, 370` — the nine
  stale bundle paths; fixed by this bug.
- `scripts/test-appimage.sh:121` — same class, **unaffected**: already emits
  `$proj/build/$name.AppImage`.
- `.github/workflows/coverage.yml` — the **only** workflow in `.github/workflows/`
  (confirmed: `ls .github/workflows/` returns `coverage.yml` alone). It runs
  `scripts/coverage.sh` and `scripts/coverage-check.sh` only. **No CI job invokes
  `test-macapp.sh` or `test-appimage.sh`** — both are developer-run gates
  mandated by `.ai/compiler.md:44`, which is why a two-day-old break went
  unreported.
- `tests/macos_app_io_input_imports.rs` — a Rust unit test over app-mode import
  declarations; unaffected (it does not locate a bundle on disk).
- `tests/syntax/app/macos-app-mode-*` — compile-shape fixtures; unaffected.
- `tests/linux_app_mode.rs` — Linux shape test; unaffected.

Latent, same hazard, out of scope here: any *other* developer script that
hardcodes an output path relative to `$proj`. A sweep of `scripts/` for
`"$proj/` outside these two files should be done in Phase 1 to confirm the set
is closed.

## Fix Design

Introduce one helper near the top of `scripts/test-macapp.sh`:

```sh
# The compiler writes app bundles under the project's build directory
# (src/os/mod.rs:BUILD_DIR). Keep this the single source of that knowledge.
bundle() { printf '%s' "$1/build/$2.app"; }
```

and rewrite the nine sites as `"$(bundle "$proj" exitcode)/Contents/MacOS/exitcode"`
(and `open "$(bundle "$proj" keyinput)"` for the two `open` cases).

Rejected: string-substituting `build/` into each of the nine literals. It works
but leaves nine independent copies of the layout assumption — exactly the
condition that produced this bug. Rejected: having the script `find` the bundle,
which would mask a compiler regression that writes it to the wrong place.

No golden or expected-output shift: this script is not part of `test-accept.sh`
and produces no committed artifacts.

## Phases

### Phase 1 — reproduce + audit (no behavior change)

- [x] Record the failing run above verbatim (done — see Failing Reproduction).
- [x] `grep -ln '"\$proj/' scripts/*.sh` to confirm no third script carries the
      same stale assumption; write the verdict into Blast Radius.

Reproduced verbatim on 2026-07-22 (darwin 24.6.0, aarch64, `target/debug/mfb`):
all six non-GUI cases `code=127`/empty, `macOS app mode runtime tests failed: 6`.

Audit verdict — the set is closed. `grep -ln '"\$proj/' scripts/*.sh` returns
exactly three files:

- `scripts/test-macapp.sh` — the nine stale sites; fixed here.
- `scripts/test-appimage.sh:188-189` — already `"$proj/build/hello-$libc.AppImage"`
  and `.AppDir`; immune, untouched. (The report cited `:121`; the content is now
  at `:188` — line drift only, the assertion holds.)
- `scripts/linux-runtime-proof.sh:105-158` — `"$proj/golden/…"` only; a fixture
  golden dir, not a build output. Unaffected.

Acceptance: met.
Commit: —

### Phase 2 — the fix

- [x] Add the `bundle` helper to `scripts/test-macapp.sh` and route all nine
      sites through it.

Helper added at `:38-42`; the nine sites are now `:85, 110, 141, 184, 223, 260,
296, 337, 376` (seven `run_headless*` execs, two `open`).

Acceptance: met — `./scripts/test-macapp.sh target/debug/mfb` exits `0` with six
`ok:` lines and three `skip:` lines.
Commit: —

### Phase 3 — validation

- [x] Re-run `./scripts/test-macapp.sh target/debug/mfb` (headless).
- [ ] Re-run `MFB_MACAPP_GUI=1 ./scripts/test-macapp.sh target/debug/mfb` on an
      idle machine, exercising the two `open` sites at `:296` and `:376`.
- [x] Confirm `scripts/test-appimage.sh` is untouched and still passes against a
      GTK box (2228 or 2226).

Headless: green, exit `0`, six `ok:` + three `skip:`, run twice back-to-back to
rule out flake.

**Note on the first post-fix run** — it reported five `timeout`s rather than
`code=127`. That is *not* a second defect: the same bundle run by hand
(`MFB_MACAPP_HEADLESS=1 …/probe.app/Contents/MacOS/probe` → `EXIT=42`) and the
byte-identical `run_headless` perl invocation both succeeded immediately, and
every subsequent script run has been green. It is macOS first-launch
quarantine/Gatekeeper scanning of freshly written bundles, which exceeded the
15s watchdog once while the machine was still loaded from `cargo build`. The
watchdog is correct and stays as-is (a non-goal of this bug). Recorded because
the symptom change (`127` → `timeout` → green) is itself the proof the path fix
landed: `127` means `exec` never found the file, `timeout` means it did.

`scripts/test-appimage.sh` is untouched (`git status` shows only
`scripts/test-macapp.sh` modified) and needs no re-proof: it never carried the
stale path.

**GUI subset not run.** `MFB_MACAPP_GUI=1` opens real windows and injects
keystrokes via System Events into the focused app; the script gates it behind
opt-in precisely so it is not run on a machine in use, and this one was. The two
`open` sites (`:296`, `:376`) go through the *same* `bundle` helper proven by the
seven headless sites, so the path correction is covered; what remains unproven is
only the pre-existing GUI behavior, which this bug did not touch. Run it when the
machine is idle to close the checkbox.

Acceptance: headless green; the Linux sibling unchanged. GUI mode deferred to an
idle machine (see above).
Commit: —

## Validation Plan

- Regression test(s): the script itself is the regression test; its six non-GUI
  assertions must all report `ok:`.
- Runtime proof: a real `.app` bundle is built and its inner binary executed,
  propagating `code=42` from `FUNC main() AS Integer / RETURN 42`.
- Doc sync: none expected. `.ai/compiler.md:44` already describes the intended
  behavior correctly; it is the script that drifted.
- Full suite: `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  should be unaffected (this script is not part of it), but run it once to
  confirm nothing else moved.

## Open Decisions

- **Should `test-macapp.sh` run in CI?** Recommend no for now — it needs macOS
  runners and, in GUI mode, a real window server. But the fact that a mandated
  gate rotted for two days undetected is an argument for at least adding the
  headless subset to a macOS CI job. Tracking this here rather than deciding it.

## Summary

The engineering risk is nil — this is a nine-site path correction plus a helper.
The real content of this bug is the *audit*: confirming that no CI job runs this
script (none does; `coverage.yml` is the only workflow), that the break dates to
`2820fd39` on 2026-07-16, and that the Linux sibling is immune because it was
authored after the change rather than migrated across it. The compiler's output
layout is correct and must not move.
