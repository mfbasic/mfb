# plan-172-M: Closeout — documents, cross-platform proof and the full gates

Last updated: 2026-10-11
Effort: large (3h–1d)
Depends on: plan-172-L

Letters B–L each landed with scoped checks on the development host. This letter is
where the whole of plan-172 is proved: the type surface is compared against
`planning/new_canvas.md` field by field, the manual and spec are read as a user and a
contributor would read them, every platform runs the canvas suites, and the project's
full gates run once.

Behavioral outcome: `mfb man canvas types` shows exactly the types in
`planning/new_canvas.md`, every example in the tree builds and runs, and the canvas
suites pass on macOS, Linux (aarch64 and x86_64) and Windows.

References:

- `planning/new_canvas.md` — the surface to audit against.
- `.ai/testing-gates.md`, `.ai/compiler.md` (Hard Completion Gate),
  `.ai/specifications.md`, `.ai/man-content.md`, `.ai/build-tooling.md`,
  `.ai/remote_systems.md`.
- `scripts/artifact-gate.sh`, `scripts/test-accept.sh`, `scripts/man-examples-gate.sh`,
  `scripts/man-census.sh`, `scripts/test-canvas-vulkan.sh`,
  `scripts/test-canvas-gpu-rows.sh`, `scripts/test-macapp.sh`,
  `scripts/test-appimage.sh`.
- `planning/plan-150-A-canvas-particles-types-and-expansion.md` — the plan that takes
  this one as its prerequisite.

## Prerequisites

The plan-wide gate is in `plan-172-A` §Prerequisites. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-172-L complete and archived | `ls planning/plan-172-[A-L]-*` → no matches | MET (2026-10-11: "no matches found") |
| Every remote box this letter names is reachable | probe each per `.ai/remote_systems.md`: 2223, 2226, 2228, `test@192.168.1.206` | NOT MET (2026-10-11: 2226 "Linux aarch64" and 206 "Microsoft Windows [Version 10.0.26200.9457]" answer; 2223 and 2228 "Connection refused". 2227, x86_64 musl, answers and stands in for 2228 — Corrections) |

If plan-172-L is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- Every type, field name, field order and enum member above `## Future` in
  `planning/new_canvas.md` matches the registry, checked mechanically.
- No removed name survives anywhere in the tree outside a removed-type fixture.
- `mfb man canvas` and `mfb spec app canvas` describe the new surface with no stale
  sentence.
- The canvas suites pass on four operating-system / architecture rows.
- The project's full gates pass.
- plan-172 is archived and plan-150's prerequisite row reads MET.

### Non-goals (explicit constraints)

- **No behaviour is added or changed here.** A defect found by a gate is fixed in this
  letter, with its test, and recorded in Corrections; it is not a new feature.
- **No golden is regenerated to make a gate pass.** `AGENTS.md`'s four questions apply
  to each one.
- **The `## Future` section of `planning/new_canvas.md` is not implemented**: attributed
  text, the 3D object and particles stay out.

## 2. Current State

Written against the tree as letters B–L leave it. Each of those letters updated
`src/docs/spec/app/06_canvas.md` and the registry descriptions for its own change, ran
the canvas suites it touched on the development host, and ran GPU rows on macOS Metal
and on box 2226. Nothing has yet run the whole suite, the artifact gate, the acceptance
corpus, the man-example gate, an x86_64 row or a Windows row end to end.

### Measured populations

All are re-measured on entry; the expected values are what a complete plan leaves.

