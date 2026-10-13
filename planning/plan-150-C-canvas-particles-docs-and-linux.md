# plan-150-C: Particle systems — documentation, examples, and Linux/Vulkan parity

Last updated: 2026-10-11 (re-derived against the completed plan-172; see
"Re-derivation record")
Effort: medium (1h–2h)
Depends on: plan-150-B

Letters A and B build and measure the particle system. This letter is what makes it a
**shipped feature rather than a working one**: the manual entries a user learns it
from, a worked example per effect, the spec sync the project requires of any new
package surface, and the Vulkan-side proof that the "no shader change" claim holds on
Linux and Windows and not only on Metal.

Behavioral outcome: a user who has never seen the feature can read `mfb man canvas`,
copy a runnable example, and get fire, fog, or fireworks on macOS, Linux and Windows —
and the Vulkan path is proved to match the software oracle, not assumed to.

References:

- `planning/completed/plan-150-A-canvas-particles-types-and-expansion.md` — the type set the
  manual documents.
- `planning/completed/plan-150-B-canvas-particles-cost-and-damage.md` — the measured cost model
  the manual must quote instead of adjectives.
- `planning/new_canvas.md` — the shape model every example is written in: `position`
  and `transform` on each shape, `vector::Float2` coordinates, no `Paint.transform`.
- `scripts/test-canvas-vulkan.sh`, `scripts/test-canvas-gpu-rows.sh` — the Linux and
  Windows canvas parity harnesses (`scripts/README.md`).
- `scripts/man-examples-gate.sh`, `tests/cli/cli_canvas_man_examples_compile.rs` — every
  manual example must compile; this is gated, not aspirational.
- `.ai/man-content.md`, `.ai/spec-content.md`, `.ai/specifications.md` — the two
  documentation surfaces and their standards.
- `.ai/remote_systems.md` — the test boxes and how to reach them.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-150-B complete and archived | `ls planning/plan-150-B-*` → no matches, and `ls planning/completed/plan-150-B-*` → present | MET (2026-10-13: no matches; `planning/completed/plan-150-B-canvas-particles-cost-and-damage.md`) |
| The measured cost table exists in letter B | `grep -c 'particles/frame' planning/completed/plan-150-B-*.md` → ≥1 | MET (2026-10-13: 4) |
| A Linux box with a Vulkan device is reachable | `scripts/test-canvas-vulkan.sh` defaults to `--box 2228` (Ubuntu x86_64 glibc); probe per `.ai/remote_systems.md` | MET, on a different box (2026-10-13): 2228 is not running (`ssh -p 2228 test@127.0.0.1` → "Connection refused", as are 2227 and 2225). **Box 2226** (Debian 12 aarch64, glibc) is up and reports one device: `ssh -p 2226 … vulkaninfo --summary` → `deviceName = llvmpipe (LLVM 15.0.6, 128 bits)`, `driverInfo = Mesa 22.3.6`, `apiVersion = 1.3.230`, `deviceType = PHYSICAL_DEVICE_TYPE_CPU`. The script takes `--box 2226 --target linux-aarch64`. Corrections 1 |
| The Windows box is reachable | `.ai/remote_systems.md`: 192.168.1.206, the only Windows box and the only real GPU | MET (2026-10-13: `ssh test@192.168.1.206 ver` → "Microsoft Windows [Version 10.0.26200.9457]") |

If plan-150-B is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

### Re-derivation record (2026-10-11)

- **Five types, not seven.** The `Emitter` union and its four `Emit*` records are gone:
  the emitter is a `canvas::Shape`. Letter A adds `ParticleSystem`, `Emission`, `Motion`,
  `Appearance` (records) and `EmissionMode` (enum).
- **The raises are 77050028 and 77050029** (re-measured by letter A; 77050025–27 were
  taken).
- **The spec gained whole sections** this feature must appear in: "The item unions",
  "Hit-testing", "AutoPaint", "A picture is drawn inside a shape"
  (`grep -n '^## \|^### ' src/docs/spec/app/06_canvas.md`).
- **There are seven redraw triggers**, so the sentence to add says "not an eighth".
- **The examples are written in the plan-172 surface**, and there is a Windows GPU lane
  now (`scripts/test-canvas-gpu-rows.sh --target windows-x86_64`).

## 1. Goal

- `mfb man canvas` documents `ParticleSystem` and its four companion types with a
  runnable example each, and the examples are gated to compile.
