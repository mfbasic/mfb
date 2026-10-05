# plan-167-A: Rename manifest `config` → `options`, and the named-config schema

Last updated: 2026-10-03
Overall Effort: x-large (estimate: 1d–2d across A, B, C)
Effort: large (estimate: 3h–1d)
Depends on: nothing

plan-167 gives an MFBASIC program build-time configuration values. A project
declares named configs in `project.json` (`"config": {"release": {...}, "dev": {...}}`),
each a flat map of keys to String, Integer or Float values. An optional, untracked
`project-local.json` overrides or adds values for one machine (keys, passwords,
local settings). `mfb build --config=<name>` selects one config, defaulting to
`release`, and the program reads values with
`os::getConfigOr(key, default)`, overloaded on the default's type. Any ambiguity or
error in the config data fails the build.

The source of this feature was `planning/project.md` (deleted when this plan was
written). Its five requirements were:

1. A `config` object in project.json, holding at least the named config `release`.
   Each config holds String, Integer and Float key/value pairs.
2. Any ambiguity or issue in `config` prevents the build and shows an error.
3. An optional `project-local.json` overrides project.json values. The compiler
   loads project.json first, then merges project-local.json over it. The file is
   meant for keys, passwords and local developer settings, and should not go into
   source control.
4. `os::getConfigOr(key AS String, default AS String|Integer|Float)` returns the
   value from the config the `--config=<name>` switch selects. It errors when the
   value has the wrong type. There is no `getConfig()`: every lookup has a default.
5. `--config=<name>` is optional; when it is absent `release` is used. A `<name>`
   that is not a key of `config` is a compile error.

**This sub-plan** frees the `config` key (the existing tunables object moves to
`options`), adds a lossless JSON reader that keeps number lexemes, duplicate keys
and key lines, and validates the `config` section of `project.json`. The outcome:
`mfb build` rejects every malformed `config` section listed in §5 with a
`2-200-*` diagnostic at the offending line. A well-formed one builds exactly as
today. Nothing reads config values yet.

References:

- `src/manifest/mod.rs:validate_project_manifest`: the validating loader every
  build runs. Its validators set a shared `valid` flag so a run reports every error.
- `src/manifest/mod.rs:stdin_log_cap`: today's only reader of `"config"`.
- `src/manifest/mod.rs:field_position`: maps a field name to a line by text
  search (first line containing `"field"`). It is not nesting-aware.
- `src/cli/init.rs:project_manifest`, `package_project_manifest`: the templates
  that write `"config": {"stdinLogCap": …}`.
- `src/rules/table.rs:RULES` and `src/rules/mod.rs:show_diagnostic`: rule rows and
  the diagnostic renderer.
- `src/json.rs`: thin wrappers over the `tinyjson` crate (`check_json_depth`,
  depth cap 256).
- `mfb spec tooling project-manifest` (`src/docs/spec/tooling/01_project-manifest.md`):
  the field table (`config` row), footnote ⁴, and the Diagnostic Codes section.
- `mfb spec diagnostics rule-codes` (`src/docs/spec/diagnostics/01_rule-codes.md`):
  the `2-200` subsystem table.
- `.ai/specifications.md`, `.ai/spec-content.md`, `.ai/testing-gates.md`.

## Prerequisites

These are a precondition on the whole plan-167 feature. Sub-plans B and C point
here.

| Must be true | Command | Status |
|---|---|---|
| No other plan is changing `project.json` schema or `validate_project_manifest` | `grep -ln "validate_project_manifest\|project.json" planning/plan-*.md \| grep -v plan-167` → no plan with open tasks on the manifest validator | MET (2026-10-05; re-run: hits are plan-171-E and plan-92, which mention `project.json` only as fixture files, neither touches the validator) |
| `config` has no reader other than `stdin_log_cap` | `grep -rn '"config"' src --include='*.rs'` → `src/manifest/mod.rs` (`stdin_log_cap`), `src/cli/init.rs` templates, and tests only | MET (2026-10-05; the only reader is `stdin_log_cap`, other hits are comments and the unrelated `json::get` doc examples `["config", "enabled"]`) |

