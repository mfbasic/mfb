# plan-136-B: Package parameter defaults work exactly as they do in an executable

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-136-A (hidden default functions and `CallDefault` exist and are tested)

Prerequisites: see plan-136-A §Prerequisites, plus: `ls planning/plan-136-A-*` → no match
(plan-136-A archived to `planning/completed/`). If plan-136-A is not complete, this letter cannot
start, full stop.

**Status (re-run 2026-09-13, before Phase 1): all MET.** `git merge-base --is-ancestor 53f10b1fe
main && echo MET` → `MET`; `git log --all --oneline --grep='plan-136'` → only `caf191edd` and this
session's four plan-136-A commits; `ls bugs/bug-614-*.md` → one file; `ls planning/plan-136-A-*` →
`no matches found` (archived as `planning/completed/plan-136-A-default-declaration-scope.md` in
`79425a695`).

An importer's call that omits a defaulted argument of an imported package function (`FUNC` or
`SUB`; a package exposes a `LINK` function only through an exported MFB wrapper — owner ruling
2026-09-13, see Corrections) passes the default **evaluated on that call in the package's scope** —
the same value the same code produces in an executable. A package whose default reads a
package-private global builds. Today every omitted package default is broken: a literal default is
never passed (garbage or a crash), and a computed one does not build.

References:

- plan-136-A (the rules, `DefaultKind`, hidden default functions, `CallDefault`).
- `mfb spec package functions` (`src/docs/spec/package/07_functions.md`: entry layout, parameter
  flags), `03_metadata-encoding.md` (`sigHash` input), `05_constant-pool.md`, `08_ir-section.md`.
- `mfb spec architecture binary-representation` (`05_binary-representation.md`, decode-and-merge).
- `.ai/resources-packages.md` (package/import subsystem), `.ai/testing-gates.md` (`.mfp` goldens,
  hermetic `MFB_HOME`).

## 1. Goal

- `deflib::f()` for `EXPORT FUNC f(x AS Integer = 5)` prints `5`; `deflib::g(1)` for
  `g(a AS Integer, s AS String = "dflt")` returns `"1dflt"`.
- A package default that reads a package-private `MUT` global builds, and each omitted call sees
  the global's current value — even when the importer has a local with the same bare name.
- A named-argument call that skips a defaulted middle parameter fills it.
- A corrupt `.mfp` whose parameter record points at an invalid default function is refused.

### Non-goals (explicit constraints)

- **Existing packages stay byte-identical and valid.** A literal default keeps parameter flag bit 0
  and its `CONST_POOL` id; its `sigHash` input is unchanged. Proof: regenerating every committed
  `.mfp` produces no diff.
- **No `ABI_FORMAT_VERSION`, `MFPC_MAJOR_VERSION` or `BINARY_REPR_VERSION` bump** — the record
  layout (16 bytes) and the MFBR payload shape do not change.
- **No `repository/` code change** — the registry parses no `FUNCTION_TABLE`/`CONST_POOL`.
- No default text is rendered by `mfb man`, `mfb pkg doc` or the registry Docs tab today, and none
  will be.
- Registry/built-in defaults unchanged (plan-136-A non-goal).

## 2. Current State

**Writer.** `binary_repr::writer::lower_function` writes each 16-byte parameter record
(`name | type_id | flags | default_const`): `flags` = 1 if a default exists, `default_const` =
`constants.add(strings, default)?` or `u32::MAX`. `ConstPool::add` (`binary_repr::sections`) accepts
only a scalar `IrValue::Const` — everything else is `only constant IR values can be stored in
CONST_POOL`, the bug-614 package error. `encode_functions` writes the fields.

**Reader.** `binary_repr::reader::read_function_table` reads `default_const` without checking it.
`binary_repr::builder::package_exports` derives `has_default: param.flags & 1 != 0`.
`07_functions.md` reserves bit 1 (resource non-owning) and bit 2 (resource consume); no code reads or
writes them, so **bit 3 is free**.

