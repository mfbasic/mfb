# plan-159-D: Delete `SUB` from the compiler, package format, tooling, spec and man

Last updated: 2026-09-26
Effort: large (3h–1d) (guess: about 27 compiler files plus 16 kind-string sites, 33 spec files and 16 man topics, all measured below)
Depends on: plan-159-C

`SUB` stops existing. `SUB` is no longer a keyword. `FunctionKind::Sub`,
`ExitTarget::Sub`, `DocHeaderKind::Sub`, `IrDocKind::Sub`, `BinaryReprExportKind::Sub`,
the `"sub"` IR/NIR kind, `ExternalSignature.sub`, `FUNCTION_FLAG_SUB` and the SUB-only
rules are all deleted. The spec, `mfb man`, the formatter, `mfb pkg info`, the doc HTML
and the VS Code grammar describe one kind of callable, `FUNC`. A program that still
says `SUB` gets an ordinary parse error. There is no special-case migration path; see
Open Decisions.

References: plan-159-A (prerequisites, measurements), `.ai/specifications.md` and
`.ai/spec-content.md` (citation rot: a deleted symbol makes the *claim* suspect),
`.ai/man-content.md` §9 (narrative topics), `.ai/compiler.md` (checking topology),
`.ai/testing-gates.md`.

## Prerequisites

The plan-wide prerequisites are in plan-159-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-159-C complete | `ls planning/plan-159-C-*` → no match | NOT MET |
| No `.mfb` outside the exclusion list uses `SUB` | plan-159-B §1 `rg` → only the exclusion list | NOT MET |
| No Rust-embedded MFBASIC uses `SUB` outside the SUB-rule tests | plan-159-C §1 `rg` | NOT MET |

## 1. Goal

- `echo 'SUB main()\nEND SUB' > x.mfb; mfb build` → a parse error (the top-level
  "must begin with FUNC, …" diagnostic, whose text no longer lists `SUB`).
- `rg -n '\bSUB\b' src wire tools/editors --type-add 'j:*.{json,js}' -t rust -t j -t md`
  → only the three assembler comments (`x86_64/encode/emitter.rs:654,1757`,
  `aarch64/encode/emitter.rs:709`) and the retired-rule `reserved:` strings.
- `rg -n '\bSUB\b' src/docs` → 0.
- The final gate (Validation Plan) is green.

### Non-goals

- `wire::docs` numeric ids are a **frozen wire format** (`wire/src/docs.rs:142`):
  `DOC_KIND_SUB = 1` is not renumbered. It becomes a reserved id. The existing
  unknown-id rule already reads it as `"func"` (`doc_kind_name`).
- Retired rule codes keep their numbers as `reserved:` entries, following the
  `2-203-0100` precedent (`src/rules/table.rs`, "reserved: retired by plan-59-E").
- No codegen change. `.ncode`/`.ncodesum` stay byte-identical, because after B/C no
  input reaches a `"sub"` path any more.

## 2. Current State (from the A census; every site was read)

**Compiler symbols.**
`rg -c '(FunctionKind|ExitTarget|DocHeaderKind|BinaryReprExportKind|IrDocKind|Keyword)::Sub\b|is_sub|current_is_sub|\bsub:\s' src --type rust`
reports 30 files. Four of them are false positives: `arch/x86_64/encode/emitter.rs`
(a comment), `codegen/builtins/collections/mod.rs` (`func_is_subset`),
`codegen/memory/arena/arena.rs` (a register param named `sub`), and 2 of the 3 hits in
`ir/tests.rs`. The real sites:

- **Lexer:** `src/lexer.rs` (`Keyword::Sub` l.98, `keyword()` l.1201,
  `keyword_lexeme` l.1284, the round-trip test l.1505).
- **AST:** `src/ast/types.rs` (`FunctionKind::Sub` l.491, `ExitTarget::Sub` l.638,
  `DocHeaderKind::Sub` l.103/118); `src/ast/items.rs` (`parse_function` l.66 `ISOLATED`
  check and l.131 `END SUB`; `check_top_level_item_start` l.516; message l.59);
  `src/ast/stmt.rs` (`EXIT SUB` l.125, l.135); `src/ast/parser.rs:291`;
  `src/ast/doc_items.rs:15,24,34`; `src/ast/serialize.rs` (l.838, 1542, 1627);
  `src/ast/build.rs:175` (if C has not already deleted it).