| What | Expected | Command |
|---|---|---|
| Removed names left in the tree | 0 outside removed-type fixtures | `grep -rlE 'canvas::(Point\|RoundedRect)\b\|cornerRadius\|paint\.transform' src tests examples tools scripts \| grep -vc '_removed_invalid'` → 0 |
| `Group` sites still setting `dx` | 0 | `grep -rhoE 'canvas::Group\[[^]]*' src tests examples tools scripts \| grep -c 'dx *:='` → 0 |
| `Group` constructors that name their fields and have no `id` | 0 | `grep -rhoE 'canvas::Group\[[^]]*' src tests examples tools scripts \| grep 'position *:=' \| grep -vc 'id *:='` → 0 (172-I adds the field) |
| Canvas PNG goldens | 11 (8 + `corners`, `group_affine`, `picture_shapes`) | `ls tests/golden/canvas/*.png \| wc -l` → 11 |
| Canvas registry unions | 3 | `grep -c 'pkg.add_union(' src/codegen/builtins/canvas/mod.rs` → 3 |
| Example source files that import canvas | 35 today | `grep -rl 'IMPORT canvas' examples --include='*.mfb' \| wc -l` → 35; each example they belong to is built and run in Phase 2 |
| Live documents outside this plan naming a removed type | 2 today | `grep -rlE 'RoundedRect\|canvas::Point\|Paint\.transform' .ai planning --include='*.md' \| grep -v 'completed\|plan-172\|new_canvas'` → `planning/plan-152-canvas-program-side-cost-spike.md`, `planning/plan-164-A-bezier-types-reduction.md` |

### Verified properties

- **The artifact gate does not cover canvas rendering.** VERIFIED in
  `.ai/testing-gates.md`. It is run here for what it does cover — the two IR-embedding
  canvas fixtures and everything else the plan's registry changes may have churned.
- **A plain host run of the acceptance corpus has four known reds.** VERIFIED in
  `.ai/testing-gates.md`: `libsnd-load-sound-rt`, `libsnd-playback-rt`,
  `native-link-inline-trap-rt`, `tls-connect-google-rt`. None is canvas.

## 3. Design Overview

Four audits, then the gates, ordered so the cheap ones fail first.

1. **Surface audit.** A throwaway script in `/tmp/plan-172-M/` extracts each `TYPE`,
   `UNION` and `ENUM` block above `## Future` from `planning/new_canvas.md` and compares
   names and order against `cargo run -- man canvas types`.
2. **Residue audit.** The removed-name greps above, over the whole tree and over the
   prose in `.ai/` and `planning/`.
3. **Reader audit.** `mfb man canvas --all` read as an MFBASIC developer; `mfb spec app
   canvas` read as a contributor, against `.ai/spec-content.md`.
4. **Platform audit.** The canvas suites on four rows.

Then the full gates, once.

**Gate class: behaviour.** Nothing here is a byte-identity claim.

## Compatibility / Format Impact

None beyond what letters B–L already recorded. This letter changes documents and fixes
what the gates find.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — Surface and residue audits

- [x] Re-measure the two "today" rows and record them. `/tmp/plan-172-M/measure.sh`,
      on the merged tree: example sources importing canvas 35; live documents naming a
      removed type 2, the same two (`planning/plan-152-canvas-program-side-cost-spike.md`,
      `planning/plan-164-A-bezier-types-reduction.md`). And the rows that have an
      expected value: `Group` sites setting `dx` 0; field-naming `Group` constructors
      with no `id` 0; canvas PNG goldens 11; registry unions 3.
- [x] Run the surface comparison; fix any difference in the registry, or — where the
      plan deliberately departed from the file (for example a rename an Open Decision
      settled) — correct `planning/new_canvas.md` and say why in Corrections.
      `/tmp/plan-172-M/surface.py planning/new_canvas.md <mfb man canvas types>`: 18
      declarations compared (the three unions with what each `INCLUDES`, three enums,
      twelve records), field names, types and order. One difference, in the document:
      `AutoPaint`'s four paint fields. Corrected there (Corrections). After: "differences:
      0".
- [x] Run the residue greps; port or delete each hit. Seven files, none a user-facing
      use: `__canvas_rectHeader`'s fifth parameter was still named `cornerRadius` (now
      `corner`), three comments named `RoundedRect.cornerRadius` as the arithmetic's
      origin (reworded), and four goldens echoed the parameter. After: 0.
- [x] Update each stale `.ai/` document found by the last row; `planning/` documents of
      other in-flight plans are reported to the owner, not edited. The last row finds
      nothing in `.ai/`. The two `planning/` documents are other plans' (152, a spike's
      record, and 164-A) and are reported, not edited.

Acceptance: the surface comparison reports no difference and the residue count is 0.
  Check: the removed-names grep above → 0 (est. 2 min).
  Observed 2026-10-11: "differences: 0" and "removed names outside removed-type
  fixtures: 0". `scripts/artifact-gate.sh target/release/mfb all` after the resync:
  "2350 golden(s) checked, 0 diff(s)".
