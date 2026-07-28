# bug-326: repo-wide dead-code sweep — dead items, lying `#[allow]` attributes, and blanket file-level suppressions

Last updated: 2026-07-18
Effort: medium (1h–2h)
Severity: LOW
Class: Dead-code

Status: Fixed (2026-07-19)
Regression Test: `cargo check --all-targets` clean + full acceptance suite.
**Removing the blanket allows is itself the regression guard** — with all eight
file-level `#![allow(dead_code)]` attributes gone and the build warning-free, a
re-introduced dead item is reported immediately rather than sitting unseen. The
policy is now written down in `AGENTS.md` so the next occurrence is caught in
review (C4).

A cluster of dead code found across the whole tree during the cleanup review.
None of it affects compiled program behavior; the reason to fix it is that the
suppressions actively **hide** the class. Today `cargo check` reports six
warnings in the `mfb` binary, while eight file-level `#![allow(dead_code)]`
attributes cover 2,634 lines and roughly two dozen item-level allows sit on
individual items — several of which are **lies**: the item they suppress is in
fact heavily used, so the attribute buys nothing and misinforms the next reader.

The single correct outcome a fix produces: every genuinely dead item is deleted,
every stale or false `#[allow]` is removed, every blanket file-level allow is
replaced by targeted allows on the specific items that need them, and every item
that must be *kept* despite having no reader (layout anchors, spec-anchored
constants, test-only integrity guards) is re-documented to say honestly why it
exists. After that, `cargo check` is clean and stays clean.

Two items are dangerous to get wrong and are called out in group (D):
**deleting a layout anchor silently changes a struct offset or erases a
documented arena map**, and deleting a spec-anchored constant breaks a
`[[path:symbol]]` citation in `src/docs/spec/`.

References:

- Found during the cleanup-focused source review (consolidated findings index,
  agents 02/03/05/06/07/08/09/10/11/12/14/15/17/18/21/22).
- `bugs/bug-300-docs-deadcode-low-cluster.md` — the prior low-cluster document;
  this one follows its itemized shape. bug-300-E5 already owns
  `x86_64/regmodel.rs:ZERO_REGISTER`, so that lead is **not** repeated here.
- `src/docs/spec/architecture/06_native.md:327-337` (regalloc strategy prose),
  `src/docs/spec/unicode/01_tables-and-algorithms.md:57,177` (spec anchors into
  otherwise-unread constants).

## Current State

`cargo check --all-targets` today, on a clean tree at `25c38ba1`:

```
cargo check --all-targets
```

Observed — 6 warnings in the `mfb` binary, 2 in integration tests:

| Warning | Site |
| --- | --- |
| fields `slot` and `direction` are never read | `src/ir/link.rs:214-215` |
| constant `RUNTIME_X86_64_LEN` is never used | `src/os/linux/appimage/mod.rs:40` |
| constant `RUNTIME_AARCH64_LEN` is never used | `src/os/linux/appimage/mod.rs:41` |
| associated function `dir` is never used | `src/os/linux/squashfs.rs:136` |
| method `check_link_function` is never used | `src/syntaxcheck/mod.rs:688` |
| unused import `std::path::PathBuf` | `tests/gtk_term_utf8_grid.rs:20` |
| unused import `Path` | `tests/tls_listen_accept_build.rs:25` |

That is what the compiler is *allowed* to tell us. What the suppressions hide,
counted in this review:

| Suppression | Count | Lines covered | Genuinely dead underneath |
| --- | --- | --- | --- |
| `#![allow(dead_code)]` file-level | 8 files | 2,634 | 20 items (1 + 18 + 1) |
| `#[allow(dead_code)]` in `abi.rs` | 14 | — | 4 items; 7 are lies; 3 are macro-wide |
| `abi.rs` macro-level allows | 3 macros | 46 generated builders | 10 unused, 36 used |
| item-level allows elsewhere | ~10 | — | 4 dead, 3 lies, 3 anchors |