- **IR:** `src/ir/lower.rs` (`function_return_type` l.653, `lower_function` l.664
  kind `"sub"`); `src/ir/shape.rs` (`current_is_sub` l.121, `SUB_RETURN_FORBIDDEN`
  l.1584–1589, `EXIT_SUB_IN_FUNC` l.1613–1619, the rule list l.4933,
  `thread_start_entry_valid` l.2488/2495); `src/ir/verify/ops.rs:486`;
  `src/ir/verify/values.rs` (after A this keys on `Nothing`, so only its comment
  wording remains); `src/ir/verify/mod.rs:684`; `src/ir/types.rs:144`
  (`ExternalSignature.sub`); `src/ir/docs.rs` (`IrDocKind::Sub` l.23, l.171).
- **Target:** `src/target/shared/validate/names.rs:81,112` (accepts `"func" | "sub"`);
  `src/target/shared/nir/lower.rs:219` (the global initializer is built with kind
  `"sub"`); `src/target/shared/plan/mod.rs:556` and `validate/mod.rs:253` (test builders).
- **Other front-end:** `src/monomorph/helpers.rs:527,666,684`;
  `src/manifest/entry.rs:44`; `src/manifest/package.rs` (4);
  `src/resolver/resolution.rs:210` (+3); `src/doc/mod.rs:181,192`;
  `src/doc/html.rs` (`kind_label("sub")` → "Subroutine"); `src/fmt.rs` (17:
  `Block::Sub` l.387, l.446, `K::Sub` l.551, l.775/781; comments l.546/591);
  `src/cli/pkg.rs:2130` `package_export_kind_name` and its test l.2390;
  `src/codegen/engine/validation/validation.rs:908`.
- **Package format:** `src/binary_repr/mod.rs:139` (`FUNCTION_FLAG_SUB`),
  `:220` (`BinaryReprExportKind::Sub`); `sections.rs:906,1144,1167`; `writer.rs:762,1333`;
  `reader.rs:1359,1370` (`decode_callable_export_kind` accepts kinds 1 and 2); the
  tests under `binary_repr/tests/{writer,writer_walker,fixtures,reader,reader_tests}`.
  The ABI hash masks flags to `ISOLATED | SUB` (`package/03_metadata-encoding.md:189`).
- **wire:** `wire/src/docs.rs:144,153,242,316` and `wire/src/docpage.rs:74,85,304`.

**Rules (`src/rules/table.rs`).**
- `2-203-0042 TYPE_SUB_CANNOT_RETURN_VALUE` (l.561): already dead. Nothing emits it.
- `2-203-0073 SUB_RETURN_FORBIDDEN` (l.971).
- `2-203-0077 EXIT_SUB_IN_FUNC` (l.995).
- `2-203-0074 TYPE_SUB_HAS_NO_VALUE`: SUB-only (A no longer renames or widens it — plan-159-A Corrections). Retired here with `SUB`: nothing else can emit it, because a `Nothing` call's value is an ordinary `NOTHING` (bug-744).
- The message text of `1-103-0002 DOC_BAD_HEADER` (l.1384, "FUNC, SUB, TYPE…") and
  `2-205-0019 DOC_GROUP_INVALID_CONTEXT` (l.1511) mentions SUB.

**Tooling outside `src/`.** `tools/editors`: 10 lines in 4 files
(`rg -n '\bSUB\b' tools/editors`): `language-configuration.json:24`,
`syntaxes/mfbasic.tmLanguage.json:98,101,125`, `mfbasic-parse.js:19,23,73,135`,
`README.md:7,97`. `tools/mcp`: 0. There is no LSP. `repository/src` has 0 real hits;
its `sub` matches are JWT and CSS.

**Docs.**
- Spec: 114 lines in 33 files (`rg -n '\bSUB\b' src/docs/spec | wc -l`). The largest
  are `language/08_error-model.md` (16), `language/07_subs.md` (16, the whole
  chapter), `language/20_worked-example.md` (8), and `21_documentation.md`,
  `06_functions.md`, `diagnostics/01_rule-codes.md`, `architecture/02_frontend.md`
  (6 each).
- Cross-references to the subs chapter: `rg -n 'language subs|see §7' src/docs`
  (`06_functions.md:15`, `04_types.md:399`, `10_control-flow.md:63`).
