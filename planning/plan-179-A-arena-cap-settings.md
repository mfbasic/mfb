# plan-179-A: Arena memory cap settings (manifest and IR plumbing)

Last updated: 2026-10-19
Overall Effort: large (3h–1d) — the whole plan-179 feature (sub-plans A, B, C)
Effort: small (<1h)
Depends on: nothing

This sub-plan adds four keys to the `options` section of `project.json`, validates
them at build time, and carries the resolved values to the code generator. It emits
no code and changes no runtime behavior. The cap itself is built in plan-179-B
(startup check) and plan-179-C (allocator enforcement).

References:

- `src/docs/spec/tooling/01_project-manifest.md` — the `options` section (table row at line 45, footnote 4 at line 87).
- `src/docs/spec/memory/04_arenas.md` — arena behavior the cap will govern (read for plan-179-C).
- `.ai/specifications.md` — a change to an observable contract updates its spec topic in the same change.
- `.ai/testing-gates.md` — `artifact-gate.sh` and `test-accept.sh` landing tier.

## Prerequisites

Nothing blocks this sub-plan. Sub-plan A is the first of the feature.

| Must be true | Command | Status |
|---|---|---|
| plan-179-A not yet landed (fresh start) | `ls planning/completed/plan-179-A-* 2>/dev/null` → no output | MET |

## 1. Goal

- A `project.json` with any combination of the four keys below builds successfully; an invalid value fails the build with a message that names the key.
- With the keys absent, the build is byte-identical to the build before this change on every target (no code is emitted from these keys yet).

### Non-goals (explicit constraints)

- No runtime behavior. No probe, no startup check, no allocator change (plan-179-B and -C).
- No change to the `.mfp` package encoding. The limits belong to the consuming executable, the same as `max_buffer_bytes` (`src/ir/types.rs`, the doc comment on `max_buffer_bytes`).
- No change to `ArenaState` layout.
- No new `options` key other than the four below.

## 2. Current State

`options` is read by `src/manifest/mod.rs` (`manifest.get("options")` at lines 55 and 74). Existing tunables follow one pattern: a `DEFAULT_*` constant, a reader that falls back to the default, and a validator that returns a build error naming the key. Examples:

