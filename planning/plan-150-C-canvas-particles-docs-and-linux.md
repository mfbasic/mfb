# plan-150-C: Particle systems — documentation, examples, and Linux/Vulkan parity

Last updated: 2026-09-22
Effort: medium (1h–2h)
Depends on: plan-150-B

Letters A and B build and measure the particle system. This letter is what makes it a
**shipped feature rather than a working one**: the manual entries a user learns it
from, a worked example per effect, the spec sync the project requires of any new
package surface, and the Vulkan-side proof that the "no shader change" claim holds on
Linux and not only on Metal.

Behavioral outcome: a user who has never seen the feature can read `mfb man canvas`,
copy a runnable example, and get fire, fog, or fireworks on macOS, Linux and Windows —
and the Linux GPU path is proved to match the software oracle, not assumed to.

References:

- `planning/plan-150-A-canvas-particles-types-and-expansion.md` — the type set the
  manual documents.
- `planning/plan-150-B-canvas-particles-cost-and-damage.md` — the measured cost model
  the manual must quote instead of adjectives.
- `scripts/test-canvas-vulkan.sh` — the Linux canvas parity harness.
- `scripts/man-examples-gate.sh`, `tests/cli/cli_canvas_man_examples_compile.rs` — every
  manual example must compile; this is gated, not aspirational.
- `.ai/remote_systems.md` — the Linux boxes and how to reach them.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-150-B complete and archived | `ls planning/plan-150-B-* → no matches` | NOT MET |
| The measured cost table exists in letter B | `grep -c 'particles/frame' planning/completed/plan-150-B-*.md` → ≥1 | NOT MET |
| A Linux box with a Vulkan device is reachable | `scripts/test-canvas-vulkan.sh` defaults to `--box 2228` (Ubuntu x86_64 glibc); probe per `.ai/remote_systems.md` | UNVERIFIED — check first |

If plan-150-B is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `mfb man canvas` documents `ParticleSystem` and its six companion types with a
  runnable example each, and the examples are gated to compile.
- Four worked example programs exist — fire, fog, fireworks, rain — under
  `examples/canvas/`.
- The Vulkan backend renders a particle scene within `Tolerance::GPU_DEFAULT` of the
  software oracle on a real Linux host, with **no** diff in either `.spv` blob.
- The spec is in sync per the project's own census.

### Non-goals (explicit constraints)

- **No behavioural change.** If this letter finds a bug, it is filed via `write-bug`
  and fixed there — not absorbed here. A documentation letter that quietly changes
  rendering is a letter nobody can review.
- **No `scripts/regen-spirv.sh` run.** The premise under test is that the shaders are
  untouched. Regenerating SPIR-V here would destroy the evidence.
- **No new public functions.** The surface is the types letter A added.

## 2. Current State

The `canvas` package registers 20 records, 1 union, 5 enums and 62 functions
(`grep -c 'pkg.add_record(RegistryRecord {' src/codegen/builtins/canvas/mod.rs` → 20;
`grep -c 'pkg.add_function' src/codegen/builtins/canvas/*.rs | awk -F: '{s+=$2} END {print s}'`
→ 62). Letter A adds 5 records, 1 union and 1 enum to that.

Manual examples are **gated**: `tests/cli/cli_canvas_man_examples_compile.rs` compiles
every example in the canvas manual, so a documented snippet that does not build fails
the suite. `scripts/man-examples-gate.sh` and `scripts/man-run-examples.sh` are the
project-level equivalents.

Linux GPU parity has a dedicated harness, `scripts/test-canvas-vulkan.sh`, which builds
a canvas program with the twelve-glyph test TrueType font and compares GPU against
software — the same oracle relationship `tests/canvas/rt_canvas_metal.rs` uses on
macOS. `examples/` currently holds 16 example projects and **no canvas directory**
(`ls examples/canvas` → no such directory).

### Measured populations

| What | Count | Command |
|---|---|---|
| Canvas records / unions / enums before letter A | 20 / 1 / 5 | `grep -c 'pkg.add_record(RegistryRecord {' src/codegen/builtins/canvas/mod.rs` → 20, etc. |
| Canvas registry functions | 62 | `grep -c 'pkg.add_function' src/codegen/builtins/canvas/*.rs \| awk -F: '{s+=$2} END {print s}'` → 62 |
| Existing example projects | 16 | `ls examples \| wc -l` → 16 |
| Existing canvas examples | 0 | `ls examples/canvas` → no such directory |
| Canvas goldens | 7 | `ls tests/golden/canvas \| wc -l` → 7 |

### Verified properties

- **Manual examples are compile-gated.** VERIFIED by reading
  `tests/cli/cli_canvas_man_examples_compile.rs` — the canvas manual's examples are
  extracted and built, so every snippet this letter writes must be runnable.
- **UNVERIFIED — whether the Vulkan path draws particles correctly.** Letter A proved
  the claim on Metal only. Both backends read the same `ItemBlock`, so the claim
  *should* transfer, but "should" is what Phase 2 exists to replace.

## 3. Design Overview

Three independent pieces, orderable by risk:

1. **Vulkan parity** (Phase 1) — the only phase that can discover a *defect*, so it
   goes first. If the Vulkan path disagrees with software, everything downstream is
   documenting something that does not work on Linux.
2. **Manual entries** (Phase 2) — the six types, each with a compiling example, quoting
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

- [ ] Extend `scripts/test-canvas-vulkan.sh` with the fireworks scene letter A
      goldened, rendered twice — `MFB_CANVAS_GPU=1` and software — and compared with
      the existing tolerance comparator.