- Four worked example programs exist — fire, fog, fireworks, rain — under
  `examples/canvas/`.
- The Vulkan backend renders a particle scene within `Tolerance::GPU_DEFAULT` of the
  software oracle on a real Linux host and on the Windows box, with **no** diff in
  either `.spv` blob.
- The spec is in sync per the project's own census.

### Non-goals (explicit constraints)

- **No behavioural change.** If this letter finds a bug, it is filed via `write-bug`
  and fixed there — not absorbed here. A documentation letter that quietly changes
  rendering is a letter nobody can review.
- **No `scripts/regen-spirv.sh` run.** The premise under test is that the shaders are
  untouched. Regenerating SPIR-V here would destroy the evidence.
- **No new public functions.** The surface is the types letter A added. In particular
  no constructor functions (the owner's answer to plan-172-A Open Decision 1).

## 2. Current State

The `canvas` package registers 20 records, 3 unions, 13 enums and 95 functions
(`grep -c 'pkg.add_record(RegistryRecord {' src/codegen/builtins/canvas/mod.rs` → 20;
`grep -c 'pkg.add_union(RegistryUnion {'` → 3; `grep -c 'pkg.add_enum('` → 13;
`grep -c 'pkg.add_function' src/codegen/builtins/canvas/*.rs | awk -F: '{s+=$2} END {print s}'`
→ 95; all 2026-10-11). Letter A adds 4 records and 1 enum to that, and no union.

Manual examples are **gated**: `tests/cli/cli_canvas_man_examples_compile.rs` compiles
every example in the canvas manual, so a documented snippet that does not build fails
the suite. `scripts/man-examples-gate.sh` and `scripts/man-run-examples.sh` are the
project-level equivalents.

Linux GPU parity has a dedicated harness, `scripts/test-canvas-vulkan.sh`, which builds
a canvas program with the twelve-glyph test TrueType font and compares GPU against
software — the same oracle relationship `tests/canvas/rt_canvas_metal.rs` uses on
macOS. `scripts/test-canvas-gpu-rows.sh` runs every scene in `tests/canvas/scenes/` on a
chosen target, Windows included, and requires each to draw on the GPU and agree with the
oracle; letter A Phase 5 puts the fireworks scene there.

`examples/` holds 25 example projects (`ls examples | wc -l` → 25, 2026-10-11). There is
a `canvas-demo` and **no `canvas` directory** (`ls examples/canvas` → no such directory).

### Measured populations

| What | Count | Command |
|---|---|---|
| Canvas records / unions / enums before letter A | 20 / 3 / 13 | the three greps above, 2026-10-11 |
| Canvas registry functions | 95 | `grep -c 'pkg.add_function' src/codegen/builtins/canvas/*.rs \| awk -F: '{s+=$2} END {print s}'` → 95 (2026-10-11) |
| Existing example projects | 25 | `ls examples \| wc -l` → 25 (2026-10-11) |
| Existing canvas particle examples | 0 | `ls examples/canvas` → no such directory (2026-10-11) |
| Canvas goldens | 11 | `ls tests/golden/canvas \| wc -l` → 11 (2026-10-11) |

### Verified properties

- **Manual examples are compile-gated.** VERIFIED: `tests/cli/cli_canvas_man_examples_compile.rs`
  exists (`ls`, 2026-10-11) — the canvas manual's examples are extracted and built, so
  every snippet this letter writes must be runnable.
- **UNVERIFIED — whether the Vulkan path draws particles correctly.** Letter A proved
  the claim on Metal only. Both backends read the same `ItemBlock` and the same draw
  entry, so the claim *should* transfer, but "should" is what Phase 1 exists to replace.

## 3. Design Overview

Three independent pieces, orderable by risk:

1. **Vulkan parity** (Phase 1) — the only phase that can discover a *defect*, so it
   goes first. If the Vulkan path disagrees with software, everything downstream is
   documenting something that does not work on Linux or Windows.
2. **Manual entries** (Phase 2) — the five types, each with a compiling example, quoting
   letter B's measured cost model.
3. **Examples and spec sync** (Phase 3) — four worked programs and the project's census
   scripts.

**Where risk concentrates: Phase 1, and it is scheduled first rather than last** — the
usual blast-radius-last rule does not apply, because this phase changes no code. It is
a measurement that can fail, and failing early is the point.

**Byte-identity is not the gate.** The gate is the Vulkan parity comparison and the
compile-gated examples.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`.
> **An unticked box means NOT DONE.**

### Phase 1 — Vulkan parity (the phase that can fail)

- [x] Extend `scripts/test-canvas-vulkan.sh` with the fireworks scene letter A
      goldened, rendered twice — `MFB_CANVAS_GPU=1` and software — and compared with
      the existing tolerance comparator. Add the same scene inside a turned, scaled
      group: the draw-entry matrix is the newest thing either backend reads. —
      `particle_scene`, run for `particles_on_black` and `particles_grouped_on_black`
      (the scene files the Rust suites include). It also holds the software render to
      `tests/golden/canvas/particles.png` EXACTLY, which no plan asked for and is the
      determinism claim measured on a second platform.
- [x] Run it on a real Linux host with a Vulkan device. The script ships an AppImage
      and takes `<mfb-exe> [--box <port>] [--target …] [--libc glibc|musl] [--icd …]`,
      defaulting to box 2228 (Ubuntu x86_64 glibc). The `--libc` must match the box —
      musl's loader absorbs the glibc compat sonames, so shipping the wrong one does not
      fail cleanly. `.ai/remote_systems.md` says **re-probe, do not assume**. Confirm the
      box reports a Vulkan device before reading a pass as meaningful. — on box 2226
      (the prerequisite row: llvmpipe, a conformant Vulkan 1.3 driver on the CPU, not a
      hardware GPU). `bash scripts/test-canvas-vulkan.sh target/debug/mfb --box 2226
      --target linux-aarch64` → "canvas Vulkan runtime tests passed", with
      `ok: particles: the scene reached the GPU (gpuFrames non-zero)`,
      `ok: particles: the software render reproduces tests/golden/canvas/particles.png exactly`,
      `ok: particles: the Vulkan render matches the software oracle (ok worst=2 differing=0.7295%)`
      and the same three for the grouped scene (no reference; `differing=1.9771%`
      before Corrections 2, 0.8363% after).
- [x] Run `scripts/test-canvas-gpu-rows.sh <mfb-exe> --target windows-x86_64` against
      the Windows box and confirm the particle scene's row reports `gpuFrames` non-zero.
      — `… --target windows-x86_64 --rows particles,particlesgroup` on box 206 (Intel
      UHD 630): `ok: particles: drawn on Vulkan (gpuFrames=1)`, `Vulkan matches the
      software oracle (ok worst=2 differing=0.7510%, 18953 lit)`; `particlesgroup`
      `gpuFrames=1`, `differing=0.8698%`; `geoVerifyMismatches=0` on all four runs. The
      two rows only: the other 58 rows are not this plan's, and the full table runs in
      the final gate's company, not here.
- [x] Confirm `git status --porcelain src/codegen/runtime/canvas/shaders/` is empty —
      no `.frag`, `.vert`, or `.spv` diff. — empty; and `git diff main --stat --
      src/codegen/runtime/ src/target/` is empty: no file of the runtime emitters or of
      any target differs from `main`.
- [x] ~~If a comparison fails: **root-cause it, do not re-baseline.** A mismatch means
      the item composition in letter A Phase 4 is wrong on one backend; localize from
      the reported coordinate, fix, and record it in letter A's Corrections section.~~
      — moot: no comparison failed, on either lane (the six `ok:` lines above).

Acceptance: the particle scene renders on Vulkan within `Tolerance::GPU_DEFAULT` of the
software oracle on a Linux host reporting a Vulkan device and on the Windows box, and
both `.spv` blobs are unchanged.
  Check: `scripts/test-canvas-vulkan.sh <mfb-exe> --box 2228` → pass (est. 15 min,
  guess — longer than 10 because it builds on a remote box; no smaller check exists,
  since the whole question is whether a real Vulkan driver agrees with the oracle), and
  the Windows row above (est. 15 min, guess).
  Result (2026-10-13): met on box 2226 and box 206, as recorded on the task lines; both
  `.spv` blobs unchanged. Metal re-run after Corrections 2: `cargo test --test
  rt_canvas_metal particle` → "2 passed; 0 failed".
Commit: ff35c95a8

### Phase 2 — Manual entries

Registry descriptor prose, in `src/codegen/builtins/canvas/mod.rs`, to `.ai/man-content.md`'s
standard. Rendering the page is the only verification: the compiler never reads it.

- [x] Write the `mfb man canvas` prose for `ParticleSystem`, `Emission`, `Motion`,
      `Appearance`, `EmissionMode` — each field's units named, the tint rule stated as
      the multiply it is and cross-referenced to `Picture`'s identical rule. — written
      with the types in letter A Phase 2 and re-read here against a run of each claim;
      one was wrong and is fixed: the prose spelled the constant `math::PI`, which does
      not exist (`math::pi`) — found by building the examples (Corrections 4).
- [x] State, in `ParticleSystem`'s prose, what each emitter shape emits from: its
      interior for a `Rectangle`, `Circle`, `Ellipse`, `Polygon` and `Text`, its length
      for a `Line`, `Arc` and `PolyLine`; that the emitter's own `position` and
      `transform` place and turn it; and that its `paint` and `id` are not used. — on
      the `emitter` field, with `CurvePolygon` (interior) and `PolyCurve` (length) for
      the ten shapes there are.
- [x] State what a particle does with each template kind (letter A §6): a shape, an
      `AutoPaint` (its default paint), ~~a `Picture` (and what `ClipOnly` means for a
      spinning sprite), a `Group` (and its tint)~~. — a shape and an `AutoPaint`, and in
      place of the two struck: "a `canvas::Picture`, a `canvas::Group` or another
      `ParticleSystem` cannot be `item`, and is refused when the program is built"
      (letter A Corrections 3).
- [x] State that a particle is never under the mouse: it is not named by
      `MouseEvent.overID` and does not hide what is beneath it from the mouse.
- [x] State explicitly, in `ParticleSystem`'s prose, that `time` is advanced **by the
      program** and that re-presenting is what draws the next frame — there is no timer
      (`.ai/canvas-threading.md` §4).
- [x] Quote letter B's measured particle counts. No adjectives where a number exists.
      — done by letter B Phase 4 and unchanged: 3.5 / 21 / 95 ms at 1,000 / 10,000 /
      50,000, 60 fps to about 8,000 (3,000 turned or scaled), no threshold.
- [x] Document the ~~two raises (77050028, 77050029, by their `errorcode` names)~~ one
      raise and the silently-nothing cases from letter A §7. — `ErrCanvasParticleLifetime`
      on `Emission.lifetime`, on `canvas::present` and on `canvas::presentLayers`, each
      with its Errors-table row; the quiet cases in `Emission`'s description and on
      `time`. 77050029 does not exist: it became a build error (letter A Corrections 3).
- [x] Add one compiling example per type to the manual. — a record has no page of its
      own to carry one (the `AutoPaint` precedent), so they are two programs on
      `canvas::present`: a fountain (`ParticleSystem`, `Emission`, `Motion`,
      `Appearance`, `EmissionMode.Continuous`, animated by the frame loop) and a
      firework (`EmissionMode.Burst`). `the_particle_system_examples_compile` finds each
      by what it contains and asserts it names its types.
- [x] (added) Resolve the manual's vocabulary ban against the field name `lifetime`
      (Corrections 3): carve-out 6 in `scripts/man-census.sh` and
      `.ai/man-content.md` §4.4, the owner's choice over renaming the fields.

Acceptance: `mfb man canvas types` documents all five types, and every manual example
compiles under the existing gate.
  Check: `cargo test --test cli_canvas_man_examples_compile` → pass, and
  `bash scripts/man-census.sh --memory-scope` → 0 unclassified (est. 5 min, guess).
  Result (2026-10-13): `cargo test --test cli_canvas_man_examples_compile` → "3 passed;
  0 failed"; `MFB=./target/debug/mfb bash scripts/man-census.sh --memory-scope` →
  "unclassified memory-vocabulary hits: 0" (carve-out 6: 12). The two examples also
  RUN: `scripts/man-run-examples.sh canvas --run present` → "built: 2 ran: 2 failed: 0".
Commit: ab1e86829

### Phase 3 — Worked examples and spec sync

- [x] Add `examples/canvas/` with four programs: `fire` (a `Line` emitter along the
      base, upward drift, orange→transparent ramp, turbulence), `fog` (a `Rectangle`
      emitter, near-zero speed, large soft `Picture` particles, low alpha), `fireworks`
      (a `Burst` system with a `Circle[radius := 0.0]` emitter, gravity and a colour
      ramp), `rain` (a `Rectangle` emitter across the top, high downward speed, a thin
      `Line` template). — `fire`, `fog`, `fireworks`, `rain`, each a project of its
      own with one `src/main.mfb`, and a `README.md`. The fog's particle is a disc with
      a radial gradient, not a `Picture` (Corrections 5). Each quits on Esc or Q.
- [x] Wire them into `scripts/build-examples.sh` so they are built by the suite. —
      nothing to wire: the script builds every `examples/**/project.json`
      (`find examples -name project.json | grep canvas/` lists the four). Its loop, run
      for the four alone on its four app targets (`/tmp/p150/build_examples.sh`): 16 of
      16 `ok`, each target's artifacts present.
- [x] Review the spec letter A wrote against `.ai/spec-content.md`:
      `src/docs/spec/app/06_canvas.md`'s "The item unions" (the twelfth variant and why
      it is in neither `Shape` nor `AutoShape`), its particle section (the model, the
      expansion, what each walk does with a system), "Hit-testing" (a system is not
      hit), and `src/docs/spec/diagnostics/02_error-codes.md`. — one claim failed the
      "which backend" rule and is rewritten: "draws the same picture on every run,
      backend and platform" is true of the ITEMS, and of the pixels only in software
      (measured: macOS's reference reproduced exactly on Linux AArch64); a GPU backend is
      within tolerance. No plan or bug number is in the section; every measured number
      names its machine.
- [x] Run the project's census scripts and resolve what they report:
      `scripts/man-census.sh`, `scripts/spec-census.sh --citations`, `--links`. —
      `man-census.sh canvas`: TYPES 228/230 against 192/194 on `main`'s compiler, so all
      36 new rows are described and the two missing are older than this plan.
      `spec-census.sh --citations`: "MISS-PATH 0 MISS-LINE 0 MISS-SYMBOL 0" of 2,699.
      `--links`: "links=1163 unresolved=0".
- [x] Add a `.ai/canvas-threading.md` §4 sentence recording that a particle system is
      advanced by trigger 1 and is **not** an eighth trigger. — landed with letter A
      Phase 4, where the Hard Completion Gate put it; `grep -c 'NOT an eighth trigger'
      .ai/canvas-threading.md` → 1.
- [x] (added) The Validation Plan's runtime proof, on macOS: each example's four-second
      copy run in a REAL window (`/tmp/p150/window_proof.sh`, `MFB_CANVAS_STATS`) —
      fire, fog, fireworks, rain each `gpuSelected=TRUE gpuFrames=241 directFrames=241
      displayHz=120.00`. Linux in a window: Corrections 6.

Acceptance: all four examples build and run headless to completion, and both census
scripts report no missing entries for the new types.
  Check: `scripts/build-examples.sh` → pass, `scripts/man-census.sh` and
  `scripts/spec-census.sh --citations` → no missing entries, 0 dangling
  (est. 10 min, guess).
  Result (2026-10-13): the four build for macOS, both Linux targets and Windows (16 of
  16) and each run headless exits 0 (`MFB_HEADLESS=1 …/fire < /dev/null`, and the
  other three); the censuses as recorded on their line.
Commit: —

## Validation Plan

- **Tests:** `tests/cli/cli_canvas_man_examples_compile.rs` (examples compile);
  `scripts/test-canvas-vulkan.sh <mfb-exe> --box 2228` (Linux parity);
  `scripts/test-canvas-gpu-rows.sh <mfb-exe> --target windows-x86_64` (Windows parity);
  `scripts/build-examples.sh` (the four new example programs).
- **Coverage check:** not applicable — this letter adds no executable compiler code.
  Say so explicitly rather than running a gate that cannot fail.
- **Runtime proof:** each of the four examples run in a real window on macOS and Linux,
  `time` advancing, with `canvas::getFrameStats` (or `MFB_CANVAS_STATS`) confirming GPU
  frames on both.
- **Doc sync:** `mfb man canvas`; the spec per `scripts/spec-census.sh`;
  `.ai/canvas-threading.md` §4.
- **Final gate (run ONCE, after Phase 3):** `scripts/test-accept.sh <mfb-exe> <actual-output-dir>` (both arguments are required; the script exits 2 without them) — the full suite
  for the whole plan-150 feature, run here because this is the last letter. Per-phase
  checks in letters A and B stayed scoped precisely so this runs once.

## Open Decisions

- **Whether `examples/canvas/` holds four programs or one menu-driven demo** — four
  recommended. A user copying "fire" should not have to extract it from a switch
  statement, and `scripts/build-examples.sh` builds directories.
- **Whether the fog example ships a soft-particle PNG** — probably yes. Check what
  `scripts/build-examples.sh` does with non-source files before committing to it; a
  `Circle` template filled with a radial gradient is **not** the fallback it once was,
  because a particle's tint does not reach a gradient ramp (letter A Open Decision 8) —
  a gradient fog particle would not fade. If no asset ships, the fallback is an image
  built at start-up with `canvas::createImage` and `canvas::setBytes`.

## Corrections

1. **The Linux lane ran on box 2226, not 2228** (2026-10-13). 2228 (Ubuntu x86_64) was
   not running; 2226 (Debian 12 aarch64) was, and is a box both scripts already name.
   Its Vulkan device is Mesa's llvmpipe — a real driver and a real `.spv` consumer, on
   a CPU. So the claim "the particle scene renders on Vulkan" rests on two independent
   drivers, llvmpipe and Intel's, and on one hardware GPU, the Windows box's. An x86_64
   Linux run is not part of the evidence.
2. **The grouped scene was at the edge of the budget, and was changed before it was
   relied on** (2026-10-13). With particles fading in from alpha 110 it measured
   1.9771% of pixels differing on llvmpipe and 1.9668% on Intel, against
   `Tolerance::GPU_DEFAULT`'s 2% — passing, on two drivers, by a hair. Nothing was
   wrong: no pixel was more than 2 levels off; the scene was simply almost all blended
   pixels. A row that close fails on the next driver for no reason, so the particles
   were made opaque (the tint is now a colour ramp): 0.8363% and 0.8698%, Metal still
   within tolerance. The scene has no stored reference, so nothing was re-baselined.
3. **The manual bans the word the API uses** (2026-10-13). `lifetime` is on the
   memory-vocabulary list (`.ai/man-content.md` §4.2) and is the name of two particle
   fields and, through them, of an error. `man-census.sh --memory-scope canvas` went
   from 0 unclassified to 12, every one the field's name. Asked, the owner chose a
   carve-out over renaming: carve-out 6, the canvas pages and the `errorCode` overview,
   the word alone (a line that still trips the ban without it is still a hit).
4. **A manual sentence written in letter A was false, and only a build found it.** The
   `Motion` prose said `math::PI`; the constant is `math::pi`. `.ai/man-content.md` §5
   is right that nothing but running a claim verifies it: the type pages have no
   example of their own to compile, so the mistake survived until the example programs
   used the spelling. Fixed in the prose; the examples and the two manual examples now
   compile the right one.
5. **There is no `Picture` particle, so the fog is not sprites** (letter A Corrections
   3). This letter's Open Decision expected the fog to ship a soft PNG and noted that a
   gradient-filled circle "would not fade". Both halves are now the design: the fog's
   puff is a disc filled with a radial gradient from a faint grey to nothing, which is
   what gives it a soft edge; a tint does not reach a gradient, so the puffs only grow.
   A flat-filled version was rendered first and looked like overlapping discs.
6. **A Linux window was not run.** The Validation Plan asks for each example in a real
   window "on macOS and Linux". No reachable Linux box has a display
   (`scripts/test-canvas-vulkan.sh`'s header: "no reachable Linux box has a display"),
   and box 2228 was down. What stands in is what exists: on macOS, a real window, four
   of four with every frame presented directly by Metal; on Linux, the same particle
   scenes drawn by a Vulkan driver headless, on box 2226; on Windows, on a hardware
   GPU. A windowed Linux Vulkan frame is read back and blitted today in any case
   (bug-1082), so a window there would exercise the same draw.

## Summary

The only phase here that can genuinely fail is Phase 1, and it is first. Everything
after it is prose, examples and census work — real work, but work whose failure mode is
"incomplete" rather than "wrong".

The thing to resist is regenerating the SPIR-V when Vulkan disagrees. The blobs being
untouched *is* the evidence that the design's central claim held; if the Vulkan picture
is wrong, the bug is in how letter A composed the particle's `position`, `transform` and
tint, and it will be wrong on Metal too under some scene that the macOS test simply did
not cover.