Everything below assumes these hold.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all*
> prerequisites.

## 1. Goal

- The tunables object is named `options` everywhere: manifests, the `mfb init`
  and `mfb init-pkg` templates, the reader, and the spec. `options.stdinLogCap`
  bakes the same cap `config.stdinLogCap` did.
- A manifest that still has `"config": {"stdinLogCap": N}` fails the build with
  `PROJECT_JSON_CONFIG_MOVED_OPTION`, which names `options.stdinLogCap`.
- `mfb build` rejects each malformed `config` case in §5 with a `2-200-*`
  diagnostic whose line is the offending key's line. A valid `config`, or no
  `config`, builds with output byte-identical to the pre-plan compiler.

### Non-goals (explicit constraints)

- `tinyjson` stays the parser for everything except the config sections. The new
  lossless reader is used only to read `config` (here) and project-local.json
  (C). No other manifest field changes parser.
- No change to any other manifest field, to `mfb.lock`, or to `projectHash`.
- No codegen change. A build's output is byte-identical to the pre-plan
  compiler's for every project, because nothing consumes config values until C.
- No backward-compatibility alias for `config.stdinLogCap`. The old shape fails
  with a targeted error (decided by the user, 2026-10-03).

## 2. Current State

- **The `config` key already exists.** `src/manifest/mod.rs:stdin_log_cap` reads
  `manifest["config"]["stdinLogCap"]` as `f64` and keeps it only if it is finite
  and ≥ 8192. `src/cli/build/mod.rs:build_project` passes the result to
  `target::write_executable`, and `NirModule.stdin_log_cap`
  (`src/target/shared/nir/mod.rs`) carries it to codegen. The spec says unknown
  keys under `config` are ignored (`01_project-manifest.md`, footnote ⁴) and that
  the field is read lazily, not validated.
- **The parser can't tell Integer from Float.** `tinyjson::JsonValue` is
  `Number(f64)`, so `1` and `1.0` parse to the same value and integers above
  2^53 lose precision. Duplicate object keys are silently overwritten, last one
  wins (`tinyjson-2.5.1/src/parser.rs`, `m.insert`). Objects are a `HashMap`, so
  key order and positions are lost.
- **Diagnostic positions come from a text search.** `field_position` finds the
  first line containing `"name"`. A config key called `name` or `version` would
  land on the top-level field's line. That's why the config validator needs its
  own positions.
- **`validate_project_manifest` has six production callers, and one of them
  is recursive.** `src/cli/build/mod.rs:build_project`, `src/cli/fmt.rs`,
  `src/cli/doc.rs` and `src/cli/pkg.rs` ×3
  (`grep -rn "validate_project_manifest(" src/cli --include='*.rs' | wc -l` → 6).
  `build_project` also runs for every source dependency:
  `src/cli/build/source_packages.rs:build_source_dependency` calls it with
  `package_output_dir: Some(cache)`. So a new check in the validator applies to
  `mfb fmt`, `mfb doc` and `mfb pkg`, and to every source dependency's
  `project.json` during the consuming app's build, not only to the project the
  user invoked.
- **Unknown top-level manifest keys are ignored.** No validator iterates the
  keys. `targets` is documented as "reserved … not read by any stage".
- **`config` has no position in the validator yet**, so no existing diagnostic
  covers it.

### Measured populations

