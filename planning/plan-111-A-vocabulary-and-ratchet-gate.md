# plan-111-A: complete the type vocabulary; install the ratchet gate

Last updated: 2026-08-29
Overall Effort: huge (>3d) — the whole plan-111 feature
Effort: large (3h–1d)
Depends on: nothing within 111 (plan-106 and plan-107 complete is the gate)

**plan-111 has exactly one task: delete every type string after the AST.**
No permitted-boundary classes, no "honest gaps", no follow-ups. When G ticks
its last box, `ParameterType` is the compiler's only type currency from
`hir::elaborate` to the emitted byte, and a `cargo test` gate makes reintroducing
a type string a build failure.

plan-106 declared this end state and shipped with 155 of its own violations
recorded as a follow-up (plan-106-E census line 6: "codegen re-parses a type NAME
in 109 places … recorded as follow-up"). plan-107 relocated the checkers but did
not touch the vocabulary. This plan finishes it, and the reason it is split into
seven letters is volume, not difficulty: every task below is a mechanical signature
change. **The design is already decided; the letters are the work.**

This sub-plan is the **lead document for plan-111**. Roadmap — letter order is
implementation order, and every letter's violation count is measured, not
estimated (§2):

| Letter | Delivers | Sites cleared | Effort |
|---|---|---|---|
| **A** (this) | `Hash`; the `STATE` variant; the ratchet gate installed with today's counts as budgets | 2 grammar copies | large |
| **B** | `ir`, `monomorph`, `resolver` typed; the front-end→codegen re-parse seams deleted | 88 | large |
| **C** | `TypeModel` re-keyed by type; the registry's dual API collapsed — **the string bottleneck** | 107 + 84 callers | large |
| **D** | codegen's scalar-semantics cluster (math, conversion, numeric, strings, money, SIMD) | 122 | large |
| **E** | codegen's collections and layout cluster | 161 | large |
| **F** | codegen's memory, engine, resource and builtin-package remainder → **codegen at zero** | 175 | large |
| **G** | `src/target` + the `.mfp` encoder; the terminal census; gate locked to hard zero; **the single byte-identity sweep** — `artifact-gate all` + `test-accept` + goldens regenerated once, after attribution; archive | residue + proof | large |

Dependency graph is a straight line: A → B → C → D → E → F → G. Every letter
lowers a gate budget CI enforces, so "mostly done" is not a state this plan can
be left in.

This letter is the only one with design content. It does three things:

1. **Installs the ratchet gate first**, so every later letter's progress is a
   number CI enforces and half-completion is impossible to hide.
2. **Derives `Hash` on `ParameterType`** so maps can be keyed by the type
   instead of by its spelling (C's whole job).
3. **Gives `STATE` a variant**, killing the last construct the type enum cannot
   express — the reason two hand-rolled grammar copies exist today.

References:

- `tests/architecture_guards.rs` — the whole-tree filesystem-lint precedent this
  letter's gate mirrors exactly (scan roots, `code_above_tests` `#[cfg(test)]`
  stripping, per-invariant exemption fn, "hard floor of 0" doc comment).
- `src/types.rs:22` — `ParameterType`'s derives (`Clone, Debug, PartialEq, Eq`;
  no `Hash`); `:485-536` — `split_state`/`state`/`without_state`; `:785` —
  `split_state_clause` copy 1; `:999` — the `round_trip` corpus helper.
- `src/codegen/resource/mod.rs:40` — `split_state_clause` copy 2, with a doc
  comment admitting the duplication and naming the parity test that pins it.
- `src/intern.rs:27` — `Symbol(NonZeroU32)`, already
  `Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord`.
- `planning/completed/plan-106-E-consolidation-no-strings-census.md` — the census
  this plan drives to zero, and its Correction 3 (the `split_once(" TO ")`
  mis-split) as the standing evidence for why a second grammar is a bug factory.
- `.ai/testing-gates.md`, `.ai/codegen-invariants.md`, `AGENTS.md`.

## Prerequisites

These are a precondition on the whole plan-111 feature, not a dependency to
negotiate. Letters B–G point here.

| Must be true | Command | Status |
|---|---|---|
| plan-106 complete | `ls planning/completed/plan-106-*` → 5 letters + 2 baselines | MET (re-verified 2026-08-29: 5 letters + 2 baselines) |
| plan-107 complete | `ls src/syntaxcheck` → no such directory; `rg -c RELOCATED_TO_IR_VERIFY src/` → 0 | MET (re-verified 2026-08-29: no such directory, 0 hits) |
| Tree compiles clean at HEAD | `cargo check --all-targets` | MET (re-verified 2026-08-29 in worktree P-111, 35s, no warnings) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop.
>
> **If you stop, report the current status of *all* prerequisites**, not only the
> one that blocked you.