**ABI hash.** `binary_repr::sections::function_sig_hash` hashes per parameter: `0` for no default,
or `1` + `serialize_const(default_const)`. `reader::validate_abi_index` recomputes every export's
hash and rejects a mismatch. `ABI_FORMAT_VERSION` (`wire/src/mfpc.rs`) is `1`, exact-match only, and
has never been bumped (`git log -G'ABI_FORMAT_VERSION: u16 = [02-9]'` → empty).

**Bodies travel already.** `SECTION_BINARY_REPR` carries the whole `IrProject`
(`ir::binary::encode_function`, including **private** functions, and `encode_param` with the full
`IrParam.default` value). `FUNCTION_TABLE` lists every function; non-exports carry
`FUNCTION_FLAG_PRIVATE`.

**Import.** `ir::package::prefix_package_symbols` and `apply_package_identity` rename every
package function — private ones included (`package_qualified_reference_names` collects them all) —
to `<id>.<pkg>.<name>` and rewrite `pkg.symbol` references; `visit_project_targets_mut` already
walks `param.default`.

**The missing fill.** The importer's lowering context (`ir::lower::lower_facts`) builds external
`CallParam { default: None }` from `ir::types::ExternalFunctionParam { has_default: bool }` — a flag
with no value. `lower_local_call_arguments` therefore drops the omitted slot, `ir::shape` accepts
the short call (it counts `has_default`), and no later stage fills it
(`nir::lower::apply_default_args` handles only `fs.*`; `monomorph::lower` does not pad).

### Measured populations

| What | Count | Command |
|---|---|---|
| Committed `.mfp` files | 160 | `git ls-files '*.mfp' \| wc -l` |
| `.info` goldens carrying `sigHash` | 11 | `git ls-files '*.info' \| xargs grep -l -i 'sighash' \| wc -l` |
| Package sources with a defaulted parameter | **1** (corrected from 0): `packages/sqlite3/src/lib.mfb:469` `EXPORT FUNC open(path AS String, readOnly AS Boolean = FALSE) AS RES Db` — a LITERAL default (bytes unchanged by this letter), with no committed `.mfp` (`git ls-files 'packages/*.mfp' 'packages/sqlite3/*.mfp'` → empty). It is a live instance of this letter's bug: an importer calling `sqlite3::open(path)` passes no `readOnly` today. No committed importer does: `grep -rn --include='*.mfb' 'sqlite3::open([^,)]*)' tests examples packages tools` → only the doc comment at `packages/sqlite3/src/lib.mfb:22`. | `grep -rEln --include='*.mfb' '\b(FUNC\|SUB)\b[^(]*\([^)]*[^:<>=!]=[^=>]' tests/syntax/packages tests/rt-behavior/packages tools packages` (re-run 2026-09-13; the plan's original command omitted `packages/`) |
| `binary_repr` unit test files | 14 under `src/binary_repr/tests/` | `ls src/binary_repr/tests/*_tests.rs \| wc -l` |

### Verified properties (probe, 2026-09-13)

P6 — source package `deflib` (`kind: package`, `file:` dependency) exporting
`f(x AS Integer = 5)` and `g(a AS Integer, s AS String = "dflt")`; importer prints
`deflib::f(7)`, `deflib::f()`, `deflib::g(1)`: build exit 0; prints `7`, then `4374773792`, then
exits **139**. A literal package default has never been passed.

**Measured in Phase 3 (was UNVERIFIED):** a compiler built before this letter refuses a package
using bit 3. `/tmp/p136b-old` = `git worktree add --detach … 6c6f85e64` (plan-136-B Phase 1, the
last commit before the format change), `cargo build --release` there; package `oldlib`
(`PRIVATE FUNC seed()`, `EXPORT FUNC f(x AS Integer = seed())`) built with THIS letter's compiler,
then imported (`file:packages/oldlib.mfp`) by the old one → `error[2-201-0001
IMPORT_PACKAGE_INVALID]: imported package binary could not be read — Package `oldlib` type exports
could not be read: failed to read '…/oldlib.mfp': unknown const id 2`. The refusal is earlier than
the predicted `sigHash` mismatch: the old `function_sig_hash` treats the record's function index
as a `CONST_POOL` id while recomputing the hash. Refused either way, never miscompiled.

