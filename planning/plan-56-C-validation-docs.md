# plan-56-C: Validation and doc sync

Last updated: 2026-07-18
Effort: medium (1h–2h)
Depends on: plan-56-B

plan-56-A makes the app-mode import surface flavor-correct and plan-56-B emits
both AppImages. Neither can prove it worked: **musl's loader silently absorbs
`libc.so.6` and `libpthread.so.0` into itself**, so a musl AppImage built with
the old glibc-hardcoded imports launches and behaves identically to one built
correctly. Verified empirically on box 2227 with `gcompat` removed and no
`/lib/libc.so.6` on disk — the wrongly-linked binary still reached GTK.

This sub-plan builds the only check that can tell them apart, and syncs the specs
that still say app mode is glibc-only.

The behavioral outcome: `scripts/test-appimage.sh` fails when a musl AppImage's
inner ELF names any glibc library, and every spec page describes two AppImages.

References (read first):

- `scripts/test-appimage.sh` — the plan-51-D acceptance script this extends;
  already ships the artifact over ssh and has the `timeout_run` watchdog.
- plan-56-A §2.4 — the musl-absorbs-glibc-names finding and its evidence.
- `.ai/specifications.md` — the spec-currency obligation.
- `.ai/remote_systems.md` — box 2227 (Alpine x86_64, musl, GTK4 installed,
  gcompat removed), the musl proof surface.
- `src/docs/spec/app/02_linux-runtime.md:1-40` — the glibc-only claims.
- `bugs/bug-320-*` — `test-accept.sh` has no `.run` watchdog; unrelated but in
  the same script family, do not conflate.

## 1. Goal

- `scripts/test-appimage.sh --libc musl` asserts the musl AppImage's inner ELF
  has **zero** glibc library names in `DT_NEEDED`, and fails if any appear.
- The same script runs the musl AppImage on box 2227 and the glibc one on 2228.
- Every spec page that says app mode is glibc-only, or names one Linux app
  artifact, matches the compiler.

### Non-goals (explicit constraints)

- **No new compiler behavior.** If this sub-plan finds a bug, the fix belongs in
  plan-56-A or -B, or a `bug-NN` — not here.
- **No riscv64 coverage.** App mode is unsupported there permanently
  (plan-51-A §3.3).
- **No goldens.** plan-51-D established that the harness builds one target per
  run (`MFB_TARGET`, which nothing sets) and that `build.log` records the command
  line, so a fixture cannot carry goldens for two targets. Linux app coverage
  stays in `tests/linux_app_mode.rs`. Do not re-litigate this — see
  plan-51-D's closing note.
- **No gcompat dependency.** The check must pass on a box with **or** without
  gcompat; that is the entire point of inspecting `DT_NEEDED` rather than
  launching.

## 2. Current State

### 2.1 The verification gap, stated precisely

On box 2227 (stock Alpine x86_64, gcompat removed), a musl app binary whose
`DT_NEEDED` contains `libc.musl-x86_64.so.1`, `libpthread.so.0` **and**
`libc.so.6` loads and runs. `ldd` reports `libpthread.so.0 =>
/lib/ld-musl-x86_64.so.1` and omits `libc.so.6` entirely; musl also supplies
`__libc_start_main` as a compat symbol.

So every runtime signal — exit code, stdout marker, GTK reaching its display
probe — is **identical** between a correct and an incorrect musl build. The
information exists only in the ELF's dynamic section.

### 2.2 `test-appimage.sh` is glibc-shaped

The plan-51-D script hardcodes one target per box (`2228 → linux-x86_64`,
`2226 → linux-aarch64`) and builds `build/<name>.AppImage`. After plan-56-B that
path does not exist: the artifacts are `<name>-glibc.AppImage` and
`<name>-musl.AppImage`. Every case needs a flavor.

### 2.3 The specs assert glibc-only in several places