**Every golden/byte-identity sweep runs ONCE, at the end of plan-111 (letter G)
— not per phase and not per letter.** That means `artifact-gate.sh all`,
`tests/golden.rs` (whose only test *is* `artifact-gate.sh all`), and
`scripts/test-accept.sh`. Per-phase gating uses the fast suite only. See §3 for
the exact commands and how a diff found at the end gets attributed.

## 1. Goal

- `ParameterType` is the only type representation between `hir::elaborate` and
  codegen's emitted bytes. After this plan:
  - **0** `ParameterType::parse` calls outside the five sanctioned boundary
    files (§2 "The five boundaries").
  - **0** functions taking a type as `&str` outside `src/ast/`, `src/lexer.rs`,
    and those five files.
  - **0** `match`/`==` decisions on a type spelling outside them.
  - **0** hand-rolled type-grammar operations (`split_once`/`strip_prefix`/
    `starts_with` on a type token) outside `ParameterType::parse` itself.
  - **0** type-keyed maps keyed by `String`.
- `tests/no_type_strings.rs` enforces all five as hard zeros and fails
  `cargo test` if any is reintroduced.
- This letter specifically: the gate exists with today's measured counts as its
  budgets, `ParameterType` is `Hash`, and `STATE` is a variant.

### Non-goals (explicit constraints)

- **No behavior change, anywhere in the plan.** Codegen output byte-identical;
  diagnostic text, codes, ordering and locations unchanged on both corpora.
- **No wire-format change.** `.mfp` type tables, IR JSON/binary, and the manifest
  all keep their exact current spellings. The `parse`↔`name` round trip stays
  byte-exact — it is load-bearing for every wire seam
  (`src/types.rs:999` `round_trip`, `:1089`, `:1108`, `:1137`, `:1385`).
- **No new abstraction layer.** No `TypeId`, no type-system framework, no trait
  for "things that have a type". `ParameterType` and its existing methods only.
- **No public/CLI surface change.** `mfb man`/`mfb spec` output unchanged.
- Do not "improve" a rule while converting it. A converted site must make the
  same decision on the same input; if it cannot, that is a bug found — file it
  per `AGENTS.md`, fix it, and say so in the commit.

## 2. Current State

The front end is typed; codegen is not. `NirValue` already carries
`ParameterType` (`src/target/shared/nir/mod.rs:259`), and so do IR/HIR — but
codegen renders those back to `String` and re-parses, because its layout,
symbol-mangling and helper tables are keyed by rendered names
(`TypeModel`, `src/codegen/engine/builder/mod.rs:597-643`: eight maps, seven of
them `String`-keyed). The registry carries a **dual API** — typed and string
versions of the same query — and the string half is the one nearly everything
calls.

`STATE` has no variant. `src/types.rs:490` states it plainly: "outside a thread
plane `parse` has no arm for it, so `File STATE Cursor` is one opaque `Named`."
That single gap is why `split_state_clause` exists twice, hand-rolled, in
`src/types.rs:785` and `src/codegen/resource/mod.rs:40` — the second copy's own
doc comment says it is "re-stated here so `ParameterType::split_state` does not
reach into `codegen` for the grammar half of its own vocabulary," pinned by
`split_state_matches_the_name_domain_helpers` (`src/types.rs:1356`).

### The five boundaries

A type spelling legitimately exists in exactly five places — all of them
*converting between the string world and the type world*, none of them making a
decision. These are the only files `ParameterType::parse` may appear in after F:

| # | File(s) | Why | parses today |
|---|---|---|---|
| 1 | `src/types.rs` | the parser's own recursion | 38 |
| 2 | `src/ir/binary.rs` | IR wire/JSON decode | 27 |
| 3 | `src/hir/mod.rs`, `src/hir/build.rs` | AST→HIR elaborate (the AST boundary) | 4 |
| 4 | `src/binary_repr/writer.rs`, `src/binary_repr/sections.rs` | `.mfp` wire codec | 2 |
| 5 | `src/manifest/package.rs` | manifest entry decode | 2 |

Total sanctioned: **73**. `src/ast/**` and `src/lexer.rs` are outside the scan
entirely — the AST *is* the string domain.

### Measured populations

Every count below was produced by the command beside it, run at HEAD
(`fd09ea809`) on 2026-08-29, with tests excluded via
`--glob '!**/tests*' --glob '!**/*_tests.rs' --glob '!src/testutil.rs' --glob '!src/docs/**'`.
Abbreviated `r` below; letters B–G re-run these and lower the gate budgets by
exactly what they removed.

