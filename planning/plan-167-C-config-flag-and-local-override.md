# plan-167-C: `--config=<name>` and `project-local.json`

Last updated: 2026-10-03
Effort: large (estimate: 3h–1d)
Depends on: plan-167-B

This sub-plan finishes plan-167. `mfb build` and `mfb test` accept
`--config <name>` / `--config=<name>` to select a named config. Without it,
`release` is selected, as before. A name that isn't declared is a compile error.
An optional `project-local.json` beside `project.json` overrides and extends the
`config` section for one machine, with the same validation as `project.json`.

The outcome: with `config.release.apiUrl = "https://prod"`, `config.dev.apiUrl =
"http://localhost"`, and a `project-local.json` setting `config.dev.apiKey =
"secret"`, `mfb build --config=dev` produces a program where
`os::getConfigOr("apiUrl", "")` is `"http://localhost"` and
`os::getConfigOr("apiKey", "")` is `"secret"`. `mfb build` alone gives
`"https://prod"` and `""`. `mfb build --config=staging` fails before codegen.

References:

- plan-167-A (`ProjectConfig`, `parse_config`, the lossless reader, the §5 rules)
  and plan-167-B (`SelectedConfig`, `fold_config_calls`).
- `src/cli/build/options.rs:parse_build_options`, `parse_test_options`,
  `parse_common_option`: `--target`/`--target=` (both forms), and the
  repeated-value rejection (`sign_owner.replace(value).is_some()` →
  `"mfb build accepts at most one --sign option"`).
- `src/cli/build/mod.rs:BuildOptions`, `build_project`.
- `src/cli/build/source_packages.rs:build_source_dependency`: the recursive
  dependency build, which inherits only target, opt, allow_unsigned and verbosity.
- `src/cli/help.rs:BUILD_HELP`, `TEST_HELP` (module rule: output-neutral, keep
  the 22-column padding).
- `src/cli/init.rs:init_project`, `write_new_file` (create-new, never
  overwrites).
- `mfb spec tooling cli-reference` (`07_cli-reference.md`: the `test` synopsis,
  the `## build Flags` table, the flag prose with `[[path:symbol]]`),
  `mfb spec tooling project-manifest`.
- `tests/common/mod.rs:temp_project`, `build_project`;
  `tests/common/debug_report.rs:build_debug_project`: the model for an
  end-to-end CLI test with extra build arguments.

## Prerequisites

The feature-wide gate is in plan-167-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-167-B complete | `ls planning/plan-167-B-*` → no match (archived) | NOT MET (2026-10-03) |
| No `--config` flag exists | `grep -n '"--config' src/cli/build/options.rs` → no hits | MET (2026-10-03) |
| No `project-local.json` reader exists | `grep -rn 'project-local' src` → no hits | MET (2026-10-03) |

Everything below assumes these hold.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all*
> prerequisites.

## 1. Goal

- `mfb build` and `mfb test` accept `--config <name>` and `--config=<name>`. A
  repeated `--config` is a usage error
  (`mfb build accepts at most one --config option`).
- The selected config is `<name>`, or `release` when the flag is absent. An
  undeclared `<name>` fails the build with `PROJECT_JSON_CONFIG_UNKNOWN`, which
  lists the declared names.
- `project-local.json`, when present, is parsed with the lossless reader and
  validated with the plan-167-A rules. It's merged over `project.json`'s `config`
  before selection. Its errors carry `project-local.json` paths and lines.
- `mfb init` (and `init-pkg`) write a `.gitignore` containing `build/` and
  `project-local.json` when no `.gitignore` exists (Open Decision D10).

### Non-goals (explicit constraints)

- `project-local.json` overrides **only `config`** (Open Decision D4). Any other
  top-level member is an error, not a silent merge. It cannot change `name`,
  `packages`, `options` or anything else that affects the lockfile,
  `projectHash`, or `mfb pkg` edits.
- `mfb pkg add/remove/update` and every `json_edit` rewrite never read or write
  `project-local.json`.
- `--config` is not passed to source-dependency builds. They emit `.mfp`, which
  never folds (plan-167-B non-goal), and a dependency's `project-local.json` is
  never read.
- No change to `-O`, `--target`, `--debug` or any existing flag's parsing or
  messages.

## 2. Current State

- `parse_common_option` handles `--target` and `-O` for both commands, in both
  the space and `=` forms. A value flag's repeat rejection lives in each command
  parser (`--sign`, build-only). `--debug` is parsed separately in each parser.
- `BuildOptions` is built in 6 places:
  - options.rs ×2;
  - `src/cli/pkg.rs` ×2 (`publish_package_project`, `check_abi`), hardcoded;
  - `source_packages.rs` ×1;
  - the `build_source_dependencies_for_test` helper in `src/cli/build/mod.rs`.