## 3. Design Overview

- **Parameter record.** Literal default: unchanged (bit 0, `CONST_POOL` id). Computed default: bit 0
  **and** new bit 3 `PARAM_FLAG_DEFAULT_FUNCTION`; `default_const` holds the `FUNCTION_TABLE` index
  of the parameter's hidden default function (plan-136-A §4.2), which already travels as a private
  function.
- **`sigHash`.** A bit-3 parameter hashes the byte `2` (no body hash — a default expression is not
  ABI). Literal and absent defaults hash exactly as today.
- **Reader validation (fail closed).** Bit 3 requires bit 0; the index is in range; the target
  carries `FUNCTION_FLAG_PRIVATE`, has zero parameters, returns the parameter's type id, and its name
  satisfies `internal_name::is_hidden_default_function`. Anything else refuses the package.
- **Export signature.** `ExternalFunctionParam.has_default: bool` becomes
  `default: ExternalDefault { None, Literal(IrValue), Function(String) }` (literal value from
  `CONST_POOL`; function as `<pkg>.<hidden name>`), carried through
  `manifest::package::package_export_signature`; `ir::shape` arity reads it.
- **Importer lowering.** `lower_facts` maps `ExternalDefault` to plan-136-A's `CallDefault`, so
  `lower_local_call_arguments` fills imported calls exactly as local ones; `apply_package_identity`
  qualifies the hidden-function call like any `pkg.symbol`.

**Correctness risk:** the reader's new validation (a crafted package is untrusted input — bug-578
territory) and the importer fill for every imported call. **Design uncertainty:** the old-compiler
rejection claim — measured in Phase 3, not assumed.

**Byte-identity is a gate here for one thing only:** every committed `.mfp` regenerates with no
diff (literal-default packages unchanged). Behavior tests gate the rest.

**Rejected alternatives.** *Serialize the default expression into the record* — needs a second
expression encoding and name rewriting at the importer, while the hidden function already travels.
*Derive the hidden function by name instead of an index* — an explicit, validated index is
fail-closed; a name convention is a guess the reader cannot verify. *Bump `ABI_FORMAT_VERSION`* —
the layout does not change, and a bump rejects every existing `.mfp` (its own doc comment).

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the work; `- [~]`
> partial; moot tasks struck through with evidence; fill `Commit:` on landing. **An unticked box
> means NOT DONE.**

### Phase 1 — RED runtime tests

