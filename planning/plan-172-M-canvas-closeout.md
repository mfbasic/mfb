# plan-172-M: Closeout — documents, cross-platform proof and the full gates

Last updated: 2026-10-10
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
| plan-172-L complete and archived | `ls planning/plan-172-[A-L]-*` → no matches | NOT MET |
| Every remote box this letter names is reachable | probe each per `.ai/remote_systems.md`: 2223, 2226, 2228, `test@192.168.1.206` | UNVERIFIED — run first |

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

- [ ] Re-measure the two "today" rows and record them.
- [ ] Run the surface comparison; fix any difference in the registry, or — where the
      plan deliberately departed from the file (for example a rename an Open Decision
      settled) — correct `planning/new_canvas.md` and say why in Corrections.
- [ ] Run the residue greps; port or delete each hit.
- [ ] Update each stale `.ai/` document found by the last row; `planning/` documents of
      other in-flight plans are reported to the owner, not edited.

Acceptance: the surface comparison reports no difference and the residue count is 0.
  Check: the removed-names grep above → 0 (est. 2 min).
Commit: —

### Phase 2 — Every example and tool builds and runs

- [ ] Build each canvas example and each `tools/` program that imports canvas with a
      release `mfb`; run each headless with `MFB_CANVAS_SYNC=1` for one frame.
- [ ] `bash scripts/man-examples-gate.sh target/release/mfb`.
- [ ] `tools/canvas-bench`: run `bench.sh target/release/mfb sweep` and compare with the
      figures 172-F recorded; a regression is root-caused here.

Acceptance: every example and tool exits 0, and the man-example gate passes.
  Check: `bash scripts/man-examples-gate.sh target/release/mfb` → pass (est.
  UNMEASURED — time it and record it; it is the only gate that builds and runs every
  manual example).
Commit: —

### Phase 3 — Reader audits

- [ ] `mfb man canvas --all` and `mfb man canvas types`: every description checked
      against `.ai/man-content.md`, including the memory-vocabulary ban, with
      `scripts/man-census.sh --memory-scope canvas`.
- [ ] `mfb spec app canvas` read whole against `.ai/spec-content.md`; no leaked `[[`
      marker in `mfb spec app --all`; every citation's symbol grep-confirmed.
- [ ] `.ai/canvas-threading.md` read whole: §1, §4, §8, §9, §10, §11 and §13 agree with
      what G, H, J and K landed.

Acceptance: the census and the spec tests pass and the rendered pages carry no removed
name.
  Check: `cargo test --bin mfb spec` → pass, and `cargo run -- man canvas --all | grep
  -cE 'RoundedRect|canvas::Point|Paint\.transform'` → 0 (est. 4 min).
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
      there, so this is the first Windows proof of 172-I.
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

<Filled in DURING execution.>

## Summary

Nothing here is new engineering. The risk this letter exists to retire is that eleven
letters, each green on its own scoped checks on one machine, do not add up to a green
tree on four.
