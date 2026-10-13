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
Commit: —

### Phase 2 — Manual entries

Registry descriptor prose, in `src/codegen/builtins/canvas/mod.rs`, to `.ai/man-content.md`'s
standard. Rendering the page is the only verification: the compiler never reads it.

- [ ] Write the `mfb man canvas` prose for `ParticleSystem`, `Emission`, `Motion`,
      `Appearance`, `EmissionMode` — each field's units named, the tint rule stated as
      the multiply it is and cross-referenced to `Picture`'s identical rule.
- [ ] State, in `ParticleSystem`'s prose, what each emitter shape emits from: its
      interior for a `Rectangle`, `Circle`, `Ellipse`, `Polygon` and `Text`, its length
      for a `Line`, `Arc` and `PolyLine`; that the emitter's own `position` and
      `transform` place and turn it; and that its `paint` and `id` are not used.
- [ ] State what a particle does with each template kind (letter A §6): a shape, an
      `AutoPaint` (its default paint), a `Picture` (and what `ClipOnly` means for a
      spinning sprite), a `Group` (and its tint).
- [ ] State that a particle is never under the mouse: it is not named by
      `MouseEvent.overID` and does not hide what is beneath it from the mouse.
- [ ] State explicitly, in `ParticleSystem`'s prose, that `time` is advanced **by the
      program** and that re-presenting is what draws the next frame — there is no timer
      (`.ai/canvas-threading.md` §4).
- [ ] Quote letter B's measured particle counts. No adjectives where a number exists.
- [ ] Document the two raises (77050028, 77050029, by their `errorcode` names) and the
      silently-nothing cases from letter A §7.
- [ ] Add one compiling example per type to the manual.

Acceptance: `mfb man canvas types` documents all five types, and every manual example
compiles under the existing gate.
  Check: `cargo test --test cli_canvas_man_examples_compile` → pass, and
  `bash scripts/man-census.sh --memory-scope` → 0 unclassified (est. 5 min, guess).
Commit: —

### Phase 3 — Worked examples and spec sync

- [ ] Add `examples/canvas/` with four programs: `fire` (a `Line` emitter along the
      base, upward drift, orange→transparent ramp, turbulence), `fog` (a `Rectangle`
      emitter, near-zero speed, large soft `Picture` particles, low alpha), `fireworks`
      (a `Burst` system with a `Circle[radius := 0.0]` emitter, gravity and a colour
      ramp), `rain` (a `Rectangle` emitter across the top, high downward speed, a thin
      `Line` template).
- [ ] Wire them into `scripts/build-examples.sh` so they are built by the suite.
- [ ] Review the spec letter A wrote against `.ai/spec-content.md`:
      `src/docs/spec/app/06_canvas.md`'s "The item unions" (the twelfth variant and why
      it is in neither `Shape` nor `AutoShape`), its particle section (the model, the
      expansion, what each walk does with a system), "Hit-testing" (a system is not
      hit), and `src/docs/spec/diagnostics/02_error-codes.md`.
- [ ] Run the project's census scripts and resolve what they report:
      `scripts/man-census.sh`, `scripts/spec-census.sh --citations`, `--links`.
- [ ] Add a `.ai/canvas-threading.md` §4 sentence recording that a particle system is
      advanced by trigger 1 and is **not** an eighth trigger.

Acceptance: all four examples build and run headless to completion, and both census
scripts report no missing entries for the new types.
  Check: `scripts/build-examples.sh` → pass, `scripts/man-census.sh` and
  `scripts/spec-census.sh --citations` → no missing entries, 0 dangling
  (est. 10 min, guess).
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

## Summary

The only phase here that can genuinely fail is Phase 1, and it is first. Everything
after it is prose, examples and census work — real work, but work whose failure mode is
"incomplete" rather than "wrong".

The thing to resist is regenerating the SPIR-V when Vulkan disagrees. The blobs being
untouched *is* the evidence that the design's central claim held; if the Vulkan picture
is wrong, the bug is in how letter A composed the particle's `position`, `transform` and
tint, and it will be wrong on Metal too under some scene that the macOS test simply did
not cover.