`grep -rn "glibc-only" src/docs/spec/` and the app-mode pages state the
restriction as fact, and `architecture/08_artifacts.md` lists a single
`build/<name>.AppImage` row (added by plan-51-D this session).

## 3. Design Overview

Three pieces:

1. **The `DT_NEEDED` assertion** (§4.1) — the only check that can detect the bug
   plan-56-A fixes. Everything else in this sub-plan is secondary to it.
2. **Flavor-aware `test-appimage.sh`** (§4.2) — a `--libc` selector, the right
   box per flavor, and every existing case run per flavor.
3. **Doc sync** (§4.3).

The risk here is the same one plan-51-D named: **false confidence**. A script
that launches the musl AppImage and sees the GTK marker will go green on a
completely wrong build. §4.1 exists because that is not a hypothetical — it is
the observed behavior.

## 4. Detailed Design

### 4.1 The `DT_NEEDED` assertion

For a musl artifact, extract the payload and inspect the inner ELF:

```sh
readelf -d squashfs-root/usr/bin/<name> | awk '/NEEDED/ {print $NF}' | tr -d '[]'
```

Assert:

- **no** entry matches `^libc\.so\.6$`, `^libpthread\.so\.0$`, `^libdl\.so\.2$`,
  `^librt\.so\.1$`, `^libm\.so\.6$` — the glibc compat set musl absorbs;
- exactly one entry matches `^libc\.musl-.*\.so\.1$`.

And the mirror for glibc: `libc.so.6` present, no `libc.musl-*`.

⚠️ Assert on the **absence** list, not only the presence of the musl libc. A
binary carrying both (which is precisely the pre-plan-56-A state) satisfies
"names the musl libc" and must still fail.

`readelf` is on the Alpine box (`binutils`); if absent the case must **fail
loudly**, never skip — a skipped check here is indistinguishable from a passing
one, which is the failure mode this whole sub-plan exists to prevent.

### 4.2 Flavor-aware acceptance script

```text
scripts/test-appimage.sh <mfb-exe> [--box <port>] [--libc glibc|musl] [--gui]
```

- `--libc` selects which artifact to ship and which assertions to apply;
  default: run **both**.
- Box selection: glibc → 2228 (Ubuntu x86_64 GTK), musl → 2227 (Alpine x86_64
  GTK). aarch64 glibc → 2226 when reachable.
- Every plan-51-D case (mode 0755, mount+start, `--appimage-extract-and-run`,
  extract-vs-AppDir, `desktop-file-validate`, corrupted-superblock-fails,
  vendored library, RUNPATH, loader expansion) runs per flavor.
- The vendor case picks the flavor-matching blob
  (`libsndfile.so.1.0.37-x86_64-musl` vs `-glibc`) and asserts the *other*
  flavor's blob is **absent** from the image (plan-56-B §4.3's routing).

Box 2227 has **no `fusermount`**, so the FUSE mount path is unavailable there.
The musl run must use `--appimage-extract-and-run`, and the script must detect
this rather than reporting a false failure — check for `fusermount`/`fusermount3`
and select the path, logging which one it used.

### 4.3 Doc sync

| page | change |
| --- | --- |
| `src/docs/spec/architecture/08_artifacts.md` | two rows: `build/<name>-glibc.AppImage`, `build/<name>-musl.AppImage`; same for the `--app-debug` AppDirs |
| `src/docs/spec/tooling/07_cli-reference.md` | `--app` emits two artifacts on Linux; the flavor suffix |
| `src/docs/spec/app/02_linux-runtime.md` | app mode is **not** glibc-only; the C-library imports are flavor-derived; a musl app needs a musl GTK4 host (Alpine `gtk4.0`) |
| `src/docs/spec/app/spec.md` | the Linux output shape is two files |
| `src/docs/spec/language/17_native-libraries.md` | the per-flavor vendor table (plan-56-B §4.3) |
| `.ai/remote_systems.md` | 2227 is the musl app-mode proof surface; note it has GTK4 but no `fusermount`, and that gcompat was removed deliberately so glibc deps fail loudly |
| `.ai/compiler.md` | app-mode proof now spans two boxes and two libcs; a musl AppImage **cannot** be validated by launching it |