| What | Total | codegen | ir | monomorph | other | Command |
|---|---|---|---|---|---|---|
| `ParameterType::parse` sites | 228 | 109 | 46 | 26 | types 38, hir 4, manifest 2, binary_repr 2, resolver 1 | `r 'ParameterType::parse\(' src/` |
| — of those, **to remove** | **155** | 109 | 19 | 26 | resolver 1 | total minus the 73 boundary sites above |
| Type-as-`&str` params | 185 | 143 | 13 | 7 | ast 5, target 4, binary_repr 4, resolver 3, types 2, numeric 2, manifest 1, hir 1 | `r '\b(type_\|type_name\|element_type\|value_type\|key_type\|field_type\|return_type\|declared_type\|target_type\|source_type\|state_type\|param_type\|arg_type\|base_type\|union_type\|member_type\|collection_type\|scrutinee_type)\s*:\s*&(\x27[a-z]+ )?str' src/` |
| Match arms on a type spelling | 186 | 147 | 10 | 1 | binary_repr 19, types 9 | `r '^\s*"(Integer\|String\|Boolean\|Float\|Fixed\|Byte\|Money\|Nothing\|AttributeString\|Scalar\|Unknown\|Error)"( \| "[A-Za-z]+")* =>' src/` |
| `==`/`!=` against a type spelling | 73 | 59 | 6 | 1 | types 3, target 2, resolver 1, ast 1 | `r '[!=]= "(Integer\|String\|Boolean\|Float\|Fixed\|Byte\|Money\|Nothing\|AttributeString\|Scalar\|Unknown\|Error\|Result)"' src/` |
| Hand-rolled grammar ops | 57 | 12 | 9 | 1 | types 25, resolver 4, binary_repr 4, manifest 1, ast 1 | `r '(split_once\|strip_prefix\|strip_suffix\|starts_with\|ends_with\|contains)\("( STATE \| TO \| OF \|List OF\|Set OF\|Map OF\|Result OF\|MapEntry OF\|RES \|Thread OF\|ThreadWorker OF\|FUNC\(\|ISOLATED FUNC\()' src/` |
| `format!` type construction | 15 | 2 | 0 | 1 | types 6, binary_repr 5, ast 1 | `r 'format!\("(List OF\|Set OF\|Map OF\|Result OF\|MapEntry OF\|Thread OF\|ThreadWorker OF\|RES )' src/` |

Per-file distributions that set D's and E's split are recorded in those letters.

The string-half-vs-typed-half registry split, which is C's population
(`rg -n "\b<fn>\(" src/ --glob '!**/tests*' | grep -v 'pub(crate) fn' | wc -l`):

| Query | string callers | typed callers |
|---|---|---|
| `resolve_call` / `resolve_call_typed` | 37 | 4 |
| `call_return_type` / `call_return_type_typed` | 32 | 2 |
| `argument_types` / `argument_types_typed` | 8 | 5 |
| `builtins::resolve_call_return_type` | 7 | — (no typed twin yet) |

### Verified properties

- **`Symbol` is already `Hash`** — read `src/intern.rs:26-27`:
  `#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]`. So deriving
  `Hash` on `ParameterType` requires no other change; every payload
  (`Symbol`, `Box<ParameterType>`, `Vec<ParameterType>`, `usize`, `bool`) is
  already `Hash`.
- **`tests/architecture_guards.rs` is a working precedent, not a proposal** —
  read in full (271 lines): it scans `src/codegen` and `src/target` from the
  integration-test crate, strips `#[cfg(test)]` bodies with `code_above_tests`,
  carries a per-invariant `is_*_scan_exempt` fn, and documents a "hard floor of
  0" for a completed migration. The gate in Phase 1 is this file's shape applied
  to a different needle set.
- **The two `split_state_clause` copies are byte-identical in behavior** — read
  both (`src/types.rs:785-793`, `src/codegen/resource/mod.rs:40-46`): same
  `split_once(" STATE ")`, same `base.contains(' ')` composite guard, same
  `Option<(&str, &str)>` return. Pinned by `split_state_matches_the_name_domain_helpers`
  (`src/types.rs:1356`). Replacing both with one variant is behavior-preserving
  *provided* the parse arm keeps the top-level-only rule — Phase 3's risk.
- **UNVERIFIED: how many `match` sites on `ParameterType` have a `_` arm.** A new
  variant is silent wherever one does (`.ai/codegen-invariants.md`; the
  one-type-grammar memory). Phase 3 task 1 measures this before adding the
  variant, not after.

## 3. Design Overview

Three independent pieces, layered:

**The ratchet gate (Phase 1) — the anti-half-completion mechanism.**
`tests/no_type_strings.rs` scans the tree for the six needle classes in §2 and
asserts each per-directory count is `<=` a hardcoded budget. The budgets start at
today's measured numbers. **Every commit in letters B–G lowers the budgets it
cleared, in the same commit as the work.** A regression fails `cargo test`.
Letter G sets every budget to 0 and deletes the budget table, leaving bare
`assert_eq!(count, 0)`.

This is scheduled first on purpose. It is the cheapest thing to build, it has
zero blast radius, and from the moment it lands the plan's remaining work is a
number in CI rather than a claim in a document. It also falsifies the plan's one
tooling premise — that a whole-tree needle scan can distinguish these six classes
without drowning in false positives — before any conversion work is spent.

