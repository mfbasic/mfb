# plan-167-B: `os::getConfigOr`, folded from the `release` config

Last updated: 2026-10-03
Effort: large (estimate: 3h–1d)
Depends on: plan-167-A

This sub-plan adds the three `os::getConfigOr` overloads and bakes config values
into the program. Both arguments must be literals. After every imported package
is merged into the executable's IR, each call is replaced by the selected
config's value, or by the default when the key is absent. Nothing is looked up
at runtime. This sub-plan always selects `release` (plan-167-C adds `--config`
and `project-local.json`).

The outcome: a program calling `os::getConfigOr("retries", 1)` prints the
`release` config's `retries` when it is declared, and `1` when it isn't. A call
whose default type differs from the declared value's type fails the build with a
diagnostic at the call's file, line and column. A call with a non-literal key or
default fails the build too. This holds in the root project and in source and
`.mfp` dependencies.

References:

- plan-167-A (the `ProjectConfig` model, `src/manifest/config.rs:parse_config`).
- `src/codegen/registry/mod.rs:Implementation`, `RegistryFunction`, `Body`,
  `match_overload` (first implementation whose parameters unify wins).
- `src/codegen/builtins/collections/func_sum.rs`: an overload set that differs by
  parameter and return type.
- `src/codegen/builtins/os/func_get_env_or.rs`: the closest sibling (name, shape,
  docs).
- `src/target/shared/lower.rs:lower_project`: merges packages
  (`nir::merge_packages`), then `runtime::required_helpers`, then
  `nir::lower_module`. It's the one `NirModule` producer for all five targets.
- `src/codegen/engine/types/type_utils.rs:native_immediate_value`: Float `Const`
  values are parsed with `str::parse::<f64>`, which is correctly rounded.
- `src/codegen/builtins/tests/declared_errors_sweep.rs:lower_member`: lowers only
  `AbiFunction`/`AbiInline` bodies, so it skips `Body::Intrinsic` members.
- `src/ir/shape.rs` (pre-lowering HIR checks), `src/ir/types.rs:IrFunction.file`,
  `IrSourceLoc`.
- `mfb spec stdlib os` (`src/docs/spec/stdlib/14_os.md`), `.ai/man-content.md`,
  `.ai/compiler.md`, `.ai/codegen-invariants.md`.

## Prerequisites

The feature-wide gate is in plan-167-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-167-A complete | `ls planning/plan-167-A-*` → no match (archived to `planning/completed/`) | NOT MET (2026-10-03) |
| `os::getConfigOr` does not exist yet | `grep -rn getConfigOr src` → no hits | MET (2026-10-03) |

Everything below assumes these hold.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all*
> prerequisites.

## 1. Goal

- `os::getConfigOr(key AS String, default AS String) AS String`, `… default AS
  Integer) AS Integer` and `… default AS Float) AS Float` are published in the
  `os` package. Each call returns the selected config's value for `key`, or
  `default` when the config doesn't declare `key`.
- The value is a compile-time constant. A built executable contains no config
  table and no lookup code.
- A declared value whose type differs from `default`'s type is a build error
  (`OS_CONFIG_TYPE_MISMATCH`) at the call's `file:line:column`.
- A `key` or `default` that isn't a literal is a build error
  (`OS_CONFIG_ARGUMENT_NOT_LITERAL`) at the call site. This includes package
  builds.
- Programs that don't call `os::getConfigOr` build byte-identically to the
  pre-B compiler.

### Non-goals (explicit constraints)

- No runtime config lookup and no new runtime error code. "Wrong type" is a
  compile-time error (Open Decision D7).
- No `os::getConfig` without a default (requirement 4).
- No `--config` and no `project-local.json`: that's C. Here the selected config
  is always `release`.
- Only values a call references end up in the binary. Unreferenced config keys
  never reach codegen.