- Man: 83 lines in 16 files (`rg -c '\bSUB\b' src/docs/man`), of which
  `variable/package.md` has 32.

**Fixtures pinning SUB-only rules.** These are the plan-159-B exclusion list, whose
resolved paths are recorded in B §2.

## 3. Design Overview

This is dead-code deletion once C has landed, so **byte-identity of
`.ncode`/`.ncodesum` is the correctness gate**. After B and C, no `.mfb` input
reaches a `"sub"` path. The one exception is `nir/lower.rs:219`: the global
initializer is internally built with kind `"sub"`. Changing it to `"func"` must
be `.ncode`-neutral, because codegen does not branch on kind (A §2). Its `.nir`
goldens are **expected** to move (`"kind": "sub"` → `"func"` for the
initializer). Any `.ncodesum` diff means a bug: objdump one fixture and fix.

Correctness risk is concentrated in the package reader. The design choice
(Open Decision 1) decides what an old `.mfp` with export kind 2 or the SUB flag
bit does. Design uncertainty is concentrated in the spec rewrite. The `SUB`
chapter's surviving truths (a `Nothing` call's value is `NOTHING`, `Nothing` callbacks,
inline-`TRAP` `RECOVER` with no operand, the trap-exit rules) move into
`06_functions.md` / `08_error-model.md`, and the chapter is deleted.

**What each retired fixture becomes.** Answer the AGENTS.md four questions in
the commit for each fixture:

- `sub-value-less-invalid` → its `RETURN NOTHING` / `RETURN` cases no longer
  exist as rules, and its value-position cases stop being errors once `SUB` is
  gone (a `FUNC … AS Nothing` call yields `NOTHING`). Delete it. Proof: the rule
  it pins (`2-203-0074`) is retired by this plan (the spec diff), and the
  surviving behavior — `Nothing` calls in value position bind `NOTHING` — is
  pinned by bug-744's `tests/runtime/rt_nothing_func_value_position.rs`.
- `isolated_sub_invalid` / `func_isolated_requires_func_invalid` → with no
  `SUB`, `ISOLATED` can only precede `FUNC`. Keep the `ISOLATED`-on-non-FUNC
  cases that still exist (lambda, local function) and drop the SUB case.
- `exit-sub-invalid` → the `EXIT SUB` statement no longer parses. Delete
  `EXIT_SUB_IN_FUNC`. If `EXIT FUNC` stays (it is always
  `EXIT_FUNC_FORBIDDEN`), keep its case.
- `control-flow-sub-trap-invalid` → convert it to `FUNC … AS Nothing`. The same
  fall-through is caught by `TYPE_TRAP_FALLTHROUGH` (proved in A §2), so the
  test still protects the behavior.

**Rejected:** keeping `SUB` as a recognised keyword that emits "SUB was
removed; use FUNC … AS Nothing". That is a compatibility path kept alive
forever, the opposite of the premise. It is listed as Open Decision 2 in case
the owner wants it for external users.

## Phases

> Tick in the same commit as the work. **An unticked box means NOT DONE.**

### Phase 1 — Package format and `wire`

- [ ] 1.0 Measure (UNMEASURED today): which packages published on the registry
      export a SUB. The command depends on the registry API
      (`.ai/resources-packages.md`); record the command and count here. **Stop and
      ask the owner** before republishing anything. That is an outward action.
- [ ] 1.1 `src/binary_repr`: delete `BinaryReprExportKind::Sub`. Make export kind
      `2` and the `FUNCTION_FLAG_SUB` bit reserved: the writer never emits them,
      and the reader handles them per Open Decision 1. Drop `SUB` from the ABI
      flag mask. Update `sections.rs`, `writer.rs`, `reader.rs`,
      `monomorph/helpers.rs:527`, `codegen/engine/validation/validation.rs:908`,
      `cli/pkg.rs` `package_export_kind_name` and its test, and the
      `binary_repr/tests/**`.
- [ ] 1.2 `wire/src/docs.rs`: `DOC_KIND_SUB` → a reserved id with a comment (the
      frozen number `1` is never reused). Remove the `"sub"` parse arm (l.242)
      and the `"sub"` name arm. Update `wire/src/docpage.rs:74,85,304` and
      `src/binary_repr/tests/reader_tests.rs:173`. `src/ir/docs.rs`: delete
      `IrDocKind::Sub`.