- After plan-167-B, `build_project` computes `SelectedConfig` for `release` from
  `project.json` on the executable and test paths.
- `mfb init` writes `project.json` and `src/main.mfb` with `create_new`, and no
  `.gitignore`. `build/` is ignored by convention only (the comment in
  `src/manifest/package.rs` near `source_dependency`).
- The acceptance harness (`scripts/test-accept.sh`) has no per-fixture build
  arguments (`run.env`, `audit.args` and `test-gate.sh` are its only hooks), so
  `--config` is tested through a Rust CLI integration test instead (§3).

### Measured populations

| What | Count | Command |
|---|---|---|
| `BuildOptions` struct literals | 6 | `grep -rn "BuildOptions {" src --include='*.rs' \| grep -v "struct BuildOptions" \| wc -l` → 6 |
| `tests/cli/*.rs` integration tests | 35 | `ls tests/cli/*.rs \| wc -l` → 35 |
| Help blocks to edit | 2 (`BUILD_HELP`, `TEST_HELP`) | `grep -n "^pub.*_HELP\|^const .*_HELP" src/cli/help.rs` |

### Verified properties

- *Both flag forms are an established pattern.* Read `parse_common_option`
  (`--target` / `--target=`). Verified.
- *A source dependency build can be told apart from the invoked project's
  build.* `build_source_dependency` sets `package_output_dir: Some(cache)`, and
  `options.rs` sets it to `None`. Verified by reading both. C uses
  `package_output_dir.is_none()` as "this is the invoked project".

## 3. Design Overview

1. **Flag.** `BuildOptions.config: Option<String>`. `parse_build_options` and
   `parse_test_options` each accept `--config <name>` / `--config=<name>`, with
   a missing-value error (`mfb {cmd} --config requires a config name`) and the
   repeat error. The flag goes in each command parser, not
   `parse_common_option`, matching the `--sign`/`--debug` precedent of a local
   `Option` plus a repeat check. The other 4 literals set `config: None`, with a
   comment explaining why.
2. **Local override.** `src/manifest/config.rs:load_project_config(project_dir,
   manifest_text) -> Result<ProjectConfig, Vec<ConfigDiagnostic>>`:
   - parses `project.json`'s `config` (plan-167-A);
   - if `project-local.json` exists, parses it with `parse_lossless`;
   - requires its root to be an object whose only member is `config` (D4);
   - validates that `config` with the A rules, except that `release` is not
     required (a local file is partial);
   - merges per §4;
   - re-runs the cross-config type check (A §5.4) on the merged result.

   `ConfigDiagnostic` gains a `file` field (`project.json` |
   `project-local.json`), so every diagnostic names the file it came from.
3. **Selection.** In `build_project`, for the invoked project only
   (`package_output_dir.is_none()`) and on every path (executable, test,
   package), select `options.config.as_deref().unwrap_or("release")`. An
   undeclared name fails with `PROJECT_JSON_CONFIG_UNKNOWN` at the `config` line,
   or at line 1 when `config` is absent. Only executable and test builds pass the
   result into codegen (B). Package builds validate it and stop there, so a typo
   is caught everywhere (requirement 5).
4. **`.gitignore` from `mfb init`.**

**Where correctness risk concentrates:** the merge (§4): which file wins, and
type conflicts across the two files. Pure functions, with a test per case.

**Where design uncertainty concentrates:** none. Every seam is an established
pattern.

**Gate class:** behavior-adding. A build without `--config` and without
`project-local.json` must stay byte-identical to the post-B compiler (checked
on one fixture).

**Rejected alternatives:**

- *General deep merge of all of `project-local.json` over `project.json`.*
  Rejected (D4): it would let a local file change `packages`, `name` or `options`
  invisibly to the lockfile, `projectHash` and every `mfb pkg` rewrite.
  Requirement 3's purpose (keys, passwords, local developer settings) is all
  config data.
- *Read `project-local.json` only on executable builds.* Rejected: a broken local
  file in a package project would then be silently ignored, against
  requirement 2.
- *Add a `build.args` hook to `scripts/test-accept.sh`.* Rejected for this plan:
  a CLI integration test covers the flag with no harness change. The harness
  stays the only owner of its own hooks.

## 4. Merge rules

For every named config `C` in `project-local.json`:

- **`C` also exists in project.json:** each local key `K` replaces project.json's
  `C.K` when `K` exists there, and is added otherwise. A replacement must keep
  `C.K`'s type. A change is `PROJECT_LOCAL_JSON_TYPE_OVERRIDE`, at the local line,
  naming the project.json line.
- **`C` does not exist in project.json:** `C` is added as a new named config
  (D5), selectable with `--config=C` on that machine.

After the merge, A's cross-config consistency rule runs over the union, so a
locally added key that conflicts with another config's type is rejected.