- `DEFAULT_MAX_BUFFER_MIB` (`src/manifest/mod.rs:35`), read at line 60.
- `options.canvasAutoPaintMS`: reader `canvas_auto_paint_ms` (`src/manifest/mod.rs:119`), validator at line 136, message style at line 159, tests at lines 185–220.
- `stdinLogCap`: consumed at `src/target/shared/lower.rs:41` and carried through `src/target/shared/nir/mod.rs:23`.
- `max_buffer_bytes` is carried on `IrProject` (`src/ir/types.rs:204`), set in `src/ir/lower.rs:335` and `src/target/package_mfp/mod.rs:501`, and constructed for a test in `src/optimizer/opt1/local_rewrites.rs:419`.
- Warnings for `options`-adjacent keys exist: plan-61-D landed `kind`/`mode` as warnings (`src/manifest/mod.rs:550`, `:1526`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `IrProject` constructions that must gain the new field | 3 non-test, plus 1 test | `grep -rn "max_buffer_bytes:" src --include='*.rs'` → `src/ir/lower.rs:335`, `src/target/package_mfp/mod.rs:501`, `src/optimizer/opt1/local_rewrites.rs:419` |
| Existing `options` docs rows/footnotes | 2 | `grep -n -i "options" src/docs/spec/tooling/01_project-manifest.md` → lines 45, 87 |
| Man pages that document `options` keys | 0 | `grep -rln "maxBufferMiB\|canvasAutoPaintMS\|stdinLogCap" src/docs/man` → no output |

No man page documents `options` today, so this sub-plan adds spec text only. `.ai/man-content.md` governs man prose, which this sub-plan does not touch.

### Verified properties

- **Adding IR fields with defaults emits no code.** Verified by reading `src/ir/types.rs:204` and the lowering at `src/target/shared/lower.rs:41`: the value is read into NIR and used only where codegen consumes it. Plan-179-A adds no consumer, so no codegen path reads the new fields.
- **The reader pattern tolerates absent keys.** Verified by reading `canvas_auto_paint_ms` (`src/manifest/mod.rs:119–133`): it returns the default when `options` or the key is absent.

## 3. Design Overview

Four keys, all under `options`:

| Key | Type | Default | Valid values |
|---|---|---|---|
| `maxArenaMemoryDisable` | boolean | `false` | `true` or `false` |
| `maxArenaMemoryPercent` | integer | `50` | 1 to 100 |
| `maxArenaMemoryMiB` | integer | `-1` | `-1` (unset) or a positive integer whose value × 1048576 fits in u64 |
| `maxArenaMemoryCheck` | string | `"halt"` | `"warn"` or `"halt"` |

Resolved into one struct on `IrProject` (new, named `ArenaMemoryLimits`), carried to NIR alongside `stdinLogCap`.

Rules this sub-plan enforces at build time:

- A value of the wrong JSON type (a string where an integer is expected, a float, a missing boolean) is a build error naming the key.
- `maxArenaMemoryPercent` outside 1 to 100 is a build error.
- `maxArenaMemoryMiB` of `0`, or any negative value other than `-1`, is a build error. A value whose byte count overflows u64 is a build error.
- `maxArenaMemoryCheck` is exactly `"warn"` or `"halt"`; anything else is a build error.
- If `maxArenaMemoryDisable` is `true` and any of the other three keys is present, the build prints a warning naming each ignored key. The build succeeds.

Rejected alternatives:

- `-1` as the off switch for everything. Rejected by the owner, 2026-10-19: `maxArenaMemoryDisable` is the only off switch; `-1` is only "unset" for `maxArenaMemoryMiB`.
- A `min`-based requested limit computed at build time. Rejected: the RAM value is only known on the machine that runs the program (plan-179-B).

## 4. Detailed Design

### 4.1 Manifest readers and validators (`src/manifest/mod.rs`)

- Constants next to `DEFAULT_MAX_BUFFER_MIB` (line 35): `DEFAULT_ARENA_MEMORY_PERCENT: u64 = 50`, `DEFAULT_ARENA_MEMORY_MIB: i64 = -1`, `DEFAULT_ARENA_MEMORY_CHECK: &str = "halt"`.
- Readers in the `canvas_auto_paint_ms` style. Validators return a message in the style of `src/manifest/mod.rs:159`: `Field \`maxArenaMemoryPercent\` must be a whole number from 1 to 100, for example ...`.
- The disable-with-keys warning uses the same warning path as plan-61-D (`src/manifest/mod.rs:550`).

### 4.2 IR and NIR (`src/ir/types.rs`, `src/ir/lower.rs`, `src/target/package_mfp/mod.rs`, `src/optimizer/opt1/local_rewrites.rs`, `src/target/shared/nir/mod.rs`, `src/target/shared/lower.rs`)

- `ArenaMemoryLimits { disable: bool, percent: u64, mib: i64, check: ArenaMemoryCheck }` with `enum ArenaMemoryCheck { Warn, Halt }`.
- `IrProject.arena_memory` beside `max_buffer_bytes` (`src/ir/types.rs:204`), with a doc comment that it is the consuming executable's setting and is never encoded into a `.mfp`.
- Construction sites set the default value.
- `src/target/shared/nir/mod.rs:60` carries the struct; `src/target/shared/lower.rs:41` reads it from the manifest, the same place `stdinLogCap` is read.

### 4.3 Spec (`src/docs/spec/tooling/01_project-manifest.md`)

- Add the four keys to the options table (after line 45's row) and a subsection "Arena memory cap" with the table of keys, defaults and rules from §3.
- Say explicitly that the cap counts arena memory only, and name the non-counted mappings (stack, audio, Metal, perf region, debug registry).
- The startup behavior and allocator behavior are documented in plan-179-B and -C, in `src/docs/spec/memory/04_arenas.md` and `src/docs/spec/memory/08_program-startup.md`.

## Compatibility / Format Impact

- New keys are optional, with defaults. Existing `project.json` files build unchanged.
- No change to `.mfp` encoding and no change to emitted code in this sub-plan.

## Phases

### Phase 1 — Settings, validation, IR plumbing, spec

Lands alone: it adds no codegen consumer, so it cannot change a build's output.

- [ ] `src/manifest/mod.rs`: defaults (constants next to `DEFAULT_MAX_BUFFER_MIB`), four readers, four validators, disable-with-keys warning (§4.1).
- [ ] `src/manifest/mod.rs` tests module (near the `canvasAutoPaintMS` tests at lines 185–220): defaults when absent; each valid value accepted; each invalid value rejected with a message naming its key (wrong type, `0`, `-2`, `101`, `0` percent, overflow MiB, bad check string); disable + keys produces the warning.
- [ ] `src/ir/types.rs`: `ArenaMemoryLimits`, `ArenaMemoryCheck`, field `arena_memory` on `IrProject` (§4.2).
- [ ] Construction sites set the default: `src/ir/lower.rs:335`, `src/target/package_mfp/mod.rs:501`, `src/optimizer/opt1/local_rewrites.rs:419`.
- [ ] `src/target/shared/nir/mod.rs:60` and `src/target/shared/lower.rs:41`: carry the struct to NIR.
- [ ] `src/docs/spec/tooling/01_project-manifest.md`: options table rows and "Arena memory cap" subsection (§4.3).

Acceptance: every manifest test in the new set passes, and a build of a project with the four keys absent produces the same artifacts as before the change.
  Check: `cargo test --bin mfb manifest::` → all manifest tests pass (est. 5 min; a scoped run is the cheapest check that fails if a validator is wrong).
  Check: a `project.json` with `"maxArenaMemoryPercent": 101` → build fails with a message naming `maxArenaMemoryPercent`.
Commit: —

Landing tier (once, at the end of this sub-plan): `scripts/artifact-gate.sh <mfb> all` and a full `scripts/test-accept.sh`. Expected result: zero diff, because this sub-plan emits no code. Any diff is a bug in this sub-plan (AGENTS.md: "Unexpected diff in a golden … = bug-hunt trigger"); inspect one fixture to localize it.

## Validation Plan

- Tests: the manifest unit tests in §Phase 1.
- Coverage check: `cargo test --bin mfb manifest::` covers `src/manifest/mod.rs`, so the changed readers and validators are in the scope of the run.
- Runtime proof: none in this sub-plan; the first runtime proof is plan-179-B.
- Doc sync: `src/docs/spec/tooling/01_project-manifest.md`.
- Final gate (once, at the end): `scripts/artifact-gate.sh <mfb> all` and `scripts/test-accept.sh`, expected zero diff.

## Decisions

- Keys and defaults: `maxArenaMemoryDisable` `false`, `maxArenaMemoryPercent` `50` (1–100), `maxArenaMemoryMiB` `-1` (unset), `maxArenaMemoryCheck` `"halt"` (`"warn"`/`"halt"`) — decided by the owner, 2026-10-19. `maxArenaMemoryDisable` is the only off switch.
- Names are `maxArenaMemory*` (renamed from `maxMemory*`, because the cap counts only arena memory) — decided by the owner, 2026-10-19.
- Requested limit is the minimum of the set keys — decided by the owner, 2026-10-19. A safety cap should take the stricter of two keys.
- `maxArenaMemoryDisable: true` with other keys present: build warning, keys ignored — author's call, recorded here. Setting both is a contradiction a user should see, and the warning does not stop the build.
- Limits are an `IrProject` setting and are not encoded in `.mfp` — author's call, matching `max_buffer_bytes`.

## Corrections

_None yet. Filled in during execution._

## Summary

Low risk: no runtime or codegen behavior changes. The risk is the manifest validation surface; the unit tests in §Phase 1 cover every rule. Nothing outside the options section is touched.