Commit: —

### Phase 2 — Every example and tool builds and runs

- [x] Build each canvas example and each `tools/` program that imports canvas with a
      release `mfb`; run each headless with `MFB_CANVAS_SYNC=1` for one frame.
      `/tmp/plan-172-M/examples.sh`: each project is built twice in a scratch copy (as
      it ships, and `--debug`) and the debug build run headless with stdin held open
      six seconds. First run: 7 of 8 — **`examples/canvas-demo` did not build**
      (Corrections). After the repair: "projects ok=8 failed=0", every one "exit
      code=0" with a dumped frame — canvas-demo (4 frames), dungeon (1), fractals
      (13), fractals-image (72), sorts (29), swat/canvas (37), water (107), weather
      (4). The three `tools/` programs: `tools/water-probe/run.sh` → "PASS: bug-716 …";
      `tools/wind-map-probe/run.sh` → first a build error, a `canvas::Size` still
      written with Integers (172-E's change, missed there; fixed), then "PASS:
      wind-map-probe …"; `tools/key-probe` builds as an app (`build.py app`) and is
      not run — its canvas leg is driven by keystrokes injected into a real window.
- [x] `bash scripts/man-examples-gate.sh target/release/mfb`. "man examples: 1127
      checked, 0 failed, 0 stale not-run entr(y/ies), 1489s"; the `canvas` row is 35
      built, 35 run. Rerun for canvas alone after Phase 3 changed its pages:
      `scripts/man-run-examples.sh canvas --run` → "examples: 35 built: 35 ran: 35 not
      run: 0 failed: 0".
- [x] `tools/canvas-bench`: run `bench.sh target/release/mfb sweep` and compare with the
      figures 172-F recorded; a regression is root-caused here.
      Seven sweeps of the merged tree against seven with the compiler built at 172-F's
      end (`04cc405cf`), the build phase (`phaseOffsetsMs`) as 172-F compared it. The
      machine was loaded and a row's seven runs spread by up to a factor of two, so
      the sweeps alone could not say: the minimums were within 3% on five moving rows,
      and 7% up on `5000/3` and 11% up on the static `1000/8` (0.061 → 0.068 ms).
      Those two rows were then run twelve times each, alternated, built once per
      compiler (`/tmp/plan-172-M/row_ab.sh`): static `1000/8` minimum 0.0591 → 0.0587
      ms, median 0.0668 → 0.0665 (ratio 0.99); moving `5000/3` minimum 1.1329 → 1.1399,
      median 1.1399 → 1.1469 (ratio 1.006). No regression: nothing since 172-F — the
      group matrix, the hit test, the pointer, `AutoPaint`, the picture, or main's 75
      commits — costs these scenes more than 1% in the build phase. (172-F's own
      recorded figures are within 8% of what its compiler measures today, some rows
      above and some below.)

Acceptance: every example and tool exits 0, and the man-example gate passes.
  Check: `bash scripts/man-examples-gate.sh target/release/mfb` → pass (est.
  UNMEASURED — time it and record it; it is the only gate that builds and runs every
  manual example).
  Observed 2026-10-11: pass, 1,489 s (24 min 49 s) on a loaded machine.
Commit: —

### Phase 3 — Reader audits

- [x] `mfb man canvas --all` and `mfb man canvas types`: every description checked
      against `.ai/man-content.md`, including the memory-vocabulary ban, with
      `scripts/man-census.sh --memory-scope canvas`. The census first: one
      memory-vocabulary hit ("the one under the pointer", in `pollMouse`) and one
      internals hit (an example's `[[vector::Float2[…` read as a citation marker);
      both reworded, and both censuses now report 0. Then the rendered pages (3,945
      lines) were read against the standard and against `planning/new_canvas.md` by a
      reviewer given the pages and not the conclusions. What it found and what was
      done is in Corrections; in short, nine sentences that still described the
      surface before this plan, two snippets with unqualified type names, three
      places where two pages disagreed, and dead "See also" links.
- [x] `mfb spec app canvas` read whole against `.ai/spec-content.md`; no leaked `[[`
      marker in `mfb spec app --all`; every citation's symbol grep-confirmed.
      `/tmp/plan-172-M/citations.py`: "citations 177 unresolved 0" (each file exists
      and names its symbol). `mfb spec app --all | grep -c '\[\['` → 5, all five
      Objective-C message sends in the macOS topics (`[[NSWindow alloc] …`), none a
      marker. The rendered topic was read against the source by a second reviewer:
      five statements corrected (Corrections).
- [x] `.ai/canvas-threading.md` read whole: §1, §4, §8, §9, §10, §11 and §13 agree with
      what G, H, J and K landed. §1, §4, §9, §10 and §13 do, checked against the source
      (the sixteen-word draw entry, the matrix words, `waitForRedraw`'s three answers,
      `signalRedraw(1)`, damage off by default). Stale and corrected: §2 (the scene
      region as arena state), §5 ("stay five"), §6 and §14 (three statements about a
      picture's record that 172-L made false), §8's R4 to R6 (their "Protected by"
      column named text §3 no longer has), §11 (a count), §16 ("twelve" names; it is
      fifteen), two test paths and three cross-references.

Acceptance: the census and the spec tests pass and the rendered pages carry no removed
name.
  Check: `cargo test --bin mfb spec` → pass, and `cargo run -- man canvas --all | grep
  -cE 'RoundedRect|canvas::Point|Paint\.transform'` → 0 (est. 4 min).
  Observed 2026-10-11: `cargo test --bin mfb -- man:: spec` → "98 passed; 0 failed";
  the grep → 0; and the same grep over `mfb spec app canvas` → 0 (it was 1: "every
  rectangle that was a `RoundedRect`", reworded).
Commit: —

### Phase 4 — Platform rows

- [ ] macOS (local): `cargo test --test 'rt_canvas_*'`, then `bash
      scripts/test-macapp.sh`.
- [ ] Linux aarch64 glibc, box 2223: the same `cargo test` row. Chosen because it is the
      native aarch64 build box.
- [ ] Linux GTK and Vulkan, box 2226: `scripts/test-canvas-vulkan.sh target/release/mfb
      --box 2226` and `scripts/test-canvas-gpu-rows.sh`; the pointer and `AutoPaint`
      cases in a real GNOME session.
- [ ] Linux x86_64, box 2228: `scripts/test-canvas-vulkan.sh target/release/mfb --box
      2228`. Emulated and slow; it is the only place the x86_64 emitters run.
- [ ] Windows, `test@192.168.1.206`: the canvas suites, the pointer and `AutoPaint`
      cases, and `overID` through a real window — `MFB_MOUSE_INJECT` does not exist
      there, so this is the first Windows proof of 172-I. The proof scene has one group
      drawn twice under two `id`s and a named picture, so the id path is read off a real
      click, not only the item's own `id`.
- [ ] `scripts/test-appimage.sh --libc both` for the Linux app mode.
- [ ] Record each row's command and result in Corrections.

Acceptance: all six rows pass.
  Check: the six commands above → pass (est. 90 min in total; each row exercises a back
  end no other row reaches, so none can stand in for another).
Commit: —

### Phase 5 — The full gates, once (largest blast radius)

- [ ] `rustup run 1.96.0 cargo fmt --all`, then `git diff --stat` to confirm only this
      plan's files moved.
- [ ] `cargo check --all-targets` warning-free; `cargo clippy --all-targets`.
- [ ] `cargo test --no-fail-fast -- --skip artifact_gate_all > /tmp/plan-172-M/run.log
      2>&1; echo EXIT=$?`, then `grep -c '^failures:' /tmp/plan-172-M/run.log` → 0.
- [ ] `scripts/artifact-gate.sh target/release/mfb all`.
- [ ] `scripts/test-accept.sh target/debug/mfb target/accept-actual`; the summary's test
      count equals the count at plan-172's starting commit plus the fixtures this plan
      added, and the only reds are the four baseline ones.
- [ ] `cargo test --test scripts_index_is_complete`.
- [ ] Archive `plan-172-M` and `planning/new_canvas.md` to `planning/completed/`.
- [ ] Re-run plan-150-A's prerequisite commands and set its plan-172 row to MET.

Acceptance: every gate above passes.
  Check: the artifact gate (est. 15–20 min per `.ai/testing-gates.md`) and the
  acceptance corpus (est. 15 min) — both tree-wide by design, run once, here; the full
  `cargo test` has no documented runtime, so time it and record it.
Commit: —

## Validation Plan

- Tests: no new test files; this letter runs everything B–L added.
- Coverage check: `scripts/coverage-check.sh` — `helper_hit.rs`, `helper_autopaint.rs`
  and `func_pointer.rs` are in the denominator.
- Runtime proof: `examples/canvas-demo` in a real window on macOS, Linux and Windows,
  with an `AutoPaint` button, a shaped picture and a rotated group visible.
- Doc sync: `src/docs/spec/app/06_canvas.md`,
  `src/docs/spec/diagnostics/02_error-codes.md`, `.ai/canvas-threading.md`, the registry
  descriptions.
- Final gate (run ONCE, in Phase 5): the full `cargo test`, the artifact gate, the
  acceptance corpus, the man-example gate, fmt and clippy.

## Open Decisions

None.

## Corrections

- **`examples/canvas-demo` did not build, and had not since 172-K (Phase 2).** Two of
  its functions — `kit.mfb:placed` and `ui.mfb:restyleItem` — `MATCH` over every
  `DrawItem`. 172-K added the `AutoPaint` variant and 172-L took a picture's
  `position`, `paint` and `transform` away, and neither letter built this example
  afterwards (172-L's record said it did; corrected there). Both functions already
  hand a `Group` back unchanged; they now do the same for a `Picture` and an
  `AutoPaint`, which none of the demo's figures is, so nothing it draws changes — its
  menu frame was looked at. This is the repair that makes it build, NOT the showcase
  rewrite the owner asked for, which is still theirs to authorise.
- **What the reader audits changed (Phase 3).** In `mfb man canvas`: `Paint` no longer
  described as transforming; the package page no longer lists a transform among
  `Paint`'s fields; `Transform` says every shape turns about its `position` (it said
  four did and the rest turned about the surface corner); `Gradient` and `Paint.clip`
  say a group's transform, not its translation, and that clip rings are measured from
  `position`; `join` says "a rectangle with rounded corners"; `Group` no longer claims
  to be the only item with no paint; the `Shape` union's `Ellipse` is no longer
  "optionally rotated"; `MouseEvent.overID` names a picture's part of the path;
  `hoverPaint` agrees with `usePressed`; the flash length gives its unit and range;
  `Picture` says whose clip it means, what box the image is fitted to, that a group
  moves the whole picture, and that an `AutoPaint` shape needs no `enableMouse`;
  `destroyImage` and `setBytes` no longer say a scene carries an image's "id"; two
  description snippets qualify `canvas::Paint` and `canvas::BlendMode`. In the
  renderer: "See also" listed anything spelled `a::b` in a description, so
  `pollMouse`'s page offered `save::label` and `avatar::frame`; it now lists only
  what names a package (`src/cli/man.rs:render_see_also`).
  NOT changed, and reported: the reviewer also flagged prose that describes the
  renderer to a developer (the geometry cache, "no GPU readback", a measured cost in
  `pollMouse`). None is stale, none is this plan's, and both censuses pass them; they
  are a judgement about the standard, not a defect a gate found.
- **What the spec audit changed.** Which items carry a `transform` (it said every one
  but `Group` and `AutoPaint`; a `Picture` has none and a `Group` has one); the frame
  skip compares content, not "equal bytes"; a picture's record is the rectangle record
  and never the flattened one (it said "always path 1", which its own radius slot
  contradicts); a redraw is also owed by an expose; `ABSENT` defines fifteen names.
- **bug-1015 holds on Windows too (Phase 4's `overID` proof found it).** The proof
  program first recorded nothing in a real window: `canvas::pollMouse` reports no
  event until the program also reads the keyboard, because the only decoder of the
  mouse reports is in the `io` read family. That bug was filed for GTK with Windows
  as the model to match; the measurement and the repro are added to it
  (`bugs/repro/bug-1015/`). The proof program reads a key each loop, as every
  example does.
- **main was merged in FIRST (2026-10-11), so the gates run once, on the tree that will
  land.** The follow-plan order is gates, then merge; main was 75 commits ahead (369
  files, with codegen changes under the same goldens this plan moved), and gates run
  before a merge that size prove the wrong tree. Merge `18f739673`. Three hand
  resolutions, each in its message: main had fixed and archived bug-1013; the two
  `STATE` paragraphs (man and spec) are main's rewrite with plan-172-N's one fact kept
  — an `ENUM` field does not stop a record being defaultable, which the merged checker
  agrees with (`state_type_fault` asks `is_defaultable`); and 29 `.ncodesum` goldens,
  regenerated by the merged compiler ("2350 golden(s) checked, 0 diff(s)").
- **Boxes 2223 and 2228 are down, and cannot be started from here.** Both refuse the
  connection; they are UTM virtual machines on the development host, and `utmctl`
  answers "OSStatus error -1743" from this session. 2228's row — the x86_64 emitters
  under Vulkan — is run on 2227 instead, which is x86_64 too (musl, lavapipe by
  `--icd auto`). 2223's row, the canvas `cargo test` suites built natively on Linux
  aarch64, has no stand-in: 2226 is aarch64 but is not a build box. That row is NOT RUN
  and is reported.
- **`planning/new_canvas.md`'s `AutoPaint` block was brought into line with the
  registry.** It listed the paint fields as `enter`, `exit`, `hover`, `pressed`; they
  are `enterPaint`, `exitPaint`, `hoverPaint`, `pressedPaint` (`EXIT` is a keyword;
  172-K; the owner's "Field names are fine", 2026-10-11). The note under the block
  already said so; the block itself is what the surface audit reads.
- **A parameter's name is in four goldens.** Renaming `__canvas_rectHeader`'s
  `cornerRadius` moved `app-mouse-surface`'s four `.ncodesum` files as well as the IR
  dumps. Looked at before resyncing: the native code listing names each function's
  parameters (`"name": "corner"` under `#canvas_rectHeader`'s `params`), so the
  checksum is of that text; no instruction changed.
- **From 172-L as built (2026-10-11): what the closeout has to know about a `Picture`.**
  Read `mfb spec app canvas`, "A picture is drawn inside a shape", and
  `.ai/canvas-threading.md` section 17 first.
  - **§2's "Canvas PNG goldens" row expects 11 and `picture_shapes` is one of them**: it
    is there (`tests/golden/canvas/picture_shapes.png`).
  - **Two bugs were filed, both open, neither caused by 172.** `bug-1079`: on box 206's
    Intel GPU a picture takes the neighbouring texel where a texel boundary is exactly
    a pixel centre — so a Windows row that draws a picture of such a size fails for
    that reason and no other; `tests/canvas/scenes/picture_shapes_on_black.mfb` is
    sized to avoid it. `bug-1080`: a defaulted record updated with `WITH` costs about
    27 KB of code a statement, which is why that scene's program is large.
  - **The rows script has two more rows** (`pictureshapes`, `pictureauto`), and they
    pass on 2226, 2227 and 206 at `b83b09637`.
  - **`examples/canvas-demo` has no shaped picture and no `AutoPaint`.** Its `Picture`
    literals were ported mechanically (and it did not build until Phase 2 below). The Validation Plan's runtime proof asks for
    one "with an `AutoPaint` button, a shaped picture and a rotated group visible";
    the showcase rewrite that would put them there is the owner's to authorise and
    has not been (their words: "Do a rewrite, not a port").
  - **What a canvas program costs changed**: one with no picture is 16,904 bytes
    smaller than at 172-K's end and one with pictures 226,012 larger (`__text`, macOS
    AArch64). Phase 2's `canvas-bench` sweep compares against 172-F's figures; the
    `pics` rows were at parity with `88005fb60` when alternated with it.
- **From 172-I's id path (owner's decision, 2026-10-10):** one more closeout count (no
  field-naming `Group` constructor without an `id`), and the Windows `overID` proof
  covers a group path and a picture.

## Summary

Nothing here is new engineering. The risk this letter exists to retire is that eleven
letters, each green on its own scoped checks on one machine, do not add up to a green
tree on four.
