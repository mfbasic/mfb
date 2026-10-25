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
| plan-179-A not yet landed (fresh start) | `ls planning/completed/plan-179-A-* 2>/dev/null` → no output | MET (2026-10-20: no match) |

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

- [x] `src/manifest/mod.rs`: defaults (constants next to `DEFAULT_MAX_BUFFER_MIB`), four readers, four validators, disable-with-keys warning (§4.1). Landed in the new `src/manifest/arena_memory.rs` (`arena_memory_limits`, `validate_arena_memory`, re-exported from `mod.rs`); warning is the new rule `2-200-0028 PROJECT_JSON_ARENA_MEMORY_IGNORED` (Corrections 3, 4).
- [x] `src/manifest/mod.rs` tests module (near the `canvasAutoPaintMS` tests at lines 185–220): defaults when absent; each valid value accepted; each invalid value rejected with a message naming its key (wrong type, `0`, `-2`, `101`, `0` percent, overflow MiB, bad check string); disable + keys produces the warning. In `src/manifest/tests/arena_memory.rs` (4 tests; MiB `2^44-1` accepted, `2^44` rejected, fractions read as the default, valid cases assert no diagnostic). `cargo test --bin mfb manifest::` → `test result: ok. 226 passed; 0 failed`.
- [x] `src/ir/types.rs`: `ArenaMemoryLimits`, `ArenaMemoryCheck`, field `arena_memory` on `IrProject` (§4.2). `cargo build` → compiles.
- [x] Construction sites set the default: `src/ir/lower.rs:335`, `src/target/package_mfp/mod.rs:501`, `src/optimizer/opt1/local_rewrites.rs:419`. 11 files, not 3 (Correction 1); the executable and dump paths in `src/cli/build/mod.rs` set the manifest value.
- [x] `src/target/shared/nir/mod.rs:60` and `src/target/shared/lower.rs:41`: carry the struct to NIR. `NirModule::arena_memory`, set in `src/target/shared/nir/lower.rs` beside `canvas_auto_paint_ms` (Correction 2).
- [x] `src/docs/spec/tooling/01_project-manifest.md`: options table rows and "Arena memory cap" subsection (§4.3). Also footnote 4's key count (three → seven), the validator list, the `2-200-0028` row here and in `diagnostics/01_rule-codes.md` (populations 29 → 30, warn rules nine → ten). `cargo test --bin mfb spec` → `67 passed; 0 failed`; `mfb spec tooling project-manifest | grep -n '\[\['` → no output.

Acceptance: every manifest test in the new set passes, and a build of a project with the four keys absent produces the same artifacts as before the change.
  Check: `cargo test --bin mfb manifest::` → all manifest tests pass (est. 5 min; a scoped run is the cheapest check that fails if a validator is wrong).
  Check: a `project.json` with `"maxArenaMemoryPercent": 101` → build fails with a message naming `maxArenaMemoryPercent`.
  Result: `cargo test --bin mfb manifest::` → 226 passed, 0 failed. `mfb build /tmp/p179a` with `"maxArenaMemoryPercent": 101` → `error[2-200-0006 PROJECT_JSON_FIELD_TYPE] … Field \`maxArenaMemoryPercent\` must be a whole number from 1 to 100`, rc=1. `scripts/artifact-gate.sh target/debug/mfb strings` → `1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)` (the full gate runs once at the end of the plan, §5 of follow-plan).
Commit: 0ed6f7b2b

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

## Notes from the abandoned first attempt (2026-10-20)

A first attempt implemented A, B and C on branch `worktree-P-179`; the owner discarded it (branch and worktree deleted, nothing merged). These are the findings worth carrying into the redo. They are review notes, not changes to the design above.

- **The reader must agree with the validator on fractions.** The attempt's reader cast `12.5` to percent `12` and `2.5` to MiB `2` (`as u64` / `as i64` truncation) while the validator rejected both, and its doc comment claimed the reader never returns an out-of-range value. Filter `fract() == 0.0` in the reader too, or do not make that claim.
- **Test the MiB upper boundary exactly.** `MAX_ARENA_MEMORY_MIB = u64::MAX / 2^20` (2^44 − 1) is exact in f64. Test `MAX` accepted and `MAX + 1` rejected; testing only `1e30` would not catch a regression to `<= u64::MAX as f64`. Valid cases should also assert that no diagnostic was collected, and the "names the offending key" test should cover `maxArenaMemoryDisable`.
- **Spec provenance.** The new manifest section needs `[[src/…:Symbol]]` citations for the defaults and the MiB ceiling (`.ai/specifications.md`). The existing footnote that counts "keys read" must be updated when four keys are added.
- **The IR field must be read in the same change.** An unread `pub(crate)` field warns as dead code in this binary-only crate; the attempt deferred the read to B and recorded it as a Correction. Either land A and B's read together, or accept that A carries an unused field until B lands.
- **The crate is edition 2021:** `if let … &&` let-chains do not compile.
- **Open question for the owner (§3, "smaller of the set keys"):** the percent always has a value (default 50), so `maxArenaMemoryMiB: 32768` alone on a 16 GiB machine still caps at 8 GiB. Confirm that this is intended, and state it in the spec either way.

## Corrections

1. **Construction sites: 11 files, not 3.** `grep -rln "canvas_auto_paint_ms: crate::manifest::DEFAULT_CANVAS_AUTO_PAINT_MS," src` → 11 files (`IrProject` and `NirModule` literals, test fixtures included: `optimizer/opt1/local_rewrites.rs`, `ir/tests/variant_corpus/mod.rs`, `ir/test_support.rs`, `ir/binary/mod.rs`, `ir/lower/mod.rs`, `target/package_mfp/mod.rs`, `target/shared/validate/mod.rs`, `target/shared/plan/mod.rs`, `codegen/memory/arena/graph_copy.rs`, `codegen/engine/validation/validation.rs`, `binary_repr/tests/fixtures.rs`). Each sets `ArenaMemoryLimits::default()`.
2. **The value does not travel the `stdinLogCap` route.** `stdinLogCap` is passed to `target::write_executable` as an argument; the right precedent is `canvasAutoPaintMS`, an `IrProject` field set in `src/cli/build/mod.rs` (executable path and `--nir`/`--ncode` dump path, so a dump shows the program's real cap) and copied to `NirModule` in `src/target/shared/nir/lower.rs`. That is what was done. The `src/target/shared/lower.rs:41` citation does not exist on main.
3. **One module, shared predicates.** The readers and validators live in `src/manifest/arena_memory.rs` rather than in `mod.rs` (552 lines). One predicate per key is shared by the reader and the validator, so a fraction such as `12.5` reads as the default instead of truncating (the first attempt's bug). One `validate_arena_memory` covers all four keys.
4. **The warning needed a rule.** There is no separate "warning path": a warning is `rules::show_diagnostic` with a `Severity::Warn` rule. Added `2-200-0028 PROJECT_JSON_ARENA_MEMORY_IGNORED` (warn) in `src/rules/table/tooling.rs`, with spec rows. An invalid value is still an error when the cap is disabled (author's call: an invalid value is a typo the user should fix either way).
5. **`DEFAULT_ARENA_MEMORY_CHECK` is typed `ArenaMemoryCheck`, not `&str`.** A `&str` constant would be unread (dead code); the typed constant is what `ArenaMemoryLimits::default` uses.
6. **`NirModule::arena_memory` is unread in this sub-plan's commit.** `cargo build` → `warning: field \`arena_memory\` is never read` until plan-179-B reads it in the plan and code stages; B's commit lands directly after A on the same branch (the option the first attempt's notes named).
7. **Open question (§3, "smaller of the set keys") answered by the recorded decision.** The Decisions section records the owner's choice (minimum of the set keys); the spec now states the consequence explicitly with the 16 GiB example.

## Summary

Low risk: no runtime or codegen behavior changes. The risk is the manifest validation surface; the unit tests in §Phase 1 cover every rule. Nothing outside the options section is touched.