Duplicate detection (A §5.3) runs within each file. A local key that duplicates
a project.json key is the override mechanism, not a duplicate.

## 5. Diagnostics

| Rule | Code | When |
|---|---|---|
| `PROJECT_JSON_CONFIG_UNKNOWN` | 2-200-0025 | the selected name is not declared after the merge. Detail: "No config named `{n}`. Declared: {list}." |
| `PROJECT_LOCAL_JSON_FIELD` | 2-200-0026 | `project-local.json` root isn't an object, or has a member other than `config` |
| `PROJECT_LOCAL_JSON_TYPE_OVERRIDE` | 2-200-0027 | §4 type change |
| (reused) 2-200-0019 … 0024 | | A's rules, reported against `project-local.json` |
| (reused) the A-phase JSON parse error path | | a malformed `project-local.json`, reported with its own path and line |

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Use `- [~]` for partial work, with one line saying what's
> left. Mark moot work `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` the
> moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1: `--config` flag and selection

`--config` selects among `project.json`'s configs. This phase stands alone,
before any local file exists.

- [ ] `src/cli/build/mod.rs:BuildOptions`: add `config: Option<String>` with a
      doc comment. Set `config: None` in the 4 non-parser literals (pkg.rs ×2,
      source_packages.rs, the test helper), each with a one-line reason.
- [ ] `src/cli/build/options.rs`: `--config <name>` / `--config=<name>` in
      `parse_build_options` and `parse_test_options`, with the missing-value and
      repeat errors from §3.1. An empty name (`--config=`) is the missing-value
      error.
- [ ] `src/cli/build/mod.rs:build_project`: selection per §3.3. Add
      `PROJECT_JSON_CONFIG_UNKNOWN` (2-200-0025) to `rules/table.rs`. Pass the
      selected config where B passed `release`.
- [ ] `src/cli/help.rs`: one line in each of `BUILD_HELP` and `TEST_HELP`:
      `  --config <name>     Read os::getConfigOr values from the named config in`
      plus a continuation line `                      project.json (default: release)`.
- [ ] Unit tests in `src/cli/build/mod.rs`, mirroring the `--debug` ones:
      `parse_build_options_parses_config_both_forms`,
      `parse_build_options_rejects_repeated_config`,
      `parse_build_options_rejects_empty_config`, and the same three for
      `parse_test_options`. Dispatch: in `src/cli/dispatch/tests.rs`,
      `run(&["build","--config"]) == 2`.
- [ ] Integration test `tests/cli/cli_build_config.rs` (register a `[[test]]` in
      `Cargo.toml`), using `tests/common`. The fixture project has `release` and
      `dev` configs that differ for one key of each type. It covers:
      - no flag → the release values;
      - `--config=dev` → the dev values;
      - `--config dev` (space form) → the dev values;
      - `--config=staging` → exit non-zero, and stderr names 2-200-0025 and
        lists `dev, release`;
      - `mfb test --config=dev` on a TESTING block that asserts the dev value →
        pass.
- [ ] Spec `07_cli-reference.md`: add `[--config name]` to the `test` synopsis,
      a `--config name` / `--config=name` row in `## build Flags`, and a prose
      paragraph quoting the three error strings with
      `[[src/cli/build/options.rs:parse_build_options]]`.

Acceptance: the flag selects values end to end, and an unknown name fails the
build.
  Check: `cargo test --test cli_build_config` → pass (est. 3 min; needs a real
  build and run, which no unit test covers). Then
  `cargo test --bin mfb parse_build_options` and
  `cargo test --bin mfb parse_test_options` → both pass (est. 2 min).
Commit: —

### Phase 2: `project-local.json`

The local file overrides and extends configs, with full validation.

- [ ] `src/manifest/config.rs`: `ConfigDiagnostic.file`; `load_project_config`
      per §3.2; the merge per §4. Add rule rows 2-200-0026 and 0027.
- [ ] `src/manifest/mod.rs:validate_config`: call `load_project_config` for the
      invoked project. Render each diagnostic against its own file's path.
      `build_project` takes the merged `ProjectConfig` for selection.
- [ ] Unit tests in `config.rs`:
      - override keeps type → ok;
      - override changes type → 0027 at the local line;
      - local adds a key;
      - local adds a config, which `--config` can then select;
      - local root has `name` → 0026;
      - local is not an object → 0026;
      - local without `release` → ok;
      - a local duplicate key → 0022 in `project-local.json`;
      - a locally added key that conflicts with another config's type → 0024;
      - malformed JSON in the local file → a parse error naming
        `project-local.json`.
- [ ] Extend `tests/cli/cli_build_config.rs`:
      - a local file overriding `dev.apiUrl` and adding `dev.apiKey` → the
        merged values;
      - a local file adding config `mine`, plus `--config=mine` → its values;
      - a bad local file → the build fails with its path in stderr.