| What | Count | Command |
|---|---|---|
| Manifests in the repo carrying `stdinLogCap` | 19 | `grep -rl --include=project.json '"stdinLogCap"' . \| grep -v '^./target' \| wc -l` → 19 |
| Manifests in the repo with a `"config"` key (all of them are the `stdinLogCap` ones) | 19 | `grep -rl --include=project.json '"config"' . \| grep -v '^./target' \| wc -l` → 19 |
| Files under `src tests scripts` mentioning `stdinLogCap` | 6 | `rg -l 'stdinLogCap' src tests scripts \| wc -l` → 6 (`manifest/mod.rs`, `cli/init.rs`, `target/shared/nir/mod.rs`, `docs/spec/threading/09_os-integration.md`, `docs/spec/tooling/01_project-manifest.md`, `codegen/io/stdin/stdin_broadcast.rs`) |
| Highest used `2-200` rule code | `2-200-0017` (plus the `0100`/`0101` block) | `grep -o 'code: "2-200-[0-9]*"' src/rules/table.rs \| sort \| tail -4` |
| Rule rows today | 253 | `rg -c 'name: "' src/rules/table.rs` → 253 (2026-10-03) |

### Verified properties

- *The tinyjson value can't support requirement 2.* Read
  `tinyjson-2.5.1/src/json_value.rs` (`Number(f64)`) and `parser.rs`
  (`insert` overwrites). So "distinguish Integer from Float" and "a duplicate key
  is an error" both need the raw text. Verified by reading the code.
- *Nothing in `src/` reads `config` other than `stdin_log_cap`.* Verified with the
  `grep -rn '"config"' src` prerequisite above.

## 3. Design Overview

Three pieces, in landing order:

1. **The `options` rename.** Mechanical, and safe to land alone. The reader, the
   templates, the 19 manifests and the spec all change in one commit.
2. **`src/json/lossless.rs`, a lossless JSON reader.** It returns an ordered tree
   whose numbers keep their lexeme, whose object members keep their key and line,
   and which doesn't drop duplicate keys. The primitive has no callers until
   piece 3. It also enforces the same depth cap as `check_json_depth`.
3. **`src/manifest/config.rs`, the config model and validator.** It runs the
   lossless reader over the whole project.json text, takes the top-level `config`
   member, and builds `ProjectConfig { configs: Vec<NamedConfig> }`, where
   `NamedConfig { name, line, values: Vec<ConfigEntry { key, line, value: ConfigValue }> }`
   and `ConfigValue = String(String) | Integer(i64) | Float(f64)`. It also emits
   the §5 diagnostics. `validate_project_manifest` calls it as one more named
   validator.

**Where correctness risk concentrates:** the Integer/Float lexeme rule and i64
range checking (§5.2), and line attribution for nested keys (§4). Both are pure
functions with unit tests. No codegen is touched, so the blast radius is limited
to build acceptance and rejection.

**Where design uncertainty concentrates:** none of consequence. The lossless
reader is ordinary JSON (RFC 8259); the tests compare it against `tinyjson` on the
repo's own manifests.

**Gate class:** this sub-plan is behavior-neutral for valid manifests, so
**byte-identity is the right gate** for the rename and the validator on a valid
project. Expected diffs are limited to the 19 manifest files, `init.rs`, the
spec, and new diagnostics on invalid fixtures. A codegen diff on any fixture is a
bug to root-cause: compare ONE fixture's `.ncode`.

**Rejected alternatives:**

- *Keep `config` for tunables and pick a new key (e.g. `configs`) for named
  configs.* Rejected: the user chose `options` for the tunables (2026-10-03), and
  `config` is the name requirement 1 uses.
- *Hybrid `config` holding both `stdinLogCap` and named-config objects.* Rejected:
  "a member that isn't an object" would be both a tunable and an error, which is
  exactly the ambiguity requirement 2 forbids.
- *Swap `tinyjson` for `serde_json` project-wide.* Rejected: far too much blast
  radius for the problem, and `serde_json` is a dev-dependency only (`Cargo.toml`).
  The lossless reader is scoped to the config sections.
- *Scan the raw text for number lexemes beside the tinyjson parse.* Rejected:
  matching a lexeme back to a tinyjson value is fragile (escapes, duplicate keys,
  same key in two configs). One parse that keeps everything is simpler and
  testable.

## 4. The lossless reader (`src/json/lossless.rs`)

`src/json.rs` becomes `src/json/mod.rs`, with no content change, plus a
`lossless` submodule. This follows the repo's `foo/mod.rs` layout.