- A package's own `config` section does not apply to its code. The consuming
  executable's config applies to every call, including package code, the same
  rule `maxBuffer` follows (`src/ir/binary.rs`: "the CONSUMING project's
  project.json"). A package's `config` is still validated by its own build
  (plan-167-A).

## 2. Current State

- **No compile-time-folded `os` member exists.** All `os` bodies are
  `abi_function` (`grep -rn "body: Body::" src/codegen/builtins/os/*.rs` → 28
  bodies, all `Body::abi_function`). `os::name` and `os::arch` are constant per
  target, but they still lower to a helper that writes the bytes
  (`os/gen_introspect.rs:lower_const_string`).
- **Package IR merges in exactly one place.** That's `lower_project`, which calls
  `nir::merge_packages`. A pass placed right after it sees the root program's
  calls and every imported package's calls, on every backend and on the dump
  path.
- **IR calls carry `loc: IrSourceLoc { line, column }`**
  (`src/ir/value.rs:IrValue::Call`), and every `IrFunction` carries `file`
  (`src/ir/types.rs`). That's enough for a `file:line:column` diagnostic after
  the merge.
- **Float constants are safe to pass as JSON lexemes.**
  `native_immediate_value` turns a `Const` Float's `value` into bits with
  `parse::<f64>`, which accepts every lexeme plan-167-A §5.2 admits (a JSON
  number with `.` or an exponent). Verified by reading `type_utils.rs`.
- **The config reaches the backends through `write_executable`.**
  `build_project` already passes `stdin_log_cap` and `DebugOptions` into
  `target::write_executable` → `lower_project`. A dump path at
  `src/cli/build/mod.rs` (the writer call near `DebugOptions { enabled:
  options.debug }`, the second of the two) passes `debug` but not
  `stdin_log_cap`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `os` member files | 24 | `ls src/codegen/builtins/os/func_*.rs \| wc -l` → 24 |
| `target::shared::lower::lower_project` callers (every one gains the config argument) | 12: `testutil.rs` ×2, `macos_aarch64/mod.rs` ×6 (one executable path, five dump paths passing `stdin_log_cap: None`), and one each in `linux_aarch64`, `linux_x86_64`, `linux_riscv64`, `win_x86_64` | `grep -rn "lower::lower_project(" src --include='*.rs' \| wc -l` → 12 |
| Executable-path `write_executable` callers in `src/target.rs`, `src/target/*/mod.rs` and `src/cli` (the backend trait chain the config rides; the `os/*/link` ones are linker-level and untouched) | 6 | `grep -rn "write_executable(" src/target.rs src/target/*/mod.rs src/cli --include='*.rs' \| grep -v "fn write_executable" \| wc -l` → 6 |
| Highest used `2-203` rule code | `2-203-0140` | `grep -o 'code: "2-203-[0-9]*"' src/rules/table.rs \| sort \| tail -1` |
| `rt-behavior/os` fixtures | 32 | `ls tests/rt-behavior/os \| wc -l` → 32 |

### Verified properties

- *An `Intrinsic` member is skipped by the declared-errors sweep.* Read
  `declared_errors_sweep.rs:lower_member`: `if !matches!(…, Body::AbiFunction {
  .. } | Body::AbiInline(_)) { continue; }`. Verified.
- *An unfolded `os.getConfigOr` call that reaches codegen fails the build instead
  of miscompiling.* **UNVERIFIED.** Task B-0 checks it with a temporary
  registration and no fold (see Phase 1).
- *IR string `Const.value` holds the decoded text (not a quoted or escaped
  lexeme).* **UNVERIFIED.** Task B-0 reads the IR lowering of a string literal
  (`src/ir/lower.rs`, the HIR string literal arm) and records the encoding here.
  The fold must produce the same encoding.
- *A negative numeric default (`-5`, `-2.5`) is a literal at the HIR level that
  `shape.rs` can recognize.* **UNVERIFIED.** Task B-0 reads how the parser and
  HIR represent `-5` (a literal, or unary minus on a literal) and records it.

## 3. Design Overview

The three overloads are registered with `Body::Intrinsic`. They exist only for
type checking and docs. Two checks and one fold enforce the meaning:

1. **Literal check (front end, every build).** A new rule in `src/ir/shape.rs`
   rejects a call whose `key` isn't a String literal, or whose `default` isn't a
   literal of the overload's type. A negative numeric literal counts. This runs
   on package builds too, so a package can't ship a call that can never fold.
2. **Fold (after the package merge, every executable/test/dump build).** A new
   pass `src/target/shared/config_fold.rs:fold_config_calls(&mut IrProject,
   &SelectedConfig) -> Vec<ConfigFoldError>`, run in `lower_project` right after
   `merge_packages` and before `required_helpers`, replaces every
   `IrValue::Call { target: "os.getConfigOr", args: [key, default], .. }` with:
   - `IrValue::Const { type_: <default's type>, value: <config value> }` when the
     config declares `key` and the types match;
   - `args[1]` (the default, itself a literal) when the config doesn't declare
     `key`;
   - a `ConfigFoldError { file, loc, key, declared, requested }` when the types
     differ.

   Dropping `default` is safe because rule 1 makes it a literal with no side
   effects.
3. **Diagnostics.** `lower_project` renders each `ConfigFoldError` with
   `rules::show_diagnostic(OS_CONFIG_TYPE_MISMATCH, …, file, line, column)` and
   returns `Err`, so the build fails.

`SelectedConfig { name: String, values: BTreeMap<String, ConfigValue> }` comes
from `ProjectConfig` (plan-167-A). `build_project` computes it on the executable
and test paths and passes it to `write_executable` → `lower_project`, and to the
dump writer.

**Where correctness risk concentrates:** the fold's coverage of every IR
position a call can sit in. A missed position leaves an unfolded intrinsic that
must fail loudly (B-0 verifies this), never silently miscompile. The other risk
is the constant's encoding (String escapes, Float lexeme, Integer range).

**Where design uncertainty concentrates:** the three UNVERIFIED properties above.
All three are cheap reads, scheduled first (B-0).

**Gate class:** a behavior-adding plan. Programs without `getConfigOr` must stay
byte-identical (checked on one unaffected fixture). The new fixtures are expected
to produce new goldens.

**Rejected alternatives:**

- *Runtime lookup table plus a new runtime error code for wrong types.* Rejected
  (D7). It embeds every config value, including unreferenced secrets, into every
  binary. It turns a build-time mistake into a runtime failure. And it needs a
  map-lookup codegen path, a new `errorCode::` row and a `declared_errors_sweep`
  entry.
- *`Body::AbiInline` that reads the value from `AbiCtx`.* Rejected: it would
  thread config through `AbiCtx` on both the inline and helper construction
  paths, and need fresh-copy string emission for a value that is just a literal.
  Folding to `IrValue::Const` reuses the literal machinery as it is
  (string data objects, Float immediates).
- *Fold during IR lowering (`ir/lower.rs`).* Rejected: it would bake a package's
  own config into its `.mfp`, and the consuming app's config must apply instead.
- *Allow a non-literal default and fold only the key.* Rejected: a call that
  finds its key would have to keep evaluating the default for its side effects,
  which a `Const` replacement can't do. A literal default is how every
  `getConfigOr` call in the requirements reads anyway.

## 4. Registry entry (`src/codegen/builtins/os/func_get_config_or.rs`)

- One `RegistryFunction { name: "getConfigOr", … }` with three implementations,
  ordered String, Integer, Float. The parameters are `key AS String` and
  `default AS <T>`, the return type is `T`, `errors: vec![]`, and the body is
  `Body::Intrinsic`.
- Docs (`INTRO`, `DESC`, `EX`) are held to `.ai/man-content.md`. They must say:
  - the value is fixed when the program is built, from the `release` config in
    `project.json` (C extends this to `--config` and `project-local.json`);
  - both arguments must be literals;
  - a missing key yields `default`;
  - a type mismatch stops the build;
  - every value a program reads is stored as plain text in the executable, so
    anyone holding the binary can read it.

  No memory vocabulary (`scripts/man-census.sh --memory-scope` → 0
  unclassified).
- Registered from `src/codegen/builtins/os/mod.rs:register`, beside `getEnvOr`.

## 5. Diagnostics

| Rule | Code | Where emitted | Detail |
|---|---|---|---|
| `OS_CONFIG_ARGUMENT_NOT_LITERAL` | 2-203-0141 | `src/ir/shape.rs` | "`os::getConfigOr` needs a literal `{param}`; the value is read when the program is built." |
| `OS_CONFIG_TYPE_MISMATCH` | 2-203-0142 | `lower_project` via `fold_config_calls` | "Config `{config}` declares `{key}` as {Declared}, but this call's default is {Requested}." |

Both are severity error. The implementer confirms in
`src/docs/spec/diagnostics/01_rule-codes.md` that `2-203` is the right subsystem
for a front-end call check. If another family fits better, record the move in
Corrections.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Use `- [~]` for partial work, with one line saying what's
> left. Mark moot work `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` the
> moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1: B-0 measurements, registry entry, literal check

This phase ships the typed surface with a front-end rule. Nothing folds yet, so
**this phase does not land alone**: Phase 1 and Phase 2 land as one commit (see
Acceptance). Phase 1 exists as a separate checklist so the B-0 facts are settled
before the fold is written.

- [ ] B-0: re-run the two caller-count commands in "Measured populations". If
      either count moved, record it in Corrections.
- [ ] B-0: resolve the three UNVERIFIED properties in "Verified properties":
      string `Const.value` encoding, the HIR shape of `-5`, and the
      unfolded-intrinsic behavior. For the last one, register the member, build
      `os::getConfigOr("k", "d")` with no fold, and record the exact failure.
      If it builds and runs, that's a missing guard: add a codegen-time
      `Err("os.getConfigOr reached codegen unfolded")` at the intrinsic dispatch
      as a task here.
- [ ] `src/codegen/builtins/os/func_get_config_or.rs` per §4, registered in
      `os/mod.rs`.
- [ ] `src/rules/table.rs`: `OS_CONFIG_ARGUMENT_NOT_LITERAL` (2-203-0141).
- [ ] `src/ir/shape.rs`: the literal check (§3.1). Skip injected `builtins/`
      source, like `check_enum_member_advisory`.
- [ ] `src/codegen/builtins/tests/os.rs`: extend the corpus program and
      `register_publishes_the_whole_os_surface` with `getConfigOr`'s three
      overloads.
- [ ] Fixture `tests/syntax/os/func_os_getConfigOr_invalid/`: a non-literal key
      (a `LET` String) and a non-literal default (a function call), with a golden
      `build.log` showing both 2-203-0141 diagnostics. Also a package-kind
      fixture `tests/syntax/os/func_os_getConfigOr_package_not_literal/`, which
      proves the rule fires on a package build.

Acceptance: see Phase 2. This phase is complete only when both are.
Commit: —

### Phase 2: Selected config, fold and type check

This phase makes calls return config values everywhere a program can be built.

- [ ] `src/manifest/config.rs`: `SelectedConfig` and
      `ProjectConfig::select(&self, name) -> Option<SelectedConfig>`, plus
      `pub(crate) fn project_config(manifest_text) -> Result<ProjectConfig, …>`
      for the build (A's validator already proved the text valid).
- [ ] `src/cli/build/mod.rs:build_project`: on the executable/test path, compute
      `SelectedConfig` for `release` and pass it to `target::write_executable`
      and to the dump writer.
- [ ] Thread `&SelectedConfig` through every `write_executable` and
      `lower_project` caller listed in Measured populations (12 + 6). Non-executable callers (package
      publishing, the ABI check) pass an empty `release` with a comment
      explaining why; those builds emit `.mfp` and never fold.
- [ ] `src/target/shared/config_fold.rs:fold_config_calls` per §3.2. It walks
      every `IrValue` in every function body, global initializer and nested
      value. Use the existing IR mutable walker if B-0 finds one; otherwise write
      the walk, with a test that builds a call in each position (statement
      expression, nested argument, record field, list literal, lambda body,
      global initializer). The Integer value is the decimal `i64`, the Float
      value is the config lexeme, and the String value uses the encoding B-0
      recorded.
- [ ] `src/target/shared/lower.rs:lower_project`: call the fold after
      `merge_packages`, render errors with `OS_CONFIG_TYPE_MISMATCH`
      (2-203-0142, new row in `rules/table.rs`), and return `Err` if any.
- [ ] Unit tests in `config_fold.rs`: found / missing / mismatch for each of the
      three types; an `i64::MIN` value; Floats `0.1`, `1e300`, `5e-324`, `-0.0`;
      a String with `"`, `\`, a newline and non-ASCII text.
- [ ] Fixture `tests/rt-behavior/os/func_os_getConfigOr_valid/`: a project.json
      with `config.release` holding a String, Integer and Float. It prints each
      found value, plus defaults for three undeclared keys. Golden `build.log`.
- [ ] Fixture `tests/syntax/os/func_os_getConfigOr_type_mismatch/`: golden
      showing 2-203-0142 at the call's line and column.
- [ ] Fixture `tests/rt-behavior/packages/source-package-get-config-or-rt/`,
      modelled on `source-package-dependency-rt`. A source package's function
      calls `os::getConfigOr("greeting", "pkg-default")`. The package's own
      project.json declares `greeting` = "from-package" and the app's declares
      "from-app". The golden shows "from-app".
- [ ] Spec `stdlib/14_os.md`: a new `### Build-time configuration
      (getConfigOr)` section stating literal-only, fold-after-merge, the
      consuming-executable rule and the two codes, with
      `[[src/target/shared/config_fold.rs:fold_config_calls]]`. Add 2-203-0141
      and 0142 to `diagnostics/01_rule-codes.md`.

Acceptance: config values reach the program from root and package code, mismatches
and non-literals are rejected, and programs without the call are unchanged.
  Check:
  `scripts/test-accept.sh target/release/mfb /tmp/acc-167 'func_os_getConfigOr*' 'source-package-get-config-or-rt'`
  → all pass (est. 3 min). Then `cargo test --bin mfb config_fold` → pass
  (est. 2 min). Then rebuild `tests/byte-identity/strings` and confirm its
  `.ncode` sha matches its golden (est. 1 min). That fixture doesn't call
  `getConfigOr`, so a diff there means the threading changed codegen.
Commit: —

### Phase 3: Man page verification

This phase checks that the developer-facing page renders, its example compiles
and runs, and it uses no banned words.

- [ ] `mfb man os getConfigOr` renders all three overloads, with Synopsis and
      Parameters filled.
- [ ] Run the page's example.

Acceptance: the page passes the man-content instruments.
  Check: `scripts/man-run-examples.sh os --run` → every example passes
  (est. 3 min). Then `scripts/man-census.sh --memory-scope` → 0 unclassified
  (est. 1 min).
Commit: —

## Validation Plan

- Tests: unit tests in `config_fold.rs` and `builtins/tests/os.rs`; fixtures for
  valid, non-literal (app and package), type mismatch, and package-code-uses-app-config.
- Coverage check: `grep -rl "2-203-014[12]" tests/syntax/os/*/golden/build.log | wc -l`
  → ≥ 2.
- Runtime proof: build `tests/rt-behavior/os/func_os_getConfigOr_valid`, run the
  `.out`, then edit its `release.retries` and rebuild. The printed value changes
  with no source edit.
- Doc sync: `mfb spec stdlib os`, `mfb spec diagnostics rule-codes`,
  `mfb man os getConfigOr`. Gate: `cargo test --bin mfb spec`.
- Final gate: in plan-167-C.

## Open Decisions

- **D7, literal-only arguments with a compile-time fold.** Recommended (§3). The
  alternative is a runtime table with a runtime error. See Rejected alternatives
  for why it loses.

## Corrections

(Filled in during execution.)

## Summary

The risk is in the fold's coverage and the encoding of the constant it emits.
B-0 front-loads the three facts the fold depends on, and Phase 2's position test
pins the coverage. No runtime code, no runtime error code, and no `AbiCtx`
change. Programs that don't use the call are untouched.