**`Hash` (Phase 2) — one line, unblocks C.** Nothing can be keyed by
`ParameterType` until this lands.

**The `STATE` variant (Phase 3) — where the design uncertainty is.** Adding a
variant to a type whose `parse`/`name` round trip is load-bearing at five wire
seams is the only step in plan-111 that could be *wrong* rather than merely
tedious. It is scheduled here, first and alone, as the cheapest experiment: the
existing `round_trip` corpus (`src/types.rs:999`) proves byte-exactness in
seconds, and if the top-level-only split cannot be expressed in the parser the
whole plan learns it on day one instead of in letter E.

Correctness risk concentrates in Phase 3 (silent `_` arms) and, later, in C
(re-keying `TypeModel`, whose blast radius is all of codegen). Everything in
B, D and E is mechanical signature replacement with a byte-identity gate.

### The gate policy: cheap gates per phase, byte-identity once at the end

plan-111 is a **provably-neutral representation migration**: the compiler must
emit the same bytes, the same diagnostics, in the same order, before and after.
Byte-identity is therefore the correct final acceptance check — but
`artifact-gate.sh all` is a full cross-target sweep, and running it on every
phase of every letter would cost far more than it catches on work this
mechanical.

So the gate is tiered:

- **Per phase** — the fast suite and the ratchet, nothing corpus-scale:

  ```
  cargo test --no-fail-fast -- --skip artifact_gate_all
  cargo test --test no_type_strings
  ```

  The `--skip` is load-bearing. `tests/golden.rs` contains exactly one test,
  `artifact_gate_all`, and it shells out to `scripts/artifact-gate.sh all` — so
  a plain `cargo test` *is* the full cross-target sweep. Verified: with the skip,
  the golden target reports `0 passed; 1 filtered out; finished in 0.00s`. What
  the per-phase run still covers is every unit test and every `rt_*` runtime
  test — which is where a wrong conversion shows up as a wrong **value**, the
  failure mode that actually matters during the conversion letters.

- **Per letter** — a **scoped, read-only** artifact gate on the 3–4
  `tests/byte-identity/` builtins that letter touched:

  ```
  scripts/artifact-gate.sh target/release/mfb <builtin>
  ```

  Measured: ~31s per builtin (1 test, 6 builds, 7 goldens). It regenerates
  nothing. It is multi-target — per-target goldens are discovered by filename and
  rebuilt with `-target`, so cross-arch drift is caught on a macOS host. Each
  letter names its builtins in its own §End-of-letter spot-check.

  This exists to bound localization. Without it a drift landing in letter B is
  first seen in G, six letters and ~458 converted sites later, and bisecting it
  means repeated full `all` runs — the expensive thing this policy avoids. ~15
  minutes total across the plan buys "the diff is in this letter."

- **Per letter A and B only, additionally** — `scripts/diag-set-diff.sh`. These
  are the only two letters that can move a source diagnostic; C–F touch codegen,
  which emits none.

- **Once, in letter G** — the full sweep and the only regeneration:
  `scripts/artifact-gate.sh all` (equivalently, `cargo test --test golden`
  unskipped), `scripts/test-accept.sh` and `MFB_OPT=3 scripts/test-accept.sh`,
  and `scripts/diag-set-diff.sh`. Goldens are regenerated **once**, there, after
  attribution.

**Attribution at the end — and this is the cost being bought.** Because no
byte-level gate runs before G, a diff surfacing there carries no "everything up
to here was clean" context, and it could have been introduced six letters
earlier. That is an accepted, deliberate trade: the sweeps are expensive and the
conversion letters are mechanical. It is paid for two ways.

First, the per-phase `rt_*` runtime tests catch *wrong behavior* as it lands, so
what reaches G should be byte drift, not miscompiles. Second, G Phase 4 classifies
every diff against a pre-plan-111 binary (`git worktree add --detach` at the
commit before letter A): baseline output == committed golden → the diff is
plan-111's, find and fix the conversion that caused it; baseline != committed
golden → pre-existing, leave it and record it (`.ai/testing-gates.md`; the
abi-function-migration memory).

**A changed golden is not automatically a golden to regenerate.** plan-111 is a
representation migration: it should be byte-identical. The one place output may
legitimately move is letter E's ` TO `-split fix, if Phase 1's census finds one —
a real bug fix with a real output change, which E is required to call out by name.
Everything else that moved is a bug in a conversion. Regenerating first and asking
later is exactly how plan-102 shipped backward seams behind a green gate.

**A failing byte-identity gate is a bug to root-cause, never a signal the design
is dead.** Objdump one fixture to localize, find the site converted wrong, fix
it. There is no outcome of an artifact-gate run that justifies stopping this plan
or reclassifying a violation as a boundary — that reclassification is exactly
what plan-106-E did with codegen's 109 parses, and it is why this plan exists.