```rust
pub(crate) enum LosslessValue {
    Null,
    Bool(bool),
    Number(String),            // the exact lexeme, e.g. "-12", "3.0", "1e5"
    String(String),            // escapes decoded
    Array(Vec<LosslessValue>),
    Object(Vec<LosslessMember>), // source order, duplicates kept
}
pub(crate) struct LosslessMember { pub key: String, pub line: u32, pub value: LosslessValue }
pub(crate) fn parse_lossless(text: &str) -> Result<LosslessValue, LosslessError>;
pub(crate) struct LosslessError { pub line: u32, pub message: String }
```

- The grammar is strict RFC 8259 (the number grammar is
  `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`). It rejects trailing commas
  and comments, as tinyjson does.
- `line` is the 1-based line of the member's key token.
- Depth is capped at the `check_json_depth` limit.
- String escapes are decoded, including surrogate pairs. A lone surrogate is an
  error.
- Strings in the input are already proven valid UTF-8 by the `&str`.

## 5. The `config` schema and its diagnostics

### 5.1 Shape

```json
"config": {
  "release": { "apiUrl": "https://api.example.com", "retries": 3, "timeout": 2.5 },
  "dev":     { "apiUrl": "http://localhost:8080" }
}
```

- `config` is optional. **Absent `config` = one implicit empty `release`** (Open
  Decision D6). Existing projects keep building, and `--config` with no name
  still works.
- When `config` is present, it must be an object, and it must contain `release`.
- Every member of `config` is a named config whose value must be an object.
- Every member of a named config is a key whose value is a JSON string, an
  Integer lexeme or a Float lexeme.

### 5.2 Value typing (Open Decision D2)

- **Integer:** a number lexeme with no `.`, `e` or `E` that parses as `i64`.
  `9223372036854775808` is an error, not a Float.
- **Float:** a number lexeme with a `.` or an exponent, parsed with Rust's
  `str::parse::<f64>` (correctly rounded). A non-finite result such as `1e999` is
  an error.
- **String:** any JSON string. A decoded value containing U+0000 is an error,
  because MFBASIC strings are NUL-terminated at the C boundary (see the
  `os::getEnvOr` `name` contract).
- `true`, `false`, `null`, arrays and objects are errors.

### 5.3 Names

- Config names and keys must match `^[A-Za-z][A-Za-z0-9_.-]*$` (Open Decision D9).
  A dotted key like `db.host` is just a key, not a nested path.
- Two members that compare equal under ASCII case-folding are a duplicate.
- Lookup is exact. `os::getConfigOr` (plan-167-B) matches its `key` argument
  byte for byte. The case-insensitive duplicate rule exists so that only one
  spelling of a key can be declared. A call that spells the key differently
  (`"Port"` against a declared `port`) finds no key and yields its default,
  like any other undeclared key.

### 5.4 Cross-config consistency (Open Decision D3)

A key that appears in more than one named config must have the same type in
every config. `release.port = 80` with `dev.port = "80"` is an error. Otherwise
a program could build and run under one config and fail to build under another,
which is the ambiguity requirement 2 forbids.

### 5.5 Diagnostics

New rows in `src/rules/table.rs`, all severity error, all with rendered position
= the member's `line`:

| Rule | Code | When | Detail |
|---|---|---|---|
| `PROJECT_JSON_CONFIG_MOVED_OPTION` | 2-200-0018 | a `config` member named `stdinLogCap` | "`config.stdinLogCap` moved to `options.stdinLogCap`." |
| `PROJECT_JSON_CONFIG_SHAPE` | 2-200-0019 | `config` is not an object, or a named config's value is not an object | "Field `config` must be an object of named configs." / "Config `{name}` must be an object of key/value pairs." |
| `PROJECT_JSON_CONFIG_RELEASE_MISSING` | 2-200-0020 | `config` present without `release` | "Field `config` must contain a `release` config." |
| `PROJECT_JSON_CONFIG_NAME` | 2-200-0021 | a config name or key fails §5.3's pattern | "Config name `{n}` must match `[A-Za-z][A-Za-z0-9_.-]*`." (and the same for keys) |
| `PROJECT_JSON_CONFIG_DUPLICATE` | 2-200-0022 | duplicate config name or key, case-insensitively | "Key `{k}` in config `{c}` is already declared on line {l}." |
| `PROJECT_JSON_CONFIG_VALUE` | 2-200-0023 | the value is not a String/Integer/Float per §5.2 | "Config `{c}` key `{k}` must be a String, Integer or Float; found {what}." |
| `PROJECT_JSON_CONFIG_TYPE_CONFLICT` | 2-200-0024 | §5.4 conflict | "Key `{k}` is {T1} in config `{a}` but {T2} in config `{b}`." |