- [ ] Spec `01_project-manifest.md`: a `## Local overrides (project-local.json)`
      section stating §3.2, §4 and the D4 scope exactly, with
      `[[src/manifest/config.rs:load_project_config]]`; the new codes.
      `01_rule-codes.md`: the 2-200 rows.

Acceptance: local values override and extend configs, and a bad local file
stops the build.
  Check: `cargo test --test cli_build_config` → pass (est. 3 min). Then
  `cargo test --bin mfb manifest::config` → pass (est. 2 min).
Commit: —

### Phase 3: `mfb init` `.gitignore` and developer docs

New projects keep `project-local.json` out of source control by default, and
the developer-facing page explains the whole feature.

- [ ] `src/cli/init.rs:init_project`, and the `init-pkg` path: write `.gitignore`
      with `build/` and `project-local.json` when the file doesn't exist. Never
      overwrite or append to an existing one. Use `write_new_file`, and treat
      "already exists" as a skip, not an error.
- [ ] Init unit tests: a fresh init writes it; a pre-existing `.gitignore` is
      byte-for-byte unchanged.
- [ ] `src/codegen/builtins/os/func_get_config_or.rs` `DESC`/`EX`: explain named
      configs, `--config`, `project-local.json` (for per-machine values like
      keys, kept out of source control), and that values are stored as plain
      text in the executable. Follow `.ai/man-content.md`; no internals.
- [ ] Spec `07_cli-reference.md`: the `init` paragraph lists `.gitignore`.

Acceptance: init writes the ignore file safely, and the man page passes its
instruments.
  Check: `cargo test --bin mfb init` → pass (est. 2 min).
  `scripts/man-run-examples.sh os --run` → pass (est. 3 min).
  `scripts/man-census.sh --memory-scope` → 0 unclassified (est. 1 min).
Commit: —

## Validation Plan

- Tests: parser unit tests, the config merge unit tests,
  `tests/cli/cli_build_config.rs` end to end, and init tests. The negative cases
  (unknown name, repeated or empty flag, bad local file, type override) each have
  a test.
- Coverage check:
  `for c in 0025 0026 0027; do grep -rq "2-200-$c" src/manifest/config.rs tests/cli/cli_build_config.rs || echo missing $c; done`
  → no output.
- Runtime proof: the §1 example. Build it three ways (no flag, `--config=dev`,
  `--config=staging`) and observe the printed `apiUrl`/`apiKey`, and the failure.
  Then confirm with `strings build/<name>.out | grep secret` that the dev build
  contains the key and the release build doesn't (only referenced values are
  baked).
- Byte-identity: build `tests/byte-identity/strings` with the post-B compiler and
  with this one, and `cmp` → identical (no flag, no local file).
- Doc sync: `mfb spec tooling cli-reference`, `mfb spec tooling project-manifest`,
  `mfb spec diagnostics rule-codes`, `mfb man os getConfigOr`, and `mfb build -h`
  / `mfb test -h`. Gate: `cargo test --bin mfb spec`.
- **Final gate (run ONCE for all of plan-167, after this phase):**
  `cargo test` (the full workspace suite), plus
  `scripts/test-accept.sh target/release/mfb /tmp/acc-167-final` (the full
  acceptance corpus), plus `scripts/artifact-gate.sh target/release/mfb all`
  (est. 45–60 min combined, host macOS aarch64). This is the one full run.
  plan-167 touches the manifest validator (every fixture's build) and
  `lower_project` (every executable), so no narrower check covers the
  blast radius. The Linux/Windows targets are covered by the artifact gate's
  cross dumps; no remote box is needed, because no target-specific code changes.

## Open Decisions

- **D4, the scope of `project-local.json`.** Recommended: `config` only, with
  anything else an error. The alternative is a general deep merge (see Rejected
  alternatives).
- **D5, may the local file add new configs and keys?** Recommended: yes. Secrets
  usually exist only locally, and a developer-only config such as `mine` is
  exactly the "local developer settings" requirement 3 describes. The
  alternative, override-only, would force every secret key to be committed to
  project.json with a placeholder.
- **D10, `mfb init` writes `.gitignore`.** Recommended: yes, only when absent,
  and never touching an existing one. The alternative is documentation only,
  which leaves the first `git add .` in a new project committing a
  `project-local.json`.

## Corrections

(Filled in during execution.)

## Summary

The engineering risk is in the merge rules and in keeping `project-local.json`
from leaking beyond `config`. Both are pinned by unit tests and the scope rule.
The flag is an established pattern. Lockfile, `projectHash`, `mfb pkg` and
dependency builds are untouched. The full suite runs once, here, for all of
plan-167.