The one place output *may* legitimately move is diagnostics whose text embeds a
type spelling. It must not: `name()` renders identically before and after, so a
diagnostic diff is also a bug. `scripts/diag-set-diff.sh` runs at the end of
letters A and B and again in G, and must record 0 differing, capturing `[exit N]`
and bare `error:` lines (the diagnostic-harness memory — a "518 same" reading
once hid a failing build).

### Rejected alternatives

- **Intern types into a `TypeId(u32)` and key everything by that.** Rejected: a
  second representation to keep in sync with `ParameterType`, and unnecessary —
  `ParameterType::Named` already holds an interned `Symbol`, so nominal
  comparison is already an integer compare. Deriving `Hash` gets map-keying for
  one line. Crucially, interning can be added *later, behind unchanged
  signatures*, precisely because this plan removes strings from those signatures
  first. Doing it now would couple a performance change to a correctness
  migration.
- **Ban `ParameterType::name()` outright.** Rejected: rendering is legitimate at
  three sinks — diagnostic messages, symbol mangling, and wire encode (801
  `.name()` calls, `r '\.name\(\)' src/`). The gate bans the *inputs* to a
  decision (`&str` type params, `parse`, spelling literals in match/`==`), which
  makes a rendered name structurally unable to reach a decision. Banning the
  render half would be unenforceable and would break the wire seams.
- **Convert codegen bottom-up, emitters first.** Rejected: plan-106-E already did
  that (24 emitters retyped, commit `91bce3797`) and the parses simply moved
  below them, which is how the 109 survived. C re-keys the *tables* first, so D
  and E have a typed thing to call.
- **Leave codegen alone and declare the front end done.** Rejected: it is the
  disposition this plan exists to reverse.

## Compatibility / Format Impact

Nothing externally observable changes. Explicitly unchanged:

- `.mfp` package format, including the type-table spellings written by
  `binary_repr::sections::type_id` and read by `binary_repr::reader`.
- IR JSON/binary encoding (`src/ir/binary.rs`).
- `.ncode`/`.ncodesum` for every target — the acceptance check.
- Diagnostic codes, text, severity, ordering, and source locations.
- `mfb man` / `mfb spec` output.
- `project.json` / manifest schema.

Internal only: `ParameterType` gains a variant and a `Hash` impl; `TypeModel` and
the registry change key type and signatures (C); ~172 internal function
signatures take `&ParameterType` instead of `&str` (B, D, E).

## Phases