`PROJECT_JSON_CONFIG_MOVED_OPTION` fires instead of `PROJECT_JSON_CONFIG_SHAPE`
for the `stdinLogCap` member, so an old manifest gets the actionable message.
It also replaces `PROJECT_JSON_CONFIG_RELEASE_MISSING`: when `config` has a
`stdinLogCap` member, the missing-`release` check is skipped. An old manifest
(`"config": {"stdinLogCap": N}`) therefore reports exactly one diagnostic, 0018.
All config errors are reported in one run, matching the `valid`-flag style of
`validate_project_manifest`.

## Compatibility / Format Impact

- **Breaking:** `"config": {"stdinLogCap": N}` is now a build error, and the fix
  is the rename to `"options"`. External projects created by `mfb init` before
  this plan carry this key, so the release notes and the error message must both
  say how to fix it.
- **Breaking, for apps that use old source packages:** a source dependency's
  `project.json` goes through `validate_project_manifest` during the consuming
  app's build (§2). A third-party package written by an older `mfb init-pkg`
  still carries `config.stdinLogCap`, so the app's build fails with 2-200-0018
  until the package is updated. The diagnostic names the dependency's own
  `project.json` path, so the fix is clear. This is the user's no-alias
  decision (2026-10-03) applied to every manifest the build reads. A package's
  `config` section is validated whenever the package is built, including inside
  a consumer's build. Its values never apply to the consumer (plan-167-B).
- **Other commands:** `mfb fmt`, `mfb doc` and `mfb pkg` run the same validator,
  so an old-shape or malformed `config` also stops them. This is intended:
  requirement 2 says any issue in `config` is an error, and every command that
  reads the manifest already fails on its other invalid fields.
- **Unchanged:** every other manifest field, `mfb.lock`, `.mfp` format, codegen,
  and the stdin cap default.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Use `- [~]` for partial work, with one line saying what's
> left. Mark moot work `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` the
> moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1: `config` → `options` rename

This phase is mechanical and lands alone. Valid builds are byte-identical, and an
old manifest fails with a clear message.

- [x] Save a pre-plan release compiler for the byte-identity check:
      `cargo build --release && cp target/release/mfb /tmp/mfb-167-base`.
      → built at f23af579c, `/tmp/mfb-167-base` 41518096 bytes.
- [x] `src/manifest/mod.rs:stdin_log_cap`: read `manifest["options"]["stdinLogCap"]`.
      Rename the doc comments that say `"config"`.
- [x] `src/cli/init.rs:project_manifest`, `package_project_manifest`: write
      `"options"`. Update the init unit tests that assert the template text.
      (The tests assert via `stdin_log_cap`, which round-trips unchanged; only the comment moved.)
- [x] Rewrite `"config"` → `"options"` in the 19 manifests measured above.
      → `grep -rl --include=project.json '"config"' . | grep -v ^./target | wc -l` → 0; same for `"options"` → 19.
- [x] Add `PROJECT_JSON_CONFIG_MOVED_OPTION` (2-200-0018) to `src/rules/table.rs`.
      Emit it from a new `validate_config` in `src/manifest/mod.rs`. In this phase
      it's a tinyjson-level check for a `stdinLogCap` member under `config`;
      Phase 3 replaces it.