- [ ] 1.3 Spec: `package/01_container-format.md:79`, `03_metadata-encoding.md`
      (l.73, 131, 134, 175, 189), `07_functions.md:65,91`, `11_doc-section.md:26`,
      `12_verifier-rules.md:101`, `08_ir-section.md:77`.

Acceptance: the packages round-trip, and none of the 161 tracked `.mfp` changes.
  Check: `cargo test --bin mfb binary_repr` → pass (est. 3 min); `bash scripts/test-accept.sh target/release/mfb /tmp/p159-acc 'package-*'` → all PASS, including the `.info` goldens (est. 2 min); rebuilding one B-rebuilt package (`packages/logger`) → `cmp` equal to its tracked `.mfp` (est. 1 min; B already removed every SUB export, so a byte change here is a writer bug).
Commit: —

### Phase 2 — Front end, IR, target, rules

- [ ] 2.1 Lexer: delete `Keyword::Sub` (4 sites). `SUB` becomes an ordinary identifier.
- [ ] 2.2 AST: delete `FunctionKind::Sub`, `ExitTarget::Sub` and
      `DocHeaderKind::Sub`, and their parse, serialize and doc sites (list in §2).
      Reword the parser messages at `ast/items.rs:59`, `ast/parser.rs:291` and
      `ast/doc_items.rs:15,34` to stop listing SUB. Keep the `ISOLATED` check at
      `items.rs:66` for the non-FUNC forms that remain.
- [ ] 2.3 IR: `function_return_type` / `lower_function` lose the Sub arm.
      Delete `current_is_sub` and both SUB rules from `shape.rs`, and remove the
      `"sub"` arm from `verify/ops.rs:486`. `ExternalSignature.sub` is deleted, and
      `thread_start_entry_valid` keys on `kind == Func` + `ISOLATED` only.
- [ ] 2.4 Target: `validate/names.rs:81,112` accept `"func"` only;
      `nir/lower.rs:219` uses `"func"`; the test builders use `"func"`. Then
      remove the IR `kind` string entirely if `"func"` is now its only value, or
      record here why it stays.
- [ ] 2.5 `manifest/entry.rs`, `manifest/package.rs`, `resolver/resolution.rs`,
      `doc/mod.rs` and `doc/html.rs` ("Subroutine" label): drop the Sub arms.
- [ ] 2.6 `src/fmt.rs`: delete `Block::Sub` / `K::Sub` and fix the comments at
      l.546/591. Spec `tooling/05_fmt.md`.
- [ ] 2.7 `src/rules/table.rs`: 0042, 0073 and 0077 → `reserved: retired by
      plan-159-D (SUB removed)`, following the 0100 precedent. Reword the
      messages of 1-103-0002 and 2-205-0019. Update
      `diagnostics/01_rule-codes.md` to match.
- [ ] 2.8 Delete the SUB-rule Rust unit tests that C excluded
      (`rg -l 'SUB_RETURN_FORBIDDEN|EXIT_SUB_IN_FUNC|FunctionKind::Sub|ExitTarget::Sub|Keyword::Sub' src tests --type rust`
      → 0 afterwards). Record the four answers for each deleted test in the
      commit message.
- [ ] 2.9 Retire or convert the exclusion-list fixtures per §3. Add
      `tests/syntax/functions/sub-keyword-removed-invalid/`, where `SUB main()`
      yields the ordinary top-level parse error, as the regression test that
      `SUB` is gone.

Acceptance: `SUB` is rejected, and codegen is unchanged.
  Check: `bash scripts/test-accept.sh target/release/mfb /tmp/p159-acc sub-keyword-removed-invalid control-flow-sub-trap-invalid` → PASS; `cargo test --release --test rt_nothing_func_value_position` → pass (est. 1 min); `bash scripts/artifact-gate.sh target/release/mfb strings` → 0 diffs (est. 1 min).
Commit: —

### Phase 3 — Spec, man, editor grammar