Also `grep -rn "glibc-only" src/ planning/` and correct every stale claim,
**except** inside `planning/old-plans/` (archived history stays as written).

## Phases

### Phase 1 — The `DT_NEEDED` assertion

Lands first: it is the check the other two sub-plans are gated on, and it is
independently valuable — pointed at a pre-plan-56-A binary it must go red.

- [ ] Add the flavor-aware `DT_NEEDED` extraction + absence/presence assertions
      to `scripts/test-appimage.sh` per §4.1.
- [ ] Make a missing `readelf` a hard failure, not a skip.

Acceptance: run against a musl AppImage built from a **pre-plan-56-A** compiler
(stash the import change, or reuse a saved artifact) and confirm the script goes
**red**; run against a post-plan-56-A one and confirm green. A check that cannot
go red on the known-bad input is not a check.
Commit: —

### Phase 2 — Flavor-aware acceptance cases

- [ ] Add `--libc`, per-flavor box selection, and the fusermount-vs-extract path
      detection (§4.2).
- [ ] Run every existing case per flavor; add the "other flavor's vendored blob
      is absent" assertion.

Acceptance: `scripts/test-appimage.sh target/debug/mfb` passes for both flavors —
glibc on 2228, musl on 2227 — and still goes red on a one-byte-corrupted
superblock in each.
Commit: —

### Phase 3 — Doc sync

- [ ] Update every page in §4.3's table.
- [ ] `grep -rn "glibc-only" src/` returns nothing stale outside
      `planning/old-plans/`.

Acceptance: `mfb spec` renders; no page describes Linux `--app` as emitting one
artifact or as glibc-only.
Commit: —

## Validation Plan

- **Tests:** the script is the test. Its own acceptance criterion is Phase 1's —
  it must go red on a known-bad musl binary. A green script that cannot go red is
  the specific failure this sub-plan exists to prevent.
- **Runtime proof:** both AppImages run on their own box — glibc on 2228, musl on
  2227 via `--appimage-extract-and-run` (no `fusermount` there). ⚠️ Treat the
  launch as a **liveness** check only; the `DT_NEEDED` assertion is the
  correctness check.
- **Doc sync:** Phase 3.
- **Acceptance:** `scripts/test-accept.sh`, `scripts/artifact-gate.sh`,
  `cargo test`, `cargo fmt` with the second pass in `repository/`.

## Open Decisions

- **Default `--libc` to both vs. glibc** — recommend both, so the musl path
  cannot rot unnoticed. The cost is one extra box round-trip per run.
- **Fail vs. skip when box 2227 is unreachable** — recommend skip (matching
  plan-51-D's rule that the GTK boxes are developer infrastructure, not CI), but
  **fail** when the box is reachable and `readelf` is missing, since that is a
  broken check rather than absent infrastructure.

## Summary

Everything in this sub-plan serves one fact: **a wrongly-linked musl AppImage
runs perfectly.** musl absorbs `libc.so.6` and `libpthread.so.0` into its own
loader, so exit codes, stdout markers, and GTK reaching its display probe are all
identical between a correct build and the broken pre-plan-56-A one. That was
confirmed on stock Alpine with gcompat removed — the obvious explanation ("it
only worked because of gcompat") was tested and disproved.

So the acceptance script's launch cases prove liveness and nothing more, and the
`DT_NEEDED` assertion is the only thing standing between plan-56 and silently
shipping glibc-flavored musl binaries forever. Phase 1 is therefore gated on the
script demonstrably going red against a known-bad artifact, not merely passing
against a good one.

Left untouched: all compiler behavior, the goldens question (settled by
plan-51-D), and riscv64.