- [x] Comments citing `config.stdinLogCap`:
      `src/codegen/io/stdin/stdin_broadcast.rs` and
      `src/target/shared/nir/mod.rs:NirModule.stdin_log_cap`. Also `src/target.rs`,
      `src/target/shared/lower.rs:lower_project`, `src/cli/build/mod.rs` and
      `error_constants.rs:STDIN_LOG_CAP_DEFAULT`, which `grep -rn '"config"' src` found.
- [x] Spec: `tooling/01_project-manifest.md` (rename the `config` row to
      `options`, rewrite footnote ⁴, add the moved-option code) and
      `threading/09_os-integration.md` (`options.stdinLogCap`).
      `diagnostics/01_rule-codes.md`: add 2-200-0018, and extend the line-97
      range sentence.
- [x] Tests: one fixture `tests/syntax/project/project-config-moved-option-error/`
      with an old-shape manifest, plus a golden `build.log` showing the
      2-200-0018 diagnostic.

Acceptance: an old-shape manifest fails with 2-200-0018, and a renamed one bakes
the same cap.
  Check: `scripts/test-accept.sh target/release/mfb /tmp/acc-167 project-config-moved-option-error`
  → pass (est. 1 min). Then build `spikes/s1` (an executable, one of the 19
  renamed) with both `/tmp/mfb-167-base` (from the parent commit, with `config`)
  and the new compiler (`options`), and `cmp` the two executables → identical
  (est. 2 min). Last, `rg -l 'stdinLogCap' src tests scripts | xargs rg -n '"config"'`
  → no hits (est. 5 s).
  Result (2026-10-05): test-accept → `acceptance tests passed (1 test(s) ran)`. The `cmp`
  ran on a stdin-reading program instead of `spikes/s1` (see Corrections): base
  (`config`, cap 65536) vs new (`options`, cap 65536) → identical; new at the default cap
  → `differ: char 10073`, proving the cap is read. Grep → only the 0018 detector
  (`src/manifest/mod.rs:validate_config`) and its fixture (see Corrections).
  `cargo test --bin mfb -- cli::init manifest:: rules:: spec` → 277 passed.
Commit: e96a55c8f

### Phase 2: Lossless JSON reader

This phase adds the primitive with no callers. It's safe because nothing calls it.

- [x] Move `src/json.rs` → `src/json/mod.rs` (`git mv`, no content change).
      (Plus the `pub(crate) mod lossless;` line, and the one spec citation
      `[[src/json.rs:json_string]]` in `architecture/20_ir-json-artifact.md`, which
      `spec_citations_resolve` caught.)
- [x] Add `src/json/lossless.rs` per §4: `LosslessValue`, `LosslessMember`,
      `parse_lossless`, `LosslessError`, and the depth cap shared with
      `check_json_depth`.
- [x] Tests in `src/json/lossless.rs` (`#[cfg(test)]`):
  - number lexemes kept exactly (`3`, `3.0`, `-0`, `1e5`, `2E-3`);
  - duplicate keys kept in order;
  - member lines on a multi-line object;
  - escapes and surrogate pairs decoded, and a lone surrogate rejected;
  - trailing comma, comment, leading zero (`01`), `+1` and `.5` all rejected;
  - depth cap;
  - every `project.json` under `tests/` parses, with the same structure as
    tinyjson. The comparison is a test-only conversion `LosslessValue` →
    `tinyjson::JsonValue`, run over `find tests -name project.json` (1590 files, 2026-10-03; 1636 on 2026-10-05,
    `find tests -name project.json | wc -l`).

Acceptance: the reader agrees with tinyjson on every manifest in the tree, and
keeps what tinyjson drops.
  Check: `cargo test --bin mfb json::lossless` → all pass (est. 2 min, mostly the
  build).
  Result (2026-10-05): `test result: ok. 9 passed; 0 failed`.
Commit: 26a8f38c5 (with Phase 3; see Corrections)

### Phase 3: `config` model and validator

This phase ends with every §5 rule enforced with a correct line, and a valid
project builds byte-identically.