> **NOTE — keep the checkboxes current as you go.**
>
> - Tick `- [x]` **in the same commit as the work it describes**.
> - `- [~]` for partially done, with one line on what remains.
> - Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`, never delete it.
> - Fill `Commit:` the moment a phase lands.
> - Add any task you discover you needed.
>
> **An unticked box means NOT DONE.**

### Phase 1 — the ratchet gate

Lands the enforcement before any conversion, so all later progress is CI-visible.

- [x] Create `tests/no_type_strings.rs`, modelled on `tests/architecture_guards.rs`:
      reuse its scan-root walk and its ~~`code_above_tests` `#[cfg(test)]` stripper~~
      (copy, don't share — the two files are independent integration tests).
      **Correction 2**: `code_above_tests` is unusable across `src/` and was
      replaced by `test_free_lines`, a brace-depth `#[cfg(test)]`-item stripper.
- [x] Implement the ~~six~~ **seven** needle classes from §1+§2 as named scan functions:
      `parse_sites`, `str_type_params`, `spelling_match_arms`, `spelling_compares`,
      `hand_rolled_grammar`, `format_type_construction`, `string_keyed_type_maps`.
      **Correction 1**: this task's original list dropped §2's `format!` class.
- [x] Scan roots: all of `src/` **except** `src/ast/**`, `src/lexer.rs`,
      `src/docs/**`, and `tests/no_type_strings.rs` itself. Document each
      exclusion in a doc comment with the reason (the AST is the string domain).
      (`is_excluded_from_scan` + `is_test_file`, each doc-commented.)
- [x] Encode the five sanctioned boundary files (§2) as a `const BOUNDARY_FILES`
      list, each entry carrying a one-line justification comment. `parse_sites`
      exempts exactly these; the other five classes exempt none. Pinned by
      `boundary_files_exist_and_are_justified`.
- [x] Add `const BUDGETS: &[(&str, usize)]` — per-directory ceilings seeded with
      §2's measured counts. Assert `count <= budget` and, on failure, print every
      offending `file:line` so the implementer sees the work, not just a number.
      Seeded at **630** across 7 classes (**Correction 3** reconciles this against
      §2's rg counts); shape is `(&str, &str, usize)` — `(class, dir, ceiling)`.
- [x] Assert the budget table is **tight**: any budget strictly greater than the
      live count also fails, with "lower this budget to N". A budget that drifts
      above reality is a silent allowance.
- [x] Doc-comment the file with the "hard floor of 0" statement and a pointer to
      this plan, mirroring `architecture_guards.rs`'s header.
- [x] Tests: the gate IS the test. Add one negative self-test asserting the
      scanner actually fires — a fixture string containing `ParameterType::parse(`
      is counted by `parse_sites`. Delivered as
      `scanners_fire_on_their_own_needles` (17 assertions: one positive and, where
      it separates a hit from a lookalike, one negative per class).
- [x] **Added task** — `curated_type_keyed_tables_all_exist`: every
      `TYPE_KEYED_TABLES` entry must still name a live table, so a rename cannot
      silently drop a row out of the population and look like progress
      (**Correction 5**).

Acceptance: **MET.** `cargo test --test no_type_strings` → `4 passed; 0 failed`
at HEAD with the seeded budgets. Failure demonstrated by lowering
`("parse_sites", "resolver", 1)` to `0`: the run failed with
`parse_sites / resolver: 1 > budget 0` followed by
`src/resolver/resolution.rs:1276 — ParameterType::parse(` and the paste-ready
live table; restoring the row returned it to green.
Commit: 05f2ba4d8

### Phase 2 — `ParameterType: Hash`

One line; unblocks letter C. Safe to land alone.

- [ ] Add `Hash` to `ParameterType`'s derive list (`src/types.rs:22`).
- [ ] Tests: add `parameter_type_hash_agrees_with_eq` in `src/types.rs` tests —
      for the `round_trip` corpus, equal types hash equal and unequal types
      (at least across all container variants) do not collide in a `HashSet`.

Acceptance: `cargo test --no-fail-fast -- --skip artifact_gate_all` green. (A derive
cannot move codegen; if letter G later shows it did, root-cause it there.)
Commit: —

### Phase 3 — `STATE` becomes a variant

Kills the last construct the type enum cannot express, and with it both
hand-rolled grammar copies. Highest design uncertainty in plan-111.

- [ ] **First, measure the silent-variant risk**: list every `match` on a
      `ParameterType` with a `_` or `_ =>` arm
      (`rg -n 'match .*(type_|ptype|parameter_type)' src/ -A30 | rg '_ =>'`),
      record the count in the Corrections section, and review each for whether a
      `Stateful` value reaching it would be silently mishandled.
- [ ] Add `Stateful { base: Box<ParameterType>, state: Box<ParameterType> }` to
      `ParameterType` (`src/types.rs:23`).
- [ ] Add the `name()` arm rendering exactly `"{base} STATE {state}"`
      (`src/types.rs:558`-family), byte-identical to today's
      `with_state` output (`src/types.rs:483`).
- [ ] Add the `parse` arm, splitting on the **top-level** ` STATE ` only —
      reuse the depth-tracking precedent `split_top_level_to` (`src/types.rs:794`)
      rather than `split_once` + a `contains(' ')` heuristic. A nested `STATE`
      inside a `Thread OF … RES File STATE Cursor TO …` plane stays with the inner
      type (plan-54).
- [ ] Rewrite `split_state`/`state`/`without_state` (`src/types.rs:512-535`) as
      structural matches on the new variant. Delete `split_state_clause`
      (`src/types.rs:785`).
- [ ] Delete `codegen::resource::split_state_clause` (`src/codegen/resource/mod.rs:40`)
      and reimplement `base_resource_name`/`state_type_name` on top of the typed
      accessors. (Their `&str` signatures die in letter E; this phase only removes
      the duplicated *grammar*.)
- [ ] Delete the now-vacuous parity test `split_state_matches_the_name_domain_helpers`
      (`src/types.rs:1356`) — with one grammar there is nothing to pin. Replace it
      with `stateful_parses_top_level_only`, asserting the nested-plane cases the
      old `contains(' ')` guard protected.
- [ ] Extend the `round_trip` corpus (`src/types.rs:999`) with every stateful
      spelling in the tree: `File STATE Cursor`, `RES File STATE Cursor`,
      `List OF RES File STATE Cursor`, `Result OF Stream STATE Pending`,
      `Thread OF RES fs.File STATE Cursor TO Integer`,
      `ThreadWorker OF RES File STATE Cursor TO Integer`, `pkg.Name STATE S`.
- [ ] Lower the `hand_rolled_grammar` budget for `types` and `codegen` by what
      this phase removed, in this phase's commit.
- [ ] Tests: `stateful_round_trips_byte_exact`, `stateful_parses_top_level_only`,
      `split_state_is_top_level_only` (existing, `src/types.rs:1332` — must still
      pass unmodified), plus the `.mfp` nested-`Map`-key regression from
      plan-106-E Correction 3 must stay green.

Acceptance: the full `round_trip` corpus is byte-exact including every new
stateful spelling; `cargo test --no-fail-fast -- --skip artifact_gate_all` green;
`scripts/diag-set-diff.sh` 0 differing with exit code and unlocated `error:`
lines captured (A is one of the two letters that runs it — §3).
Commit: —

### End-of-letter spot-check (scoped, read-only)

Before closing this letter, run the scoped artifact gate on the builtins it
touched — **`collections`, `strings`, `math`** (the `STATE` variant touches resource-typed spellings; these three cover the widest type-shape surface):

```
scripts/artifact-gate.sh target/release/mfb collections
scripts/artifact-gate.sh target/release/mfb strings
scripts/artifact-gate.sh target/release/mfb math
```

Measured cost: ~31s per builtin (one builtin = 1 test, 6 builds, 7 goldens).
This is **read-only diffing**: it regenerates nothing and updates no golden. It
is multi-target — per-target goldens (`*.linux-aarch64.ncode` and friends) are
discovered by filename and rebuilt with `-target`, so cross-arch drift is caught
on a macOS host, which no other per-letter check can see.

Expect **0 diffs**. A diff here is this letter's, which is the entire point of
running it now instead of discovering it in G behind six letters of churn —
root-cause it with objdump on one fixture and fix the conversion. **Do not
regenerate a golden here.** All regeneration happens once, in letter G, after
attribution (plan-111-A §3).

## Validation Plan

Run at the end of this letter, and repeated per letter B–G.

- Tests: `cargo test --no-fail-fast -- --skip artifact_gate_all`. Both flags matter:
  `--no-fail-fast` or the `rt_*` tests are silently skipped (they sort after
  `golden.rs`), and `--skip artifact_gate_all` or the run *is* the full
  cross-target sweep this plan defers to letter G.
- Gate: `cargo test --test no_type_strings` — budgets tight and not exceeded.
- Coverage check: the changed code is signature-level and reached by the existing
  suite; confirm `src/types.rs`'s new arms are executed by the round-trip corpus
  rather than assumed (`cargo llvm-cov --bin mfb` per `.ai/build-tooling.md` —
  `mfb` is a binary crate, measure with `--bin`, not `--lib`).
- Runtime proof: **deferred to letter G.** The acceptance corpus is swept once,
  at the end (§3). When G does run it, the second argument is an `rm -rf`
  target — use `/tmp/accept-111g`, never `tests/` or any real directory.
- Artifact gate / goldens: **not run in this letter.** `artifact-gate.sh all`,
  `tests/golden.rs` and `test-accept.sh` are a single end-of-plan sweep in
  letter G.
- Diagnostics: `scripts/diag-set-diff.sh` → 0 differing, with `[exit N]` and bare
  `error:` lines recorded.
- Doc sync: `src/docs/spec/architecture/21_type-name-encoding.md` gains the
  `STATE` variant; `.ai/codegen-invariants.md` and `.ai/testing-gates.md` get the
  gate's existence and how to lower a budget.
- Formatting: `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`
  — the root `--all` does not reach the `repository/` path dependency.

## Open Decisions

- **Variant shape for `STATE`** — `Stateful { base, state }` (recommended: a
  named-field struct variant reads at the ~20 match sites and leaves room for a
  future third field) vs. a tuple variant `Stateful(Box, Box)` matching the
  existing `MapOf(Box, Box)` style. Either satisfies the plan; pick one in
  Phase 3 and use it consistently. (§Phase 3)
- **Gate location** — `tests/no_type_strings.rs` (recommended: mirrors
  `architecture_guards.rs`, and an integration test needs no self-exemption
  reasoning about the compiler's own module tree) vs. a unit test inside
  `src/types.rs`. (§Phase 1)

## Corrections

**1 — Phase 1 named six scan classes; there are seven.** The task list spelled
`parse_sites, str_type_params, spelling_match_arms, spelling_compares,
hand_rolled_grammar, string_keyed_type_maps` — it took §1's sixth goal bullet
("0 type-keyed maps keyed by `String`") but dropped §2's sixth *measured*
population, `format!` type construction (15 sites). Both are real classes with
real populations, so the gate implements **seven**, adding
`format_type_construction`. No scope moved between letters: the `format!` sites
live in `types` (6), `binary_repr` (5) and `codegen` (1), all already owned by
E/F/G.

**2 — `code_above_tests` is wrong outside `src/codegen` and `src/target`.** §Phase 1
says to reuse `architecture_guards.rs`'s stripper, which truncates the file at
the first `#[cfg(test)]`. That is safe *there* because those two roots keep test
code in a trailing module. It is not safe across `src/`: this tree also attaches
`#[cfg(test)]` to individual mid-file items — `src/ir/shape.rs:158`
(`bound_types`, a test-only probe field) and `src/resolver/mod.rs:105`
(`resolve_hir_project`, a test-only entry point) — and truncating there discards
the rest of the file.

Measured, with the naive stripper: `parse_sites/ir` read **4** where 13 exist;
`hand_rolled_grammar/resolver`, `str_type_params/numeric`,
`string_keyed_type_maps/resolver` and `string_keyed_type_maps/binary_repr` read
**0** each, and `src/resolver/mod.rs:275` (`types: HashSet<String>`) was invisible
— 4202-line and 1199-line files scanned down to their first 158 and 105 lines.
Replaced with `test_free_lines`, which strips each `#[cfg(test)]` item by brace
depth and keeps everything else. **This is a latent defect in
`tests/architecture_guards.rs` too**, harmless only because of its narrower
roots; it is noted here rather than changed, since altering that file's counts
is outside plan-111.

**3 — the gate's total is 630, and it is not directly comparable to §2's rg
counts.** §2 measured with `rg -c`, which counts *lines* and whose `--glob`
exclusions can only drop whole files. The gate counts *occurrences* and excludes
inline `#[cfg(test)]` items. Both differences make the gate the stricter and more
honest number, and both were verified rather than assumed:

Reconciled by re-running each §2 command with `-o` (occurrences) and again with
`--glob '!src/ast/**'` added, so the two effects separate cleanly. The
`occurrences, no ast` column is the exact quantity the gate measures, and the
whole remaining gap is inline `#[cfg(test)]` code:

| Class | §2 (`rg -c` lines) | `rg -o` occ. | occ., no `ast` | Gate | Gap = inline test code |
|---|---|---|---|---|---|
| `parse_sites` (non-boundary) | 155 | 155 | 155 | 125 | 30 |
| `str_type_params` | 185 | 185 | 180 | 173 | 7 |
| `spelling_match_arms` | 186 | 186 | 186 | 186 | 0 |
| `spelling_compares` | 73 | 76 | 75 | 73 | 2 |
| `hand_rolled_grammar` | 57 | 59 | 58 | 37 | 21 |
| `format_type_construction` | 15 | 15 | 14 | 12 | 2 |
| `string_keyed_type_maps` | — (not censused) | — | — | 24 | new class, curated (Correction 5) |

(`parse_sites` shows no line/occurrence gap because the one line in the tree
carrying two calls — `src/types.rs:321`,
`map_of(ParameterType::parse(key), ParameterType::parse(value))` — is inside a
boundary file and is exempt anyway. §2's own "228 total / 229 occurrences" is
where that line shows up.)

All 30 dropped `parse_sites` were enumerated and read, and every one is a test
assertion or fixture: 13 in `codegen` (`registry/mod.rs` ×5,
`function_lowering.rs` ×4, `data_objects.rs` ×2, `datetime/mod.rs` ×1), 11 in
`monomorph` (all 8 of `helpers.rs` sit below its `mod tests` at line 596, plus
`lower.rs:2881,2887,2898`), and 6 in `ir`. **No letter's scope changes**: the
per-directory distribution is unchanged, only test-fixture noise is gone, and
B–G lower these gate numbers rather than §2's.

**4 — `TypeModel` has nine type-keyed fields, not "eight maps, seven of them
`String`-keyed" (§2).** Read at `src/codegen/engine/builder/mod.rs:598-641`:
seven `HashMap` (`enum_members`, `record_fields`, `union_variants`,
`union_variant_unions`, `union_variant_tags`, `union_variant_fields`,
`resource_closers`) and two `HashSet` (`union_names`, `resource_names`). All nine
are keyed by a type spelling — eight by a bare `String` and `enum_members` by
`(String, String)`, whose first element is the type name. **Letter C's population
is 9, not 7.** C should re-check its own task list against this.

**5 — "type-keyed maps keyed by `String`" cannot be scanned by a regex.** §1
states the goal but §Phase 1 gives no needle for it, and there is no honest one:
`rg '(HashMap|BTreeMap)<(String|&str|\(String, String\)),|Hash(Set)?<String>|BTreeSet<String>' src/`
returns **1209** lines, of which nearly all are keyed by a *symbol* — a function
name, a binding name, a package alias — which is legitimately a string. Narrowing
to identifiers containing `type` returns 23 lines that both over- and
under-report: it catches `ir/lower.rs`'s `binding_types` (keyed by variable name,
not a type) and misses every `TypeModel` field, none of which contains `type`.

So the class is a curated `TYPE_KEYED_TABLES` list of `(file, identifier)` pairs,
each read and confirmed type-keyed, with the four nearest non-type-keyed
lookalikes named in its doc comment so a later reader does not "fix" them.
`curated_type_keyed_tables_all_exist` (an added task) fails if an entry stops
naming something real, so a rename cannot masquerade as progress.

## Summary

The engineering risk in this letter is Phase 3 alone: a new `ParameterType`
variant is silent at every `match` with a `_` arm, and the `parse`↔`name` round
trip it must preserve is load-bearing at five wire seams. Phases 1 and 2 have no
blast radius at all.

Untouched by this letter and owned by later ones: all 155 removable `parse`
sites, all 185 `&str` type parameters, all 186 spelling match arms, all 73
spelling compares, `TypeModel`'s seven `String`-keyed maps, and the registry's
duplicated string API. This letter only makes them removable and makes their
removal countable.