- [x] Add `tests/runtime/rt_package_parameter_defaults.rs` (copy `run_app`/`build` from
      `tests/runtime/rt_top_level_initializer_globals.rs`; register in `Cargo.toml` the same way).
      Registered at `Cargo.toml:606` (`grep -n -B1 -A2 'rt_package_parameter_defaults' Cargo.toml`).
      The helpers gained a per-package `manifest_extra` (a package `libraries` table) and a build
      that returns its output on failure (for the pin). Cases:
  - [x] `an_omitted_literal_integer_package_default_is_passed` — P6 `f()` → `5` (RED: garbage).
        Measured: `left: ["7", "4377837600"] right: ["7", "5"]`.
  - [x] `an_omitted_literal_string_package_default_is_passed` — P6 `g(1)` → `1dflt` (RED: 139).
        Measured: `the app exited killed by signal 11 (SIGSEGV)`.
  - [x] `a_computed_package_default_reads_a_private_global_on_each_call` — package
        `PRIVATE MUT Limit = 5`, `EXPORT SUB setLimit(n)`, `EXPORT FUNC f(x AS Integer = Limit)`;
        importer has `LET Limit AS Integer = 99`, prints `deflib::f()`, calls `setLimit(6)`, prints
        again → `5`, `6` (RED: package build fails with the CONST_POOL error). The test also prints
        the importer's own `Limit` last (`99`). Measured: `build failed: Building limits (package) …
        error: only constant IR values can be stored in CONST_POOL`.
  - [x] `a_named_package_call_fills_a_skipped_middle_default` — `f(a AS Integer, b AS Integer = 2,
        c AS Integer = 3)`, `deflib::f(1, c := 9)` → `1 2 9` (RED). Measured: `left: ["1 9
        82592681109987"]` — the unfilled middle slot shifts `c := 9` into `b` and `c` reads garbage.
  - [x] `a_package_default_calling_another_packages_export_is_filled` — package B's default calls
        package A's export; the app imports B only (RED). Measured: `Building userpkg (package) …
        error: only constant IR values can be stored in CONST_POOL` (the app manifest lists `userpkg`
        only; `userpkg` lists `basepkg`). **Corrected in Phase 3:** "the app imports B only" cannot
        build for ANY cross-package call, default or not (Corrections); the app now lists both
        packages, as `rt_top_level_initializer_globals`'s chain test does.
  - [x] ~~`an_exported_link_function_default_is_filled` — a `kind: package` project exporting the
        P10 `absval(n AS Integer = -5)` LINK function; importer `absval()` → `5` (RED).~~ Replaced
        by owner ruling (Corrections): a LINK function is not directly exportable. Converted to
        `a_link_wrapper_package_fills_both_defaults` — package `cmath` with LINK `absval(n AS
        Integer = -5)`, an exported wrapper `FUNC absval(n AS Integer = -7) AS Integer` that calls
        `libc::absval(n)`, and an exported `FUNC absDefault() AS Integer` that calls
        `libc::absval()`; importer prints `cmath::absval(-3)`, `cmath::absval()`,
        `cmath::absDefault()` → `3`, `7`, `5`. RED: the package does not build — `-7` is a COMPUTED
        default (a unary minus; plan-136-A Open Decisions), so its hidden-function call reaches the
        writer. Measured (`cargo test --release --test rt_package_parameter_defaults
        a_link_wrapper_package_fills_both_defaults -- --test-threads=1`): `build failed: error: only
        constant IR values can be stored in CONST_POOL`, `test result: FAILED. 0 passed; 1 failed`.
  - [x] `a_package_default_naming_a_parameter_is_refused_at_package_build` — located
        `SYMBOL_DEFAULT_NAMES_PARAMETER` from plan-136-A (GREEN pin). Measured: `... ok`.

Acceptance: RED cases fail for the documented reason; the pin passes.
  Check: `cargo build --release && cargo test --release --test rt_package_parameter_defaults --
  --test-threads=1` → six fail, one passes (est. 6 min).
  **Measured 2026-09-13:** full run → `test result: FAILED. 1 passed; 6 failed; … finished in
  66.17s`, each failure for the reason recorded on its line, the pin `ok`. The exported-LINK case in
  that run failed for a DIFFERENT reason (`Package `cmath` does not export `absval``), which is how
  the owner ruling above was reached; its replacement was then run alone and fails for its recorded
  reason.
Commit: —

### Phase 2 — the parameter record, `sigHash` and validation

- [x] `binary_repr/mod.rs`: `PARAM_FLAG_DEFAULT_FUNCTION = 1 << 3` beside the record struct.
      Placed beside the function flags, with `PARAM_FLAG_DEFAULT = 1 << 0` naming the existing bit;
      compiled (`cargo build --release --bin mfb` → `Finished`) and exercised by every test below.
- [x] `binary_repr::writer::lower_function`: literal default → unchanged; hidden-function default
      (`IrParam.default` is the zero-argument call to an `is_hidden_default_function` name) → bits
      0|3 and the function's `FUNCTION_TABLE` index. Any other shape is a writer error naming the
      function and parameter (a total match, no default arm). `writer::lower_param_default` lists
      all 23 `IrValue` variants; `lower_function` gained `function_ids` for the index.
      `read_binary_repr_package_round_trips_a_function_default ... ok` (flags `0b1001`, index `3`;
      the literal beside it keeps flag `1`).
- [x] `binary_repr::sections::function_sig_hash`: byte `2` for bit-3 parameters (tested before the
      `u32::MAX` check, since a bit-3 record stores an index there).
      `function_sig_hash_distinguishes_a_function_default ... ok`: differs from no default and from
      a literal, and equal for indices 3 and 9.
- [x] `binary_repr::reader`: validate bit-3 records per §3 in `read_function_table` (or the first
      point the function table is known), with a message naming the function and parameter.
      `reader::validate_default_functions`, called right after `read_function_table` in
      `read_binary_repr_package`. `validate_default_functions_accepts_what_the_writer_produces ...
      ok`; `validate_default_functions_refuses_each_invalid_record ... ok`.
- [x] `binary_repr::builder::package_exports` and `manifest::package::package_export_signature`:
      produce `ExternalDefault`. Done on the binary side: `BinaryReprExportParam.default:
      BinaryReprExportDefault { None, Literal { type_, value }, Function(name) }` via
      `builder::export_default`, which decodes a literal `CONST_POOL` entry kind by kind (new exact
      inverses `numeric::fixed_decimal_from_raw` / `money_decimal_from_raw`, with round-trip tests
      `... ok`). `builder_tests` confirms `main.params[1].default` decodes to `Literal { Integer, "0"
      }`. REMAINS: `manifest::package::package_export_signature` still maps to `has_default`,
      because `ir::types::ExternalDefault` does not exist until Phase 3's first task; that mapping
      lands with it. **Resolved in Phase 3:** `package_export_signature(package, export)` maps
      `BinaryReprExportDefault` → `ir::ExternalDefault`, qualifying a hidden function as
      `package.name` (the spelling `apply_package_identity` rewrites); both production callers
      pass their `package_name`. `cargo build --release --bin mfb` → `Finished` (its one warning,
      the now-unused `BinaryReprExportDefault::is_some`, removed — `builder_tests` compares against
      `BinaryReprExportDefault::None` instead).
- [x] Unit tests: `sections_tests.rs` `function_sig_hash_distinguishes_a_function_default`;
      `reader_tests.rs` `read_binary_repr_package_round_trips_a_function_default` and one rejection
      test per validation rule (out of range, not private, has parameters, wrong return type, bit 3
      without bit 0); `builder_tests.rs` extend `package_exports_lists_callables_with_signatures`;
      add a computed default to `fixtures.rs` `rich_project` only if no existing test pins its bytes
      (check `grep -n 'rich_project' src/binary_repr/tests/*.rs` users first). `rich_project` is
      pinned (`read_binary_repr_package_round_trips_rich_project` asserts `functions.len() == 3`,
      `package_info_reports_counts_and_metadata` asserts `function_count == 3`), so it is unchanged;
      a separate `fixtures::default_function_project` carries the computed default. The rejection
      test also covers a target that is not named as a hidden default function. `cargo test
      --release --bin mfb -- binary_repr:: numeric:: package_format` → `test result: ok. 188
      passed; 0 failed` (three test sites in `manifest/package.rs` that built
      `BinaryReprExportParam` with `has_default` were updated first: the initial run failed to
      compile with `E0560` at `package.rs:1541/1546/1594`).
- [x] A crafted corrupt fixture for the decoder corpus
      (`codegen::builtins::tests::package_format::every_corrupt_package_fixture_is_refused_by_the_decoder`):
      a bit-3 record pointing at an exported, one-parameter function; generated by
      `tools/security-package-sources/<new>/generate.py` via `mfp_craft.py`, per that tool's README.
      `tools/security-package-sources/pkg-08-default-function/generate.py` (new mutator
      `mfp_craft.mutate_default_function_target`) → `wrote tests/syntax/security/pkg-08-default-function/packages/sec_default.mfp
      (1585 bytes)`; consumer fixture `tests/syntax/security/pkg-08-default-function` (leaf unused:
      `git ls-files | grep -c 'pkg-08'` → 0) with golden `build.log` → `error[2-201-0001
      IMPORT_PACKAGE_INVALID]: … function `f` parameter `x` has an invalid default function: the
      target function is not private`; `scripts/test-accept.sh target/release/mfb
      /tmp/p136b-p2-accept pkg-08-default-function` → `acceptance tests passed (1 test(s) ran)`;
      corpus row added, `every_corrupt_package_fixture_is_refused_by_the_decoder ... ok`. Both
      READMEs (`tools/security-package-sources`, `tests/rt-behavior/security`) list PKG-08.

Acceptance: round-trip and every rejection test pass; the corrupt fixture is refused.
  Check: `cargo test --release --bin mfb binary_repr:: package_format` → `test result: ok`
  (est. 5 min).
  **Measured 2026-09-13:** `cargo test --release --bin mfb -- binary_repr:: numeric:: package_format`
  (the plan's filter plus `numeric::` for the two new decimal inverses) → `test result: ok. 188
  passed; 0 failed`; the round-trip, both validation tests and the corpus test `... ok` by name.
Commit: ba303b258

### Phase 3 — importer fill, compatibility proof

- [x] `ir::types::ExternalFunctionParam`: replace `has_default` with `default: ExternalDefault`;
      update every reader (`grep -rn 'has_default' src --include='*.rs'`). `ExternalDefault { None,
      Literal { type_, value }, Function(package.name) }`, re-exported from `ir`. Readers:
      `ir::shape` (below), `manifest::package::package_export_signature` (Phase 2's resolved box),
      and five test literals in `ir/shape.rs` and `ir/tests.rs`. The remaining `has_default` hits
      are `ShapeParam`'s own field and `NirImportParam`, which no code constructs.
- [x] `ir::lower::lower_facts`: external `CallParam.default` from `ExternalDefault` →
      plan-136-A's `CallDefault`. A literal becomes the new `CallDefault::Constant(IrValue::Const)`,
      passed as-is, so re-lowering text can never classify it differently. A function becomes
      `CallDefault::Function(package.name)`, which `apply_package_identity` qualifies at merge.
      `rt_package_parameter_defaults` → `test result: ok. 7 passed; 0 failed`.
- [x] `ir::shape`: imported-call arity from `ExternalDefault` (`has_default: param.default.is_some()`
      in the imported-signature arm). `a_named_package_call_fills_a_skipped_middle_default ... ok`
      (a call omitting a middle argument passes the arity rule and is filled).
- [x] Old-compiler refusal: `git worktree add --detach /tmp/p136b-old <plan-136-B base commit>`,
      `cargo build --release` there, build the `a_computed_package_default…` package with the new
      compiler, and import its `.mfp` with the old one; record the exact error in **Verified
      properties**. Expected: refused (sigHash mismatch). If it is accepted, that is a finding to
      fix before landing (a package the old compiler would miscompile), not a stop. Base commit
      `6c6f85e64`; old build `Finished … in 2m 04s`. Refused, with `unknown const id 2` (recorded in
      §Verified properties — earlier than the predicted sigHash mismatch, same outcome). The probe
      package is the same shape as the case (a computed default reading a package-private name).

Acceptance: every `rt_package_parameter_defaults` case passes.
  Check: `cargo build --release && cargo test --release --test rt_package_parameter_defaults --
  --test-threads=1` → all pass (est. 6 min).
  **Measured 2026-09-13:** `cargo build --release --bin mfb` → `Finished`; the runtime test → first
  `6 passed; 1 failed` (the cross-package case, root-caused to the transitive-dependency premise —
  Corrections), then after listing both packages `test result: ok. 7 passed; 0 failed; … finished in
  64.64s`. The test code this phase changed (`ExternalFunctionParam` literals, the new
  `package_export_signature` argument) compiles and passes: `cargo test --release --bin mfb --
  ir::shape ir::tests manifest::package` → `test result: ok. 454 passed; 0 failed`.
Commit: —

### Phase 4 — byte-identity of existing packages, spec sync

- [ ] Regenerate every committed package: `scripts/sync-package-mfp.sh target/release/mfb` and the
      security generators (`python3 tools/security-package-sources/<pkg>/generate.py` for each
      existing one); then `git status --short -- '*.mfp' '*.info'` → **empty** (no committed package
      has a computed default, so none may change). Est. 10–15 min: it rebuilds 160 packages, and
      nothing smaller proves the literal path's bytes and `sigHash` are untouched across every shape
      in the corpus.
- [ ] `07_functions.md`: parameter flags (bit 3), `defaultConst` meaning for bit 3, the validation
      rules, with citations.
- [ ] `03_metadata-encoding.md`: the `sigHash` byte `2`.
- [ ] `05_constant-pool.md`: a computed default is not a constant; point at the flag.
- [ ] `05_binary-representation.md` (decode-and-merge): an imported call's omitted argument is
      filled from the export signature.
- [ ] `cargo test --release --bin mfb spec` → ok.

Acceptance: no committed `.mfp`/`.info` changes; spec describes the record.
  Check: the `git status` above → empty; `cargo test --release --bin mfb spec` → `test result: ok`
  (est. 3 min).
Commit: —

## Compatibility / Format Impact

- **Changes:** parameter flag bit 3 and the meaning of `default_const` when it is set; the
  `sigHash` input byte `2`; the reader's validation; `ExternalFunctionParam`'s shape (in-memory).
- **Unchanged:** record size and every other field; every existing package's bytes, identity id and
  `sigHash`; all format version constants; the registry.
- **Old compilers:** refuse a package that uses bit 3 (Phase 3 records the proof).

## Validation Plan

- Tests: `tests/runtime/rt_package_parameter_defaults.rs`; `binary_repr` round-trip and rejection
  tests; the corrupt-package fixture.
- Runtime proof: P6 prints `7`, `5`, `1dflt`, exit 0.
- Doc sync: Phase 4.
- Final gate: once, at the end of plan-136-C.

## Open Decisions

- `sigHash` for a computed default — recommended byte `2` only vs. hashing the hidden function's
  encoded body. A default expression is not ABI; a literal default's value is hashed today only
  because it is stored in the pool. Hashing bodies would make any internal refactor of a default
  look like an ABI break to `mfb repo check-abi`.

## Corrections

- **A LINK function is not directly exportable — owner ruling, 2026-09-13 (Phase 1).** The plan's
  `an_exported_link_function_default_is_filled` case presumed `EXPORT FUNC absval AS libc::absval`
  publishes a LINK function to importers. It does not, with or without a default, and never has:
  probe `/tmp/p136-probe-linkexport` (the same package with NO default) → importer build
  `main.mfb:5 error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: … Package `cmath` does not export
  `absval``, identically with main's pre-plan binary. `resolver::packages` unions an imported
  package's visible names from `EXPORT_TABLE`, the `RESOURCE` table and the `GLOBAL` table; a LINK
  re-export alias has no export row and is visible only when it is a resource's close op
  (`sqlite3::close`). LINK functions also have no `FUNCTION_TABLE` row, so this letter's parameter
  record could not carry their defaults. Asked, the owner ruled: *"I dont think directly exporting a
  LINK function is or should be supported at all. It should have a MFB function wrapper that is
  exportable."* So the Goal no longer lists exported LINK functions, and the case became
  `a_link_wrapper_package_fills_both_defaults`: an exported `FUNC` wrapper's default crosses the
  package boundary (this letter), and the LINK function's own default is filled inside the package
  (plan-136-A's `link_params`).
- **"The app imports B only" cannot build a cross-package call at all (Phase 3).** After the fill
  landed, `a_package_default_calling_another_packages_export_is_filled` failed at the app build with
  `error: NIR call target 'basepkg.base' does not resolve` (unlocated). Not a hidden-default defect:
  probe `/tmp/p136b-tbody` — `userpkg` exports `g()` whose ORDINARY BODY calls `basepkg::base()`, no
  default anywhere, app lists `userpkg` only — fails identically, with this letter's binary and with
  main's pre-plan binary. Probe `/tmp/p136b-tboth` — the case's exact source, app listing
  `userpkg` AND `basepkg` — builds and prints `5`. A dependency an importer does not list is not
  merged, so the precedent chain test `rt_top_level_initializer_globals` lists both packages; the
  case now does too. The unlocated internal error for that program shape is a pre-existing defect
  no bug or plan records (`grep -rln "NIR call target .* does not resolve" bugs planning` → nothing);
  it is captured as a bug after this phase lands, not fixed here.

## Summary

The risk is the untrusted-input validation of the new record and the importer fill on every
imported call. Untouched: every existing package byte, all version constants, and the registry.