- [x] `src/manifest/config.rs`: `ProjectConfig`, `NamedConfig`, `ConfigEntry`,
      `ConfigValue`, `ConfigType`. Add
      `parse_config(text: &str) -> Result<ProjectConfig, Vec<ConfigDiagnostic>>`,
      which reads the top-level `config` member of the lossless tree and applies
      §5.1–§5.4. `ConfigDiagnostic` holds the rule name, detail and line.
      Implicit `release` when `config` is absent (D6).
- [x] Add rule rows 2-200-0019 … 2-200-0024 (§5.5) to `src/rules/table.rs`.
- [x] `src/manifest/mod.rs:validate_config`: replace the Phase 1 check with
      `parse_config` over the manifest text, which `validate_project_manifest`
      has already read. Render each `ConfigDiagnostic` with `show_diagnostic` at
      its line, and clear `valid` on any error.
- [x] Do not expose `ProjectConfig` to the build in this phase. AGENTS.md forbids
      dead code kept for "a later phase". The validator is `parse_config`'s
      production caller, and B adds the build-side accessor along with its caller.
- [x] Unit tests in `src/manifest/config.rs`: one per §5.5 rule, each asserting
      the rule name and line; implicit `release`; valid multi-config;
      Integer/Float edge cases (`9223372036854775807` ok, `…808` error, `1e999`
      error, `-0` is Integer 0, `0.0` is Float).
- [x] Acceptance fixtures under `tests/syntax/project/` (the existing home of the
      `project-*` manifest fixtures), each with a golden `build.log`:
      `project-config-valid` (builds, exit 0), plus one `project-config-<rule>-error`
      fixture per §5.5 rule (0019–0024).
- [x] Fixture `tests/syntax/project/project-config-dependency-moved-option-error/`:
      an executable whose source dependency (declared by directory, modelled on
      `source-package-dependency-rt`) still has `"config": {"stdinLogCap": N}`.
      The golden `build.log` shows a single 2-200-0018 diagnostic naming the
      dependency's `project.json`. This pins the §Compatibility consequence.