- [ ] 3.1 Spec: move the surviving content of `language/07_subs.md` into
      `06_functions.md` (a `Nothing` call's value is `NOTHING`, `Nothing` callbacks) and
      `08_error-model.md` (value-less `RECOVER`, trap exits via bare `RETURN`),
      then delete `07_subs.md`. Chapter numbering follows Open Decision 3. Fix
      the cross-references (`06_functions.md:15`, `04_types.md:399`,
      `10_control-flow.md:63`) and every remaining hit in the 33 files from §2.
      `19_grammar.md`: remove the `SUB` productions and `EXIT SUB`.
      `02_lexical-structure.md`: remove `SUB` from the keyword list.
- [ ] 3.2 Man narrative topics (`src/docs/man/**`, 16 files): rewrite the prose
      that mentions `SUB`, `variable/package.md` (32) first. Follow
      `.ai/man-content.md` §9 bans.
- [ ] 3.3 `tools/editors`: the 10 lines in 4 files (§2). Bump the extension version
      if its README/manifest versions releases.
- [ ] 3.4 `tools/sub-to-func/`: decide per Open Decision 2 (keep it for external
      migration, or delete it). If deleting, grep its name and update every live
      caller in the same commit.

Acceptance: no `SUB` left in the docs, the citations resolve, and the man pages render.
  Check: `rg -n '\bSUB\b' src/docs` → 0 (est. 5 s); `bash scripts/spec-census.sh --citations` and `--links` → 0 dangling (est. 1 min); `cargo test --bin mfb spec` → pass (est. 3 min); `bash scripts/man-census.sh --memory-scope` → 0 unclassified (est. 1 min).
Commit: —

## Validation Plan

- Tests: `sub-keyword-removed-invalid` (negative), bug-744's
  `rt_nothing_func_value_position` and A's `entry-func-nothing-*`, and the converted `control-flow-sub-trap-invalid`.
- Coverage: every deleted branch was reachable only from `SUB` input. After B and C
  the only `SUB` input is the new negative fixture, which exercises the parse-error
  path.
- Runtime proof: `mfb init /tmp/p159-final && mfb run /tmp/p159-final` → runs;
  `mfb run examples/<one example that had SUBs>` → the same output as before B.
- Doc sync: the spec (all 33 files), the man topics, `diagnostics/01_rule-codes.md`,
  and the editor README.
- **Final gate (run ONCE, after Phase 3):** `cargo test --no-fail-fast` (est. 40 min;
  this is the plan's single full-suite run) and
  `bash scripts/artifact-gate.sh target/release/mfb all` → 0 unexpected diffs (est.
  20 min). Before calling it green, check that every diff is in the predicted set
  (`.nir` initializer kind), and fix anything else.

## Open Decisions

1. **An old `.mfp` carrying export kind 2 or `FUNCTION_FLAG_SUB`.** Two options:
   - **Recommended:** the reader rejects it with a clear "rebuild this package"
     diagnostic. There is one package format and no compatibility path (AGENTS.md:
     no fallbacks), and every tracked `.mfp` is rebuilt in B.
   - Alternative: read kind 2 as kind 1. This silently accepts packages built by
     an older compiler forever.
   Measure task 1.0 first. If published registry packages export SUBs, the
   recommended option requires republishing them, which is the owner's call.
2. **Old source that says `SUB`.** Two options:
   - **Recommended:** a plain parse error, with no special case.
   - Alternative: a dedicated diagnostic pointing at `tools/sub-to-func/`, keeping
     the tool. This is kinder to external users, but it keeps the keyword in the
     lexer.
3. **Spec chapter numbering after deleting `07_subs.md`.** Two options:
   - **Recommended:** leave the gap. Renumbering 08–22 churns every `§N`
     cross-reference for no reader benefit.
   - Alternative: renumber. First check whether `PACKAGE_ORDER` or a spec test
     requires contiguous numbering (`src/docs/spec/mod.rs:43`).
4. **`EXIT FUNC`.** It is always an error (`EXIT_FUNC_FORBIDDEN`). With `EXIT SUB`
   gone, the whole `EXIT <callable>` form serves no purpose.
   - **Recommended:** leave it out of scope for plan-159 and file it as a follow-up
     plan.
   - Alternative: fold its removal in here.

## Corrections

(none yet)

## Summary

After B and C, D deletes dead code, and byte-identical native goldens gate it. The
real decisions are the package-reader policy for old `.mfp` files (which may involve
republishing registry packages, an owner call) and where the `SUB` chapter's
surviving rules move to in the spec.