- [ ] Run it on a real Linux host with a Vulkan device. The script ships an AppImage
      and takes `<mfb-exe> [--box <port>] [--libc glibc|musl]`, defaulting to box 2228
      (Ubuntu x86_64 glibc). The `--libc` must match the box — musl's loader absorbs the
      glibc compat sonames, so shipping the wrong one does not fail cleanly.
      `.ai/remote_systems.md` says **re-probe, do not assume**; three of that table's
      facts changed during plan-56 itself. Confirm the box reports a Vulkan device
      before reading a pass as meaningful.
- [ ] Confirm `git status --porcelain src/codegen/runtime/canvas/shaders/` is empty —
      no `.frag`, `.vert`, or `.spv` diff.
- [ ] If the comparison fails: **root-cause it, do not re-baseline.** A mismatch means
      the block composition in letter A Phase 4 is wrong on one backend; localize from
      the reported coordinate, fix, and record it in letter A's Corrections section.

Acceptance: the particle scene renders on Vulkan within `Tolerance::GPU_DEFAULT` of the
software oracle on a Linux host reporting a Vulkan device, and both `.spv` blobs are
unchanged.
  Check: `scripts/test-canvas-vulkan.sh <mfb-exe> --box 2228` → pass (est. 15 min — longer than 10 because it
  builds on a remote box; no smaller check exists, since the whole question is whether a
  real Vulkan driver agrees with the oracle).
Commit: —

### Phase 2 — Manual entries

- [ ] Write the `mfb man canvas` prose for `ParticleSystem`, `Emitter`, `Emission`,
      `Motion`, `Appearance`, `EmissionMode` — each field's units named, the tint rule
      stated as the multiply it is and cross-referenced to `Picture`'s identical rule.
- [ ] State explicitly, in `ParticleSystem`'s prose, that `time` is advanced **by the
      program** and that re-presenting is what draws the next frame — there is no timer
      (`.ai/canvas-threading.md` §4).
- [ ] Quote letter B's measured particle counts. No adjectives where a number exists.
- [ ] Document the two raises (77050025, 77050026) and the silently-nothing cases from
      letter A §7.
- [ ] Add one compiling example per type to the manual.

Acceptance: `mfb man canvas types` documents all six types, and every manual example
compiles under the existing gate.
  Check: `cargo test --test cli_canvas_man_examples_compile` → pass (est. 5 min).
Commit: —

### Phase 3 — Worked examples and spec sync

- [ ] Add `examples/canvas/` with four programs: `fire` (an `EmitLine` base, upward
      drift, orange→transparent ramp, turbulence), `fog` (an `EmitRect` volume, near-zero
      speed, large soft `Picture` particles, low alpha), `fireworks` (a `Burst`
      `EmitPoint` with gravity and a colour ramp), `rain` (an `EmitRect` at the top,
      high downward speed, a thin `Line` template).
- [ ] Wire them into `scripts/build-examples.sh` so they are built by the suite.
- [ ] Run the project's census scripts and resolve what they report:
      `scripts/man-census.sh`, `scripts/spec-census.sh`.
- [ ] Add a `.ai/canvas-threading.md` §4 sentence recording that a particle system is
      advanced by trigger 1 and is **not** a sixth trigger.

Acceptance: all four examples build and run headless to completion, and both census
scripts report no missing entries for the new types.
  Check: `scripts/build-examples.sh` → pass, `scripts/man-census.sh && scripts/spec-census.sh`
  → no missing entries (est. 10 min).
Commit: —

## Validation Plan

- **Tests:** `tests/cli/cli_canvas_man_examples_compile.rs` (examples compile);
  `scripts/test-canvas-vulkan.sh <mfb-exe> --box 2228` (Linux parity); `scripts/build-examples.sh` (the four
  new example programs).
- **Coverage check:** not applicable — this letter adds no executable compiler code.
  Say so explicitly rather than running a gate that cannot fail.
- **Runtime proof:** each of the four examples run in a real window on macOS and Linux,
  `time` advancing, with `MFB_CANVAS_STATS` confirming GPU frames on both.
- **Doc sync:** `mfb man canvas`; the spec per `scripts/spec-census.sh`;
  `.ai/canvas-threading.md` §4.
- **Final gate (run ONCE, after Phase 3):** `scripts/test-accept.sh <mfb-exe> <actual-output-dir>` (both arguments are required; the script exits 2 without them) — the full suite
  for the whole plan-150 feature, run here because this is the last letter. Per-phase
  checks in letters A and B stayed scoped precisely so this runs once.

## Open Decisions

- **Whether `examples/canvas/` holds four programs or one menu-driven demo** — four
  recommended. A user copying "fire" should not have to extract it from a switch
  statement, and `scripts/build-examples.sh` builds directories.
- **Whether the fog example ships a soft-particle PNG** — probably yes, and it would be
  the first binary asset under `examples/`. Check what `scripts/build-examples.sh`
  does with non-source files before committing to it; a procedurally-generated
  radial-gradient `Circle` template is the fallback and needs no asset.

## Corrections

<Filled in DURING execution.>

## Summary

The only phase here that can genuinely fail is Phase 1, and it is first. Everything
after it is prose, examples and census work — real work, but work whose failure mode is
"incomplete" rather than "wrong".

The thing to resist is regenerating the SPIR-V when Vulkan disagrees. The blobs being
untouched *is* the evidence that the design's central claim held; if the Vulkan picture
is wrong, the bug is in how letter A composed the transform and tint, and it will be
wrong on Metal too under some scene that the macOS test simply did not cover.