- [x] Spec `tooling/01_project-manifest.md`: state that the `config` checks run
      wherever the manifest is validated (`mfb build`, `mfb test`, `mfb fmt`,
      `mfb doc`, `mfb pkg`, and each source dependency's build).
- [x] Spec `tooling/01_project-manifest.md`: a new `## Named configs (config)`
      section stating §5.1–§5.4 exactly as implemented, with
      `[[src/manifest/config.rs:parse_config]]`; the new codes in Diagnostic
      Codes. Also the `2-200` table in `diagnostics/01_rule-codes.md`.

Acceptance: each malformed config is rejected at the right line, and a valid
config changes nothing in the output.
  Check: `scripts/test-accept.sh target/release/mfb /tmp/acc-167 'project-config-*'` → all pass (est. 2 min). Then
  build `tests/syntax/project/project-config-valid` with `/tmp/mfb-167-base` (the base
  compiler ignores a named-config `config` because `stdin_log_cap` reads only
  `stdinLogCap`) and with the new compiler; `cmp` the executables → identical
  (est. 1 min).
  Result (2026-10-05): test-accept `'project-config-*'` → `acceptance tests passed (9 test(s) ran)`;
  `cmp` of `project_config_valid.out` from `/tmp/mfb-167-base` and the new compiler →
  identical. Coverage loop (Validation Plan) → no output.
  `cargo test --bin mfb -- manifest:: json:: rules:: spec` → 309 passed; `cargo check
  --all-targets` and `cargo clippy --bin mfb` → no warnings.
Commit: 26a8f38c5

## Validation Plan

- Tests: the lossless reader's unit tests; per-rule unit tests in
  `manifest/config.rs`; one `tests/syntax/project/project-config-*-error` fixture per
  diagnostic; one valid fixture.
- Coverage check: every new rule code has a fixture whose golden shows it.
  Check with `for c in 0018 0019 0020 0021 0022 0023 0024; do grep -rl "2-200-$c" tests/syntax/project/project-config-*/golden/build.log >/dev/null || echo missing $c; done`
  → no output.
- Runtime proof: Phase 1's `cmp` of a renamed `stdinLogCap` project against the
  base compiler.
- Doc sync: `mfb spec tooling project-manifest`,
  `mfb spec diagnostics rule-codes` and `mfb spec threading os-integration`
  render the new text. Gate: `cargo test --bin mfb spec`.
- Final gate: run the plan-167 full suite once, at the end of sub-plan C (see
  plan-167-C's Validation Plan). Not per sub-plan.

## Open Decisions

- **D2, Integer vs Float.** Recommended: the lexeme rule (§5.2): `3` is Integer
  and `3.0` is Float, and a Float key written `3` is a type error. The
  alternative is to coerce an Integer lexeme to Float where the key's other uses
  are Float. Rejected: that coercion is exactly the ambiguity requirement 2
  forbids.
- **D3, cross-config type consistency.** Recommended: an error (§5.4). The
  alternative is a per-config type checked only for the selected config, which
  lets a build pass under `release` and fail under `dev`.
- **D6, absent `config`.** Recommended: an implicit empty `release`, so every
  existing project still builds. The alternative is to require `config` in
  every manifest, which breaks every project without one.
- **D9, name pattern.** Recommended: `^[A-Za-z][A-Za-z0-9_.-]*$` with
  case-insensitive duplicate detection. The alternative is any non-empty string,
  which allows `"Port"` beside `"port"`, a lookalike ambiguity.

## Corrections

- **Phase 1 `cmp` fixture.** `spikes/s1` never reads stdin, so its executable is
  identical at any `stdinLogCap` (measured: default-cap and 65536-cap builds `cmp`
  equal), which makes the planned check vacuous. Replaced with a 7-line
  `io::readLine` program at cap 65536, built by the base compiler under `config` and
  by the new one under `options` → identical; the default-cap build differs, so the
  check does see the cap.
- **Phase 1 grep check.** `rg -l 'stdinLogCap' src tests scripts | xargs rg -n '"config"'`
  cannot return no hits: this plan itself requires the 0018 detector to look for
  `config.stdinLogCap` and a fixture that carries the old shape. The corrected
  expectation is that the only hits are `src/manifest/mod.rs:validate_config` (and its
  doc comment) and `tests/syntax/project/project-config-moved-option-error/`.
- **Phases 2 and 3 land as one commit.** Phase 2 alone adds `parse_lossless`
  with no production caller, which `cargo check --all-targets` reports as dead
  code, and AGENTS.md forbids suppressing it. Phase 3 supplies the caller
  (`manifest::config::parse_config`), so the two share one commit.
- **Diagnostics added beyond §5.5's table.** A repeated top-level `config` member
  is reported as `PROJECT_JSON_CONFIG_DUPLICATE` (tinyjson keeps the last one
  silently, which is the ambiguity requirement 2 forbids). A config section that
  the lossless reader rejects but tinyjson accepted is reported as
  `PROJECT_JSON_PARSE_FAILED` at the reader's line (none of the 1636 test
  manifests hits this: `agrees_with_tinyjson_on_every_project_json_under_tests`).
  The duplicate detail for a config *name* reads "Config `{n}` is already declared
  on line {l}.", since §5.5's wording names a key.
- **`ConfigValue::Float` keeps its lexeme** (`Float(f64, String)`), because
  plan-167-B folds the Float to a constant carrying "the config lexeme".
- **0018 detail text.** §5.5's detail ("`config.stdinLogCap` moved to
  `options.stdinLogCap`.") repeats the rule message word for word in the rendered
  diagnostic. The detail is now the fix: "Rename this `config` object to `options`;
  `config` now holds named build configurations." 

## Summary

The risk here is low and local. The rename is a mechanical breaking change with a
targeted error. The lossless reader is a standalone parser checked against
tinyjson on 1590 real manifests. The validator is pure functions with a test per
rule. Codegen is untouched, so valid projects stay byte-identical.