The blanket allows are strikingly imprecise. `src/target/shared/regmodel.rs:10`
covers 110 lines and hides **nothing** — every item in the file is referenced.
Meanwhile `src/target/shared/code/simd_kernel_coeffs.rs:1` hides exactly one
item, `ATAN_COEFFS`, and its own comment ("most coeff sets are consumed as later
Phase-5 kernels land") is the reason nobody was ever told.

Contrast case that works correctly today: the tree has **zero** `unused_variable`
or `unreachable_code` warnings, and plan/bug citation hygiene is clean. The
defect is specific to dead *items* and the attributes covering them.

## Root Cause

Three distinct mechanisms, which is why this is a sweep rather than one fix:

1. **Completed migrations left their scaffolding behind.** plan-00-G moved the
   explicit-carry ops to MIR but left `abi::sub_borrow`'s builder function with
   no caller; plan-20 completed at phase D but `src/ir/tests.rs:282` still cites
   phases "plan-20-E..I" that never existed; plan-35 moved the terminal escape
   sequences into `term_grid.rs` but left 13 data objects behind in `term.rs`;
   plan-53 landed `BIND STATE` but never wired up `BindState.resource_slot`.

2. **`#[allow(dead_code)]` was used as a promissory note.** A dozen attributes
   carry comments of the form "consumed by later phases" / "used by plan-15
   Phase 3". Those phases either landed by another route or were dropped. Because
   the attribute suppresses the diagnostic permanently, nothing ever revisited
   the promise. Worse, three of these attributes are now simply **false** — the
   item *is* used, so the allow describes a state of the world that no longer
   holds in the opposite direction.

3. **Blanket file-level allows were applied to solve a one-item problem.** Seven
   of the eight `#![allow(dead_code)]` files are collateral: the author had one
   awkward item and reached for the file-level hammer. `src/testutil.rs:11` is
   the one defensible blanket (a test-support module legitimately exports
   helpers ahead of their consumers) — and even it hides a genuinely dead
   `EMPTY_MAIN`.

## Items

### Group A — genuinely dead, delete

#### A1 — `runtime/strings_specs.rs`: 189 dead lines, and its own justification is false
- `src/target/shared/runtime/strings_specs.rs:1-189`;
  `src/target/shared/runtime/mod.rs:87-91` and `:106-107`;
  `src/target/shared/runtime/catalog.rs:115-118`.
- Zero `STRINGS_*_SPEC` references exist outside the file (`grep -rn
  'STRINGS_[A-Z_]*_SPEC' src/` → 0 hits outside it; the only external mention is
  the comment at `catalog.rs:117`). `strings::` ops are all native-direct, so no
  `_mfb_rt_strings_*` helper is ever emitted (bug-120.1). The module is kept
  alive solely by `#[allow(dead_code)]` at `mod.rs:90` plus
  `#[allow(unused_imports)]` at `mod.rs:106`.
- The retention justification is **wrong**: both comments say the module stays
  "to avoid a wide `RuntimeHelper::Strings` enum-variant churn", but that variant
  is constructed *only* inside the dead file (14 sites, all in
  `strings_specs.rs`). The sole other reference is the display arm at
  `runtime/mod.rs:32`. Deleting the module costs exactly one enum variant and
  one match arm — not a "wide churn".
- Fix: delete `strings_specs.rs`, the `RuntimeHelper::Strings` variant, the
  `mod.rs:32` arm, both allows, and the stale `catalog.rs:115-118` comment.
  Net ≈ −200 lines.

#### A2 — four genuinely dead items in `abi.rs`
- `src/target/shared/abi.rs:19` `RUNTIME_HELPER_CLOBBERS`, `:170` `SYSRET`,
  `:536` `sub_borrow`, `:1186` `float_negate_multiply_add_d`.
- `RUNTIME_HELPER_CLOBBERS` — 2 tree-wide refs: its definition and the comment at
  `:10`. Its own doc already concedes "nothing consumes this constant yet
  (bug-120)"; the register allocator models call clobbers independently in
  `regalloc/analysis.rs`.
- `SYSRET` — 2 refs: definition and the `:136` comment.
- `sub_borrow` — **precision matters here.** The *op* `"sub_borrow"` is very much
  alive (encoders at `src/arch/aarch64/encode/emitter.rs:94`,
  `src/arch/riscv64/encode/emitter.rs:154`,
  `src/arch/x86_64/encode/emitter.rs:555`, plus specs and tests). What is dead is
  the **builder function** `abi::sub_borrow`, whose only caller is its own test
  at `abi.rs:1292`. It is asymmetric with `add_carry`, which is still called.
  Delete the builder, keep the op.
- `float_negate_multiply_add_d` — 1 ref (its definition); its doc already says
  "has no caller — kept for completeness".
- Fix: delete all four (the `sub_borrow` builder only) and their allows.

#### A3 — 10 of 46 macro-generated vector builders in `abi.rs` are unused
- `src/target/shared/abi.rs:1041`, `:1050`, `:1059` — three macro-level
  `#[allow(dead_code)]` attributes inside `vector_three_same!`,
  `vector_two_misc!`, `vector_shift_imm!`.
- The three macros are invoked 46 times. Verified individually: **36 are used**
  (`vector_orr` 51 refs, `vector_fadd` 31, `vector_eor` 25, …) and **10 are
  unused**: `vector_fcmge_zero`, `vector_fcmgt_zero`, `vector_fcvtas`,
  `vector_frinta`, `vector_frintn`, `vector_frintp`, `vector_frintz`,
  `vector_neg`, `vector_sshl`, `vector_ushl`.
- Because the allow sits *inside the macro body*, it blankets all 46 expansions
  to cover 10 stragglers — the same over-reach as a file-level blanket.
- Fix: delete the 10 unused invocations and the three macro-level allows. If any
  must be retained as an ISA-completeness set, say so explicitly and move them to
  group D.

#### A4 — 5 `RegisterModel` trait methods dead across all three arch impls
- Declarations: `src/target/shared/regmodel.rs:31` `class_of`, `:38`
  `caller_saved`, `:47` `emit_move`, `:77` `closure_env`, `:87`
  `current_thread`. Implementations in `src/arch/aarch64/regmodel.rs:104,131,165,183`,
  `src/arch/x86_64/regmodel.rs:97,134,151`, `src/arch/riscv64/regmodel.rs:102,116,152,175`.
- Every caller is a unit test in the same file. `caller_saved` is dead
  *because* the allocator hand-rolls per-ISA masks at
  `src/target/shared/code/regalloc/analysis.rs:90-107` — the ISA fact is stated
  twice and the authoritative-looking copy is the unused one.
- **Caveat, do not delete blindly:** `closure_env` is spec-anchored at
  `src/docs/spec/memory/09_closures.md:83` (`[[src/target/shared/regmodel.rs:closure_env]]`).
  Deleting it breaks that anchor. Either keep `closure_env` and re-document it
  under group D, or delete it *and* update `09_closures.md:75-83`.
- Fix: delete 4 declarations + their 12 impls + the tests that only exercise
  them; resolve `closure_env` per the caveat.

#### A5 — rv64 `FT0` and its `const _` life-support
- `src/arch/riscv64/encode/emitter.rs:40` `const FT0: u8 = 0;` and `:739`
  `const _: (u8, u8, u8) = (T1, T2, FT0);` with the comment "Scratch registers
  reserved for later phases (referenced to keep them named)."
- Those phases are complete. `T1` (32 refs) and `T2` (25 refs) in the same file
  are heavily used and need no artificial reference, so the `const _` exists
  solely to keep `FT0` alive. Note the identically-named `FT0` in
  `src/arch/riscv64/v128.rs:44` is a *different* constant (`&str`, ~30 uses) and
  must not be touched.
- Fix: delete `emitter.rs:40` and `:739`.

#### A6 — `ATAN_COEFFS`
- `src/target/shared/code/simd_kernel_coeffs.rs:81`; only its definition exists
  tree-wide. `atan` uses the fdlibm `ATAN_AT` table instead.
- This is the sole item hidden by that file's blanket allow (`:1`), whose comment
  — "most coeff sets are consumed as later Phase-5 kernels land" — is precisely
  the sentence that guaranteed nobody would be told.
- Fix: delete `ATAN_COEFFS` and the file-level allow (see C1).

#### A7 — `UNICODE_PROPERTY_FLAG_COMB_IS_SECOND`
- `src/target/shared/code/private/unicode.rs:20`; definition only.
- Unlike its 16 neighbours (group D3) this one is not part of a contiguous
  numbering set — it is a lone flag value.
- Fix: delete.

#### A8 — `StructSlotView.slot` / `.direction`, and a const-`true` gate
- `src/ir/link.rs:214-215` (both fields; **this is one of the six warnings
  `cargo check` already reports**), `:210` `CSTRING_STRUCT_FIELDS`, `:231`/`:247`.
- Neither `view.slot` nor `view.direction` is read anywhere (0 hits for
  `view.slot`/`view.direction`). Separately, `CSTRING_STRUCT_FIELDS` is
  `const … = true` and is the *only* value ever passed as `allow_cstring` — both
  call sites (`src/ir/verify/mod.rs:2833`, `src/syntaxcheck/mod.rs:486`) pass it
  — so the `ctype == "CString" && !allow_cstring` branch at `:247` is
  unreachable. plan-50-E→F scaffolding.
- Fix: delete both fields; drop `CSTRING_STRUCT_FIELDS`, the `allow_cstring`
  parameter, and the dead branch.

#### A9 — dead `display` local kept alive by a `let _ =` 245 lines later
- `src/ir/verify/mod.rs:2966` (`let display = format!("{}::{}", function.alias,
  function.name);`) and `:3211` (`let _ = display;`).
- Exactly two occurrences of the binding in the file. It is computed once per
  link function and read by none of the ~20 diagnostics emitted in between —
  each formats `function.name` directly — then discarded 245 lines later.
- Fix: delete both lines.

#### A10 — `BindState.resource_slot`: mandatory surface syntax that is never used
- `src/ast/types.rs:411-412` (field), parsed at `src/ast/items.rs:940-961`,
  and the consumer at `src/ir/lower.rs:394` reads **only** `b.struct_slot`.
- `BIND STATE <resource_slot> = <struct_slot>` requires the user to write
  `resource_slot`, but it is never validated and never used: a user can name a
  nonexistent slot and nothing complains.
- Fix: this one is a **judgement call, not a pure deletion** — either validate
  `resource_slot` against the declared return resource (preferred; it is
  mandatory syntax and should mean something), or make it optional in the
  grammar. Do not silently delete the field while leaving the syntax mandatory.

#### A11 — `resource.rs::info()` dead
- `src/builtins/resource.rs:80-82` (`#[allow(dead_code)]` at `:80`).
- Zero callers. Its doc calls it "the registry's primary API" while
  `is_sendable` — the one method in the impl with *no* allow — is the one
  actually used. (Its sibling allows are group B; see B3.)
- Fix: delete `info()`.

#### A12 — `RegallocKind::name()`
- `src/target/shared/code/regalloc/mod.rs:73-81` (allow at `:74`).
- No caller. `available_strategies()` at `:85` independently supplies the same
  two strings for the error message. The `.name()` hits in `src/cli/build.rs`
  are `target.name()`, a different type.
- Fix: delete the impl block.

#### A13 — `testutil::EMPTY_MAIN`
- `src/testutil.rs:115`; definition only. All 8 other helpers in the file are
  heavily used, so the blanket allow at `:11` buys nothing for them and hides
  only this.
- Fix: delete.

#### A14 — manifest `role` and `targets` are written but never read
- Written by `src/cli/init.rs:79` (`"role": "main"`) and `:84`
  (`"targets": ["native"]`); documented as consumed at
  `src/docs/spec/tooling/01_project-manifest.md:42,74-77,100,109-110`.
- No reader exists. The manifest key readers are enumerated at
  `src/manifest/mod.rs:204,315,364,406,566,853,863,883,909,921,931,939,947`;
  `sources` is read at `:204` but the per-source `role` key is not, and
  `targets` is never fetched. (`src/manifest/libraries.rs:255`
  `registered_targets()` is unrelated; so is `src/cli/repo.rs:184`.)
- Every scaffolded project therefore ships two inert keys the docs call
  meaningful.
- Fix: either read them or stop emitting them — and correct
  `01_project-manifest.md` either way.

#### A15 — `PROJECT_JSON_VALID` is defined and specified but never emitted
- `src/rules/table.rs:60`; the only other Rust references,
  `src/rules/mod.rs:186` and `:291`, are both inside the `#[cfg(test)]` module
  opening at `src/rules/mod.rs:147`.
- Specified as a live info rule at
  `src/docs/spec/diagnostics/01_rule-codes.md:269` and
  `src/docs/spec/tooling/01_project-manifest.md:278`, so the spec promises a
  diagnostic the compiler never produces.
- Fix: emit it on successful manifest validation, or delete the rule and both
  spec rows.

#### A16 — three audit category ranks emit no findings
- `src/audit/report.rs:182` (`"sourceFlow" => 3`), `:184` (`"native" => 5`),
  and — missed by the original lead — `:187` (`"policy" => 8`).
- The categories actually emitted from `src/audit/collect/` are dependency,
  lint, lockfile, package, permission, resource. All three unreachable ranks are
  pinned by tests at `src/audit/report.rs:469,471,474`, which is why they never
  surfaced. Note `sourceFlow` *is* a live report-section name
  (`src/audit/json.rs:160`) — just never a finding category.
- Fix: delete the three arms and their test assertions, or document them as a
  forward-compatible ordering.

#### A17 — `ResourceEntry.native` is hardcoded `false` everywhere
- Production construction at `src/audit/collect/source.rs:140` and `:158`, both
  literal `native: false`. The only `native: true` is
  `src/audit/report.rs:309`, inside `#[cfg(test)] pub(super) mod testsupport`.
- The rendering branch at `src/audit/text.rs:114-118` and the field at
  `src/audit/json.rs:275` are therefore dead in every real run.
- Fix: populate `native` from the LINK table, or drop the field and its
  rendering branch.

#### A18 — `AllocInput.instructions` and `.model` are written but never read
- `src/target/shared/code/regalloc/mod.rs:122` and `:130` (allows at `:121`,
  `:129`); populated at `:244-246`.
- The only `AllocInput` field ever read is `input.eager` at `:159`.
- **Scope note:** the original lead claimed the whole `AllocationStrategy`
  abstraction was dead. It is not — see "Leads that did not hold". Only these
  two fields are dead.
- Fix: delete both fields and their allows; stop populating them.

#### A19 — three unreferenced scripts, one destructive if re-run
- `scripts/gen_vector_tests.py`, `scripts/audit.sh`, `scripts/fix_citations.py`.
- Tree-wide search returns two hits total, both self-references in
  `scripts/fix_citations.py:28-29` (its own usage docstring). Zero references to
  `gen_vector_tests` or `audit.sh` anywhere: no CI (`.github/workflows/` holds
  only `coverage.yml`), no Makefile (none exists), no docs, no sibling script.
- `gen_vector_tests.py:2-11` self-declares: *"LEGACY generator … kept for
  historical reference only; do not re-run it without re-bucketing its output,
  or it will recreate stale fixtures at the repo-root-relative
  tests/func_vector_* paths."* The fixtures it writes were relocated by the
  tests reorg into `tests/syntax|rt-error|rt-behavior/vector/*`, so re-running it
  **creates stale duplicates at obsolete paths**.
- Fix: delete `gen_vector_tests.py` (highest value — it is a live footgun);
  delete or wire up the other two.

#### A20 — `#[ignore]`d census test citing plan phases that never existed
- `src/ir/tests.rs:282` — `#[ignore = "porting census (plan-20-E..I); run with
  --ignored --nocapture"]` on `verify_vs_syntaxcheck_diagnostic_parity`, with the
  same citation repeated in the doc comment at `:275`.
- This is the only `#[ignore]` in the tree. `planning/` contains exactly
  `plan-20-typed-ir-single-checker.md` and phases **A, B, C, D** — phases E
  through I do not exist in `planning/` or `planning/old-plans/`. plan-20 is
  complete (65 rules relocated to `ir::verify`), so the census has served its
  purpose.
- Fix: delete the test, or keep it as a standing parity check with an honest
  citation and no invented phases.

#### A21 — `term.rs` ships 13 escape-sequence data objects no emitted code references
- `src/target/shared/code/term.rs:32-47` (15 byte-string consts), `:49-64` (15
  matching `*_SYMBOL` names), `:70-88` `esc_entries()`, and `console_data_objects()`
  at `:91+` which emits a `CodeDataObject` for every one.
- Emitted code references exactly two: `ESC_ON_SYMBOL` at `:391` and
  `ESC_OFF_SYMBOL` at `:512`. The other **13** — `ESC_CLEAR`, `ESC_BOLD_ON/OFF`,
  `ESC_UNDERLINE_ON/OFF`, `ESC_SHOW_CURSOR`, `ESC_HIDE_CURSOR`, `ESC_FG_PREFIX`,
  `ESC_BG_PREFIX`, `ESC_SEMICOLON`, `ESC_LETTER_M`, `ESC_BRACKET`,
  `ESC_LETTER_H` — appear only in the const block and the `esc_entries()` table.
- These are pre-plan-35 leftovers (plan-35-C moved them to `append_const` in
  `term_grid.rs:1005-1071`). Unlike every other item here they have a *shipped*
  cost: the bytes land in every binary that uses `term::`.
- Fix: delete the 13 consts, their `*_SYMBOL` names, and their `esc_entries()`
  rows. Golden resync required (emitted data changes).

#### A22 — `alsa.rs`: dead `_input` param and a branch to the next instruction
- `src/target/shared/code/audio/alsa.rs:704` — `_input: bool` in
  `emit_configure_hw_params` (declared `:700`), unreferenced in the body. Note
  the `#[allow(clippy::too_many_arguments)]` at `:699` sits above an 8-param
  function one of whose params is dead.
- `src/target/shared/code/audio/alsa.rs:1671-1672` — `abi::branch(&clamp)`
  immediately followed by `abi::label(&clamp)`, an unconditional branch to the
  very next instruction, in the `Query::Xruns` arm.
- Fix: drop the param; delete the branch. Golden resync for the branch (emitted
  bytes change).

#### A23 — dead `stderr` parameter in `lower_io_flush_helper`
- `src/target/shared/code/io_helpers.rs:522` (`stderr: bool`), sole caller
  `src/target/shared/code/mod.rs:1553` passing a hardcoded `false`.
- With no `stderr == true` caller, the entire `if !stderr` / `if stderr`
  distinction inside the helper is dead, including the guarded
  `abi::branch_ne(&output_error)` at `:548`.
- **Correction to the original lead:** it claimed the unreachable error tail
  pushes "2 spurious `ERR_OUTPUT_SYMBOL` data relocations into every
  stderr-flushing binary". That does not hold — no stderr-flushing binary
  exists, because no caller passes `true`. The real defect is the dead parameter.
- Fix: delete the `stderr` parameter and collapse the branches.

#### A24 — inert `alloc_ok`-style labels in `os.rs` and two dead labels elsewhere
- `src/target/shared/code/os.rs` — 8 `alloc_ok` binding/emission pairs with
  **zero** branches: `:234`/`:252`, `:287`/`:317`, `:425`/`:443`, `:783`/`:886`,
  `:1036`/`:1053`, `:1090`/`:1110`, `:1769`/`:1773`, `:1935`/`:2016`. Each is the
  fallthrough side of a `branch_ne(alloc_fail)`. (The lead said ~10; verified
  count is 8.)
- `src/target/shared/code/fs_helpers_paths.rs:24`/`:115` — `missing` is bound
  and emitted, never branched to. (The `missing` branch at `:249` belongs to a
  different helper starting at `:151` with its own binding at `:155`.) Note
  `alloc_ok` at `:22` *is* branched at `:48` and is not part of this finding.
- `src/target/shared/code/net/io.rs:369`/`:474` — `build_list` bound and
  emitted, never branched to.
- Fix: delete the inert labels. Golden resync if label emission affects output.

#### A25 — `crypto_ec/openssl.rs` redefines the `emit_alloc` its parent provides
- `src/target/shared/code/crypto_ec/openssl.rs:376-394` vs
  `src/target/shared/code/crypto_ec.rs:241-259`.
- Semantically identical — `branch_link(ARENA_ALLOC_SYMBOL)`, one
  `CodeRelocation { kind: RelocIntent::Call, binding: "internal", library: None }`,
  then `compare_immediate(return_register(), RESULT_OK_TAG)` + `branch_ne(fail)`.
  Only local parameter names differ (`ins`/`rel` vs `instructions`/`relocations`);
  emitted output is byte-identical. `openssl` is a child module of `crypto_ec`,
  so the parent's private fn is already in scope.
- Fix: delete the child copy; its 6 call sites (`:710,753,776,1042,1203,1466`)
  resolve to the parent unchanged.

#### A26 — `tls/macos.rs::dlsym` is a zero-value 9-param pass-through
- `src/target/shared/code/tls/macos.rs:209-232` — `#[allow(clippy::too_many_arguments)]`
  at `:209` over a 9-parameter `fn dlsym` whose entire body forwards all 9
  arguments unchanged to `emit_dlsym` at `:221-231`. Zero transformation; the
  allow exists purely to license the indirection.
- Not callerless (7 sites: `:251,316,336,372,502,523,560`), so this is redundant
  indirection rather than unreachable code.
- Fix: call `emit_dlsym` directly at the 7 sites; delete the wrapper and its allow.

#### A27 — `__collections_reverse` is dead and a comment claims it is called
- `src/builtins/collections_package.mfb:77` (definition) and `:9-10` (comment).
- Exactly two occurrences tree-wide; zero call sites. The comment — *"The
  internal `__collections_slice`/`__collections_reverse` helpers are plain
  top-level functions and are called unqualified"* — is true of `__collections_slice`
  and false of `__collections_reverse`. It is also absent from `FUNCTIONS`,
  `NATIVE_MEMBERS`, and the man pages.
- Fix: delete the function; correct the comment to name only `__collections_slice`.

#### A28 — 8 unreachable alias arms in the x86_64 encoder dispatch
- `src/arch/x86_64/encode/emitter.rs:708` (`"fmov_i2f"`), `:714` (`"fmov_f2i"`),
  `:811` (`"i2f"`), `:817` (`"f2i_trunc"`), `:824` (`"f2i_floor"` and
  `"f2i_ceil"`), `:848` (`"f2i_nearest"`), `:866` (`"rotr_w"`).
- `:293` binds `let m = instruction.op.mnemonic();` where `op` is a `CodeOp`
  (`src/target/shared/code/types.rs:39`). `CodeOp::mnemonic()` returns the
  **AArch64** spelling (`src/arch/ops.rs:254,314,327,328`, …). The neutral names
  exist only at the MIR layer (`src/target/shared/code/mir.rs:165-176`) and
  `MirOp::to_code()` (`mir.rs:101-109`) renames back to `CodeOp` before
  selection — so the neutral strings can never reach this dispatch. In each arm
  the surviving alternative is the AArch64 name.
- Consequently the `m.starts_with("f2i_floor")` disjunct at `:827` is also
  unreachable; only the `fcvtms` half of that condition can fire.
- Fix: drop the 8 dead alternatives and the dead disjunct. Better, per the
  original finding: match on `CodeOp` rather than the mnemonic string so the
  compiler enforces exhaustiveness and this class cannot recur.

#### A29 — three items `cargo check` already reports
- `src/syntaxcheck/mod.rs:688` `check_link_function` — genuinely dead; it is a
  thin wrapper calling `check_link_function_in(file, function, &[])` at `:693`,
  while the live path calls `check_link_function_in` directly at `:408`. The
  doc comment of the reimplementation at `src/ir/verify/mod.rs:2936` still
  *names* this dead symbol.
- `src/os/linux/squashfs.rs:136` `SquashNode::dir` — used only at
  `src/os/linux/squashfs/tests.rs:790`; dead in the binary build.
- `tests/gtk_term_utf8_grid.rs:20` and `tests/tls_listen_accept_build.rs:25` —
  unused imports.
- Fix: delete `check_link_function` and fix the `verify/mod.rs:2936` comment;
  gate `SquashNode::dir` behind `#[cfg(test)]`; drop the two imports.

### Group B — stale or false `#[allow]` attributes to remove

Removing these is what makes the compiler resume reporting the class. Three of
them are outright lies: the suppressed item **is** used.

#### B1 — 7 `#[allow(dead_code)]` in `abi.rs` on heavily-used functions (LIES)
- `src/target/shared/abi.rs:787` `load_u32`, `:795` `load_u16`, `:827`
  `store_u16`, `:1004` `vector_load`, `:1013` `vector_store`, `:1119`
  `vector_dup_from_x`, `:1127` `vector_extract_to_x`.
- Tree-wide reference counts: `load_u32` **76**, `load_u16` **18**,
  `vector_dup_from_x` **17**, `vector_extract_to_x` **17**, `vector_load` **14**,
  `vector_store` **10**, `store_u16` **3**. Every one of these is used; the
  attribute is false in all seven cases.
- Fix: delete the seven attributes. No code change.

#### B2 — `AbiSpec.line`: the allow claims the field is unread; it is read
- `src/ast/types.rs:371-373` — `/// Source line of the ABI clause; retained for
  diagnostics.` followed by `#[allow(dead_code)] pub line: usize,`.
- **The original lead claimed zero reads. That is wrong.** The field is read at
  `src/syntaxcheck/mod.rs:762` and `:875`, both passing `function.abi.line` as
  the diagnostic line for `NATIVE_*` rules. The doc comment is accurate; the
  attribute contradicts it.
- This is the only `#[allow(dead_code)]` in all of `src/ast/`, and every sibling
  (`AbiSlot` `:384`, `BindIn` `:397`, `ConstPin` `:424`) has a live `line` with
  no allow.
- Fix: delete the attribute only.

#### B3 — `resource.rs`: `close_function()`'s allow is a lie
- `src/builtins/resource.rs:87` `#[allow(dead_code)]` over `close_function()`.
- The method **is** used, at `src/syntaxcheck/types.rs:384` and `:385`
  (`self.resource_registry.close_function(base) == Some(callee)` and the
  `name.as_str()` variant), plus five test sites.
- Its sibling allows: `:80` `info()` is genuinely dead (→ A11); `:34`
  (struct-level, for `close_may_fail`/`kind`) and `:103` (`close_may_fail`)
  need re-checking on the same pass — the block comment at `:30-33` makes the
  same "consumed by later overhaul phases" promise this bug is retiring.
- Fix: delete the `:87` attribute; re-verify `:34` and `:103` and delete or
  re-document per the outcome.

#### B4 — stale `#[allow(unused_imports)]` in `audio/mod.rs`
- `src/target/shared/code/audio/mod.rs:80`, over `:81`'s
  `pub(super) use super::tls::{emit_alloc, emit_arena_free, emit_data_address, emit_fail};`.
- All four imports are used: `emit_alloc` (`audio/macos.rs:240,759,1576,1716,2688`),
  `emit_arena_free` (`audio/macos.rs:2266`, `audio/alsa.rs:1575`),
  `emit_data_address` (`audio/macos.rs:337,1807`, `audio/alsa.rs:198,236`),
  `emit_fail` (`audio/macos.rs:442,458,467,1019,1028`, and more).
- Fix: delete the attribute.

#### B5 — spec overstates the regalloc strategy abstraction
- `src/docs/spec/architecture/06_native.md:327-337` vs
  `src/target/shared/code/regalloc/mod.rs:255+`.
- The spec says "The allocation method is a swappable `AllocationStrategy`,
  selected by the `--regalloc <name>` build flag." Half true: `BumpAndReset`
  does go through the trait (`mod.rs:156` impl, `:243` call), but the **default**
  `linear-scan` path bypasses the trait entirely. Related: `regalloc/mod.rs:11-15`
  claims only one strategy ships and misnames the flag (it is `--regalloc`
  post-plan-42).
- Fix: qualify the spec sentence — one of the two selectable strategies uses the
  trait, the default does not — and correct the module doc.

### Group C — blanket file-level allows to replace with targeted ones

Eight files carry `#![allow(dead_code)]`, covering **2,634 lines** and hiding
20 genuinely dead items between them. The policy problem is that the scope of
each attribute vastly exceeds the problem it was added for.

| File | Line | LOC | Dead items underneath |
| --- | --- | --- | --- |
| `src/target/shared/code/private/unicode.rs` | 1 | 983 | 18 (1 dead + 17 anchors) |
| `src/unicode_runtime_tables.rs` | 1 | 523 | 1 (+1 unreachable fn) |
| `src/arch/x86_64/regmodel.rs` | 12 | 275 | trait impls (A4) |
| `src/arch/aarch64/regmodel.rs` | 15 | 272 | trait impls (A4) |
| `src/arch/riscv64/regmodel.rs` | 16 | 255 | trait impls (A4) |
| `src/testutil.rs` | 11 | 115 | 1 (`EMPTY_MAIN`) |
| `src/target/shared/regmodel.rs` | 10 | 110 | **0** |
| `src/target/shared/code/simd_kernel_coeffs.rs` | 1 | 101 | 1 (`ATAN_COEFFS`) |

#### C1 — delete the four blankets that hide 0–1 items
- `src/target/shared/regmodel.rs:10` hides **nothing** — every item in the file
  is referenced. Delete outright.
- `src/target/shared/code/simd_kernel_coeffs.rs:1` hides only `ATAN_COEFFS`
  (A6). Delete both.
- `src/testutil.rs:11` hides only `EMPTY_MAIN` (A13). This is the one
  *defensible* blanket — a test-support module may legitimately export helpers
  ahead of their consumers — so if it is kept, replace the bare attribute with a
  comment stating that rationale explicitly.
- `src/unicode_runtime_tables.rs:1` — see D4/D5.

#### C2 — the three arch `regmodel.rs` blankets
- `src/arch/aarch64/regmodel.rs:15`, `src/arch/x86_64/regmodel.rs:12`,
  `src/arch/riscv64/regmodel.rs:16`.
- These are what hide A4's 15 trait implementations. Once A4 lands, re-check
  each file: `src/arch/aarch64/regmodel.rs:193` `is_fp_callee_saved` is
  `pub(crate)` but file-local and should be narrowed at the same time.
- Fix: delete all three; add targeted allows only where a remaining item
  genuinely needs one.

#### C3 — `private/unicode.rs:1`
- Covers 983 lines to hide one genuinely dead constant (A7) and 17 anchors
  (D3). After A7 and D3 land, replace with per-item allows on the anchor block.

#### C4 — adopt the policy
- Blanket `#![allow(dead_code)]` should not be used to suppress a small, known
  set of items. Record the rule in `CLAUDE.md` alongside the existing
  `clippy::items_after_test_module` guidance, so the next occurrence is caught in
  review rather than in the next sweep.

### Group D — must be KEPT, but re-documented

**Read this group before deleting anything above.** These items have no reader
and will be reported the instant the blanket allows come off — but deleting them
causes real harm: a struct offset silently shifts, a documented arena map gains
a hole, or a `[[path:symbol]]` spec anchor breaks. Each needs a targeted allow
plus an honest comment saying *why* it exists, not a promise about a future
phase.

#### D1 — `STDIN_LOG_*` reserved slots: layout anchors with a false comment
- `src/target/shared/code/error_constants.rs:492` `STDIN_LOG_MUTEX_OFFSET = 0`,
  `:513` `STDIN_LOG_SELFPIPE_READ_OFFSET = 192`, `:515`
  `STDIN_LOG_SELFPIPE_WRITE_OFFSET = 200`. Attributes at `:491`, `:512`, `:514`.
- Zero readers in `src/`, `tests/`, `scripts/`. **They are anchors**: they belong
  to a contiguous documented arena map (0, 64, 128, 136, 144, 152, 160, 168,
  176, 184, 192, 200, 208). The sibling `STDIN_LOG_CV_OFFSET = 64` at `:493` is
  read nine times from `stdin_broadcast.rs`, and the mutex at offset 0 is
  addressed implicitly (`stdin_broadcast.rs:190-191` passes the bare log address
  as `ARG[0]`). Deleting these three renumbers nothing but erases the map.
- All three carry the identical comment
  `// used by plan-15 Phase 3 (self-pipe) / layout doc`, which asserts a use that
  does not exist. It is doubly wrong on `STDIN_LOG_MUTEX_OFFSET`, which has
  nothing to do with the self-pipe — that is the mutex slot; the self-pipe fds
  are the two at 192/200.
- Fix: **keep all three.** Replace the comment with an honest one: reserved
  layout slots completing the `_mfb_rt_stdin_log` block map; the self-pipe pair
  is unbuilt (plan-15 D4 deferred). State the deferral in the subjunctive.

#### D2 — `UNICODE_NFD_ENTRY_SIZE` is spec-anchored
- `src/target/shared/code/private/unicode.rs:16` (`= 16`), anchored from
  `src/docs/spec/unicode/01_tables-and-algorithms.md:177` via
  `[[src/target/shared/code/private/unicode.rs:UNICODE_NFD_ENTRY_SIZE]]`.
- Nothing reads it, but it is the **record stride** for the NFD table whose
  field offsets (0/4/8 + pad) sit immediately around it at `:17-19`, and the
  spec's layout table cites it by name. The runtime lookup scales its midpoint by
  this stride (`<< 4`).
- Fix: **make it load-bearing rather than deleting it.** Best option: use it in
  the binary-search stride computation so the constant and the `<< 4` cannot
  drift apart. Failing that, keep it with a targeted allow and a comment naming
  the spec anchor.

#### D3 — 16 `GRAPHEME_BOUNDCLASS_*` / `INDIC_CONJUNCT_BREAK_*` protocol mirrors
- `src/target/shared/code/private/unicode.rs:21-38`. Unread:
  `GRAPHEME_BOUNDCLASS_LF, CONTROL, EXTEND, L, V, T, LV, LVT,
  REGIONAL_INDICATOR, SPACINGMARK, PREPEND, ZWJ, EXTENDED_PICTOGRAPHIC, E_ZWG`
  and `INDIC_CONJUNCT_BREAK_LINKER, CONSONANT, EXTEND`.
- **These are anchors, not dead code.** They mirror the utf8proc boundclass
  enumeration emitted into the runtime tables; `GRAPHEME_BOUNDCLASS_CR = "2"` at
  `:21` *is* used, so the set is partially live and the numbering
  (2…14, 19, 20 / 1…3) must stay contiguous and correct. Deleting the unread
  members would leave a live constant with no legend.
- Precedent already exists in this file: the `bug-70` comment at `:7-10` records
  constants that *were* correctly removed, and explains why.
- Fix: keep; group them under one targeted allow with a comment naming utf8proc
  as the source of the numbering.

#### D4 — `property_for_codepoint` is spec-anchored
- `src/unicode_runtime_tables.rs:100`; called only by its own tests
  (`:495-499`), and anchored from
  `src/docs/spec/unicode/01_tables-and-algorithms.md:57`.
- Fix: keep with a targeted allow naming the spec anchor, or promote it to the
  real lookup path. Do not delete without updating `01_tables-and-algorithms.md:57`.

#### D5 — `category_value` is reachable but can never fire
- `src/unicode_runtime_tables.rs:372`, called from `:315` via
  `_ if value.starts_with("UTF8PROC_CATEGORY_")`.
- Statically reachable, so the compiler will never report it — but the guard
  can never be true, because `parse_properties` skips utf8proc field 0. A
  36-line function that cannot execute.
- Fix: this is the one item here that is *not* an anchor. Either delete it and
  the unreachable guard, or — if field 0 is meant to be parsed — fix the parser.
  Decide deliberately; do not simply delete if the category data is wanted.

#### D6 — `MODE_LINE_NOECHO` completes a documented enumeration
- `src/target/linux_gtk/mod.rs:171` (`= "0"`). Its siblings `MODE_LINE_ECHO`
  (`:172`, 6 refs) and `MODE_RAW` (`:173`, 5 refs) are used; this one has only
  its definition.
- It is the **zero-init default**, which is exactly why no code writes it. The
  spec documents it as such at `src/docs/spec/app/03_console-io.md:129`
  ("(default; never set explicitly)") and
  `src/docs/spec/app/02_linux-runtime.md:220`.
- Fix: keep; add a targeted allow with a comment stating it is the zero-init
  default and is never assigned.

#### D7 — `RUNTIME_X86_64_LEN` / `RUNTIME_AARCH64_LEN` are integrity guards
- `src/os/linux/appimage/mod.rs:40-41`. **Both are among the six warnings
  `cargo check` reports today**, so they are the most likely items to be deleted
  by a careless sweep.
- They are used at `:263-264`, inside the `#[cfg(test)] mod tests` opening at
  `:256`, by `every_runtime_ends_exactly_at_its_own_length` — the test that
  catches a stale or truncated embedded AppImage runtime blob. Their own doc
  says so: *"Recorded blob lengths. A stale or truncated copy fails a unit test
  rather than shipping."*
- Fix: **keep.** Gate them with `#[cfg(test)]` so the warning disappears without
  losing the guard. Do not delete.

#### D8 — `closure_env` (cross-reference)
- See A4. `src/target/shared/regmodel.rs:77` is spec-anchored at
  `src/docs/spec/memory/09_closures.md:83`. If A4 deletes it, `09_closures.md:75-83`
  must be updated in the same commit.

## Outcome (2026-07-19)

`cargo check --all-targets` reports **zero warnings**, and the tree has **zero**
file-level `#![allow(dead_code)]`. `cargo test` 3096 passed / 0 failed;
`scripts/artifact-gate.sh` 1189 goldens, 0 diffs; macOS acceptance 1014/1014.

Several of this document's claims did not survive contact with the code. They are
recorded here rather than quietly worked around, because the pattern — a finding
written from a grep and never re-run — is the same one that produced the dead
code in the first place.

### Claims that were wrong

- **The "Current State" warning table was stale.** Five of its seven warnings
  (`ir/link.rs` slot/direction, the two AppImage `RUNTIME_*_LEN`,
  `squashfs::dir`, `check_link_function`, both unused test imports) had already
  been fixed between base `25c38ba1` and now. Two live warnings were *not* in the
  table, and one of them was a real defect the table's absence hid — see below.
- **A6 (`ATAN_COEFFS`) must not be deleted.** `simd_kernel_coeffs.rs` is
  generated by `tools/math-kernels/gen_coeffs.py`, which emits `atan` as one of
  its five primitive reduced approximations. Deleting the block would be undone
  by the next regeneration and would leave the tool and the file disagreeing.
  Kept with a targeted allow explaining that `atan` is computed from the fdlibm
  `ATAN_AT` table instead.
- **D3 is wrong.** The 16 `GRAPHEME_BOUNDCLASS_*` / `INDIC_CONJUNCT_BREAK_*`
  constants are *not* unread — with the file's blanket allow removed, the
  compiler reports none of them. Only `UNICODE_PROPERTY_FLAG_COMB_IS_SECOND`
  (A7) was dead. No targeted allow was needed for the block.
- **A25 and A26 no longer exist.** `crypto_ec/openssl.rs` has no duplicate
  `emit_alloc`, and `tls/macos.rs::dlsym` is now a 5-parameter borrow-splitting
  shim with no `too_many_arguments` allow — not the 9-parameter pass-through
  described.

### Judgement calls, and why

- **A4 — the five `RegisterModel` methods are kept, not deleted.** `closure_env`
  is spec-anchored, and it and `current_thread` are the only statement anywhere
  of which physical register each ISA pins for its role token; deleting them
  would leave `abi::realize_abi_token`'s literal table as the sole record.
  `caller_saved` is unread because `regalloc::analysis` hand-rolls the same masks
  — a real duplication, but the fix is to route the allocator through the model,
  which is a larger change than this sweep. All five carry targeted allows and
  the trait doc states the rule: a method that is neither spec-anchored nor the
  sole statement of an ISA fact gets deleted, not an allow.
- **A14 / A15 / A16 / A17 resolved as documentation, not behavior.** Each offered
  "emit it or delete it". Emitting `PROJECT_JSON_VALID` on every successful
  manifest validation would add an info line to every build (mass golden churn);
  deleting the rule would recycle `2-200-0010` for a later meaning; dropping
  `ResourceEntry.native` or the audit category ranks would change committed
  audit output. In each case the code is now honest about what it does and the
  spec rows say "reserved / not emitted" instead of describing a diagnostic
  users can never see. `ResourceEntry.native` turned out to be correct at
  `false`: `resource_producer` recognizes built-in producers only, and native
  `LINK` resources are reported through `native_resources`.
- **A19 not done here.** `bugs/bug-344-test-tooling-infrastructure.md` owns those
  three scripts in more detail and reaches a *different* conclusion (keep the
  net-timeout pair). Two documents deleting the same files by different rules is
  how a useful script gets lost; left to bug-344.
- **A24 not done.** The inert-label analysis could not be confirmed at scale —
  label variables are reused across helpers within a file, so a per-file grep
  cannot tell an unbranched label from a shared one. Labels are pseudo-ops
  resolved at assembly and emit no bytes, so the cost of leaving them is zero.
  A per-function checker would be the way to do this properly.

### Found while sweeping, not in this document

- **A test had silently not run since it was written.** `src/cli/build.rs`
  carried a doc comment and `#[test]` for `copy_resources_maps_the_worked_examples`,
  and a second test was later inserted *between* the attribute and its function.
  The attribute bound to the newcomer, and plan-55-A's three worked examples had
  no `#[test]` at all. `cargo check --all-targets` had been reporting it as
  "function is never used" — a warning the stale table above never listed.
  Restored; it passes.
- **`FIXED_ONE_MINUS_1_STR`** (`builder_simd_math.rs`) was dead, with a comment
  claiming `CeilFixed` read it "for call-site intent". `CeilFixed` reads
  `FIXED_FRACTION_MASK_STR` like everything else. Deleted.

## Goal

- Every item in groups A and B is removed; `cargo check --all-targets` is clean
  with **zero** warnings and **zero** file-level `#![allow(dead_code)]`.
- Every item in group D is retained, carries a targeted `#[allow(dead_code)]`
  (or `#[cfg(test)]` for D7), and has a comment stating the real reason it
  exists — no "consumed by a later phase" promises.
- No `[[path:symbol]]` spec anchor is left dangling: `06_native.md`,
  `01_tables-and-algorithms.md`, `09_closures.md`, `01_project-manifest.md`, and
  `01_rule-codes.md` agree with the code after the sweep.

### Non-goals (must NOT change)

- **Any compiled-program behavior.** Two items shift emitted bytes (A21's term
  escape data objects, A22/A24's dead labels and branch); those require golden
  regeneration and the delta must be *only* the removed bytes.
- **Struct offsets, arena layouts, and the `.mfp` wire format.** Group D exists
  precisely to protect these. Deleting a D-group anchor to silence a warning is
  the tempting wrong fix and is forbidden.
- The `AllocationStrategy` trait, `BumpAndReset`, and `--regalloc bump` — these
  are live (see below) and must not be deleted.
- The `sub_borrow` **op** and its three encoders — only the `abi::sub_borrow`
  builder function goes.
- Re-adding a blanket `#![allow(dead_code)]` to quiet a stubborn file. If an item
  must stay, it gets a targeted allow and a reason.

## Blast Radius

Each item is an independent site; there is no shared buggy pattern to fix once.
The couplings that matter:

- `A4 → D8 → src/docs/spec/memory/09_closures.md` — deleting `closure_env`
  breaks a spec anchor.
- `A6 → C1` — `ATAN_COEFFS` and its file blanket must go together.
- `A7 + D3 → C3` — `private/unicode.rs`'s blanket can only come off after both.
- `A13 → C1` — `EMPTY_MAIN` and the `testutil.rs` blanket decision.
- `A18 → B5` — the dead `AllocInput` fields and the spec sentence overstating
  the abstraction.
- `A21, A22, A24` — the only items requiring golden resync; land them together
  so one regeneration covers all three.
- `A1` — touches an enum variant, so `runtime/mod.rs:32` and `catalog.rs:115-118`
  move with it.

Out of scope, same hazard, deliberately not fixed here:

- `src/arch/x86_64/regmodel.rs:ZERO_REGISTER` — already owned by bug-300-E5.
- The ~84 over-wide function signatures and the `HelperBody` 4-tuple cluster —
  duplicate-code findings, not dead code.
- `src/target/shared/code/audio/alsa.rs:699` `too_many_arguments` and
  `src/target/shared/code/tls/macos.rs:209` — the *allow* is in scope (A26),
  the broader parameter-count refactor is not.

## Fix Design

Land group by group, each group a separate commit, in this order:

1. **D first.** Add the targeted allows and honest comments *before* removing any
   blanket. This guarantees that when the blankets come off in step 3, the
   anchors are already protected and nobody is tempted to delete them to make the
   build quiet.
2. **A and B.** Deletions and attribute removals; mostly mechanical, each
   independently revertable.
3. **C last.** Remove the blanket allows. This is the step that arms the guard —
   after it, the class cannot silently return.

Rejected alternatives, so they are not re-litigated:

- *Delete the `AllocationStrategy` trait* (the original lead). Rejected: the
  trait is live. `BumpAndReset` implements it at `regalloc/mod.rs:156` and is
  called at `:243`, reachable whenever the user passes `--regalloc bump`
  (parsed `src/cli/build.rs:198-207`, `:879-888`; applied `:243`). `Allocation`
  is live too (`.physical` read at `:159`/`:247`, `.extra_callee_saved` at
  `:250`). Only the two `AllocInput` fields are dead.
- *Delete group D items to reach a zero-warning build faster.* Rejected: silently
  changes a struct offset (D2), gaps a documented arena map (D1), breaks spec
  anchors (D2/D4/D8), or drops a supply-chain integrity guard (D7).
- *Suppress the remaining stragglers with one more blanket.* Rejected — that is
  the root cause.

Expected output shift: A21 removes 13 data objects from every `term::` binary;
A22 and A24 remove dead labels and one branch. All other items are
compile-time-only.

## Phases

### Phase 1 — protect the anchors + audit (no deletions)

- [ ] Apply group D: targeted allows + honest comments on D1–D6, `#[cfg(test)]`
      on D7, and the D8 decision recorded.
- [ ] Confirm each group-A item's citation still resolves at HEAD (line numbers
      drift).

Acceptance: no behavior change; every D item documented with a real reason; the
audit list has a verdict per site.
Commit: —

### Phase 2 — the deletions and the attribute removals

- [ ] Group A: A1–A29, respecting the A4/D8 and A10 judgement calls.
- [ ] Group B: B1–B5 (attributes and spec prose only).

Acceptance: `cargo check` warning count strictly decreases; acceptance suite
green except the three golden-shifting items.
Commit: —

### Phase 3 — remove the blankets, regenerate goldens, full validation

- [ ] Group C: delete all eight `#![allow(dead_code)]`; add targeted allows only
      where a D item requires one.
- [ ] Regenerate goldens for A21/A22/A24; diff and confirm the delta is *only*
      the removed data objects, labels, and branch.
- [ ] `cargo check --all-targets` → zero warnings.
- [ ] Full acceptance suite on every affected target.

Acceptance: zero warnings, zero blanket allows, golden delta exactly the intended
removal, suite green.
Commit: —

## Validation Plan

- Regression test: `cargo check --all-targets` clean, plus the absence of any
  `#![allow(dead_code)]` in `src/` — the removal of the blankets *is* the guard,
  since the compiler now reports any recurrence. Consider a CI grep banning
  file-level `#![allow(dead_code)]` outside an explicit allowlist.
- Runtime proof: build a `term::`-using program before and after; confirm the 13
  escape data objects are gone from the binary and terminal behavior is
  unchanged (A21 is the only item with a shipped footprint).
- Layout proof: for group D, confirm `_mfb_rt_stdin_log` offsets and the NFD
  record stride are byte-identical before and after.
- Doc sync: `06_native.md:327-337`, `01_tables-and-algorithms.md:57,177`,
  `09_closures.md:75-83`, `01_project-manifest.md:42,74-77,278`,
  `01_rule-codes.md:269`.
- Full suite: `scripts/test-accept.sh` on each affected target; golden resync via
  `scripts/sync-goldens.sh` for A21/A22/A24 only.

## Leads that did not hold

Recorded so they are not re-investigated. Each was checked and rejected:

1. **`AllocationStrategy` is a dead abstraction.** False — see Fix Design. Only
   `AllocInput.instructions`/`.model` are dead (A18). The spec is *overstated*,
   not inverted (B5).
2. **`stdin_broadcast.rs:86-95` `emit_libc` is a zero-value passthrough.** False.
   It swaps argument order: `emit_libc(symbol, name, …)` forwards to
   `emit_libc_call(name, symbol, …)` (`src/target/shared/code/types.rs:336-343`).
   It has ~40 callers in that file, and inlining it carelessly would silently
   swap the libc symbol with the caller-provenance string. A naming item, not
   dead code.
3. **`io_helpers.rs:1061-1090` `emit_continuation_read` is a dead pass-through.**
   False — 12 callers (`:1418,1444,1481,1509,1546,1566,1887,1913,1950,1978,2015,…`).
   A deduplication candidate, not dead code.
4. **`io_helpers.rs:559-568` emits 2 spurious `ERR_OUTPUT_SYMBOL` relocations
   into every stderr-flushing binary.** False — no caller passes `stderr = true`
   (`src/target/shared/code/mod.rs:1553` hardcodes `false`), so no such binary
   exists. Replaced by the real finding, A23.
5. **`io_helpers.rs:610` and `:715` are dead labels.** False — `done` is branched
   at `:603`; `os_poll` is branched via `emit_stdin_poll_ready_check` →
   `stdin_broadcast.rs:185`. Both are live in the non-`app_mode` path;
   conditionally inert at most.
6. **`lower_io_flush_helper`'s unused params are justified by a FALSE comment.**
   Overstated — 6 of 8 sibling io helpers *do* take `platform_imports` and
   `platform` (`mod.rs:1512,1605,1641,1674,1688,1721`); `:1568`/`:1582` are the
   exceptions. The parity claim is broadly true. The clean finding in that
   function is the dead `stderr` param (A23).
7. **`AbiSpec.line` has zero reads.** False — read at
   `src/syntaxcheck/mod.rs:762` and `:875`. Inverted into B2: the *attribute* is
   the defect.
8. **`crypto_package.mfb` has 4 helpers that are one implementation under four
   names.** Partial, and not dead code. `__crypto_truncate` (`:313-321`) and
   `__crypto_bytePrefix` (`:2140-2148`) *are* byte-for-byte identical, but
   `__crypto_copyBytes` (`:105-115`) and `__crypto_slice` (`:2101-2109`) have
   different signatures, and all four have live callers. A duplicate-code
   finding.
9. **4 one-line char helpers obsoleted by plan-27.** False — all have live
   callers (`__csv_crChar` at `csv_package.mfb:173`; `__http_crlf` at
   `http_package.mfb:143,217,268`; the two json helpers twice each). Moreover
   `__json_backspaceChar`/`__json_formfeedChar` cannot be replaced by `\b`/`\f`:
   the lexer (`src/lexer.rs:342-370`) supports only `\"  \\  \n  \t  \r  \0
   \u{…}` — there is no `\b` or `\f` escape.
10. **`x86_64/regmodel.rs:ZERO_REGISTER` is dead.** Holds, but already filed as
    bug-300-E5; not duplicated here.
11. **7 blanket file-level allows.** Miscounted — there are **8**, and they cover
    exactly the claimed 2,634 lines (C).
12. **`os.rs` has ~10 dead `alloc_ok` pairs.** There are exactly **8** (A24).
13. **2 audit category ranks are unreached.** There are **3** — `"policy"` at
    `src/audit/report.rs:187` was missed (A16).

## Summary

Roughly 40 verified items across four groups. The engineering risk is not in the
deletions — those are mechanical and individually revertable — but in **group D**
and in the sequencing. Three of the six warnings `cargo check` reports today are
group-D items that must *not* be deleted (`RUNTIME_X86_64_LEN`,
`RUNTIME_AARCH64_LEN`, and — indirectly — the anchors that appear the moment the
blankets come off). A sweep driven by "make the warnings go away" will delete a
layout anchor, a spec-anchored stride constant, or a supply-chain integrity
guard, and nothing will fail loudly. Hence Phase 1 protects the anchors before
Phase 3 arms the compiler.

The lasting value is Phase 3: with the eight blanket allows gone, this entire
class becomes compiler-enforced instead of review-enforced, and the roughly
two dozen "consumed by a later phase" promises stop accumulating. Left untouched:
all compiled-program behavior except the 13 escape data objects, the inert
labels, and one redundant branch.
