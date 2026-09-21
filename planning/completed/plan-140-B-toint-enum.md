# plan-140-B: `toInt(ENUM)` returns the member's 0-based index

Last updated: 2026-09-20
Effort: medium (1h–2h)
Depends on: plan-140-A

`toInt(e)` for any enum value `e` (user-declared, imported from a package, or
a built-in package's enum such as `app::Mode`) returns an `Integer`: the
member's 0-based position in its `ENUM` declaration. It never fails, and it
compiles to a register move of the value the enum already holds. Integer →
enum stays a user-side lookup (`collections::get(allColors, i)` over a
`List OF Color`), as the `examples/brogue` port needs.

The single behavioral outcome:

```basic
ENUM Color
  Red, Green, Blue
END ENUM
' prints 0, 1, 2 — and toInt(Color.Blue) needs no TRAP
```

References:

- plan-140-A — the `TypeKinds` seam this sub-plan turns on (§3 there).
- `src/codegen/builtins/general/mod.rs:397` — the `TO_INT` resolver arm;
  `:265` — `expected_arguments(TO_INT)`; `func_to_int.rs` — the man page.
- `src/codegen/engine/convert/builder_conversions.rs:16` — `lower_to_int`; its
  `Byte`/`Scalar` register-move branch (`:24`) is the precedent mirrored here.
- `src/codegen/engine/analysis/module_analysis.rs:1109` — the codegen
  fallibility census (`toInt(Byte)`/`toInt(Scalar)` infallible).
- `src/docs/spec/language/18_builtin-functions.md:20` (the `toInt` row) and
  `:123` (the gap-fill override rule); `04_types.md` §4.5 Enums.
- `.ai/compiler.md` — fixture rules (every overload: valid + invalid fixture)
  and `.ai/man-content.md` (man page standard).

## Prerequisites

See plan-140-A §Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-140-A complete (archived) | `ls planning/completed/plan-140-A-*` → one file | MET (2026-09-20: `planning/completed/plan-140-A-enum-kind-seam.md`; A's build + gate rows re-measured at A's end: 0 diffs) |

If plan-140-A is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- `toInt(<enum value>)` type-checks as `Integer` in source and in package IR,
  is infallible, and at runtime returns the declaration index. Proven by a new
  `tests/rt-behavior/general/toInt_enum` fixture whose `.run` golden shows
  `0 1 2` for a three-member user enum and the expected index for an imported
  enum.

### Non-goals (explicit constraints)

- **No Integer → enum conversion.** `toString(enum)` is plan-140-C, which
  depends on this sub-plan.
- **No explicit member values** (`NO_LAYER = -1`). The index is the declaration
  position, full stop.
- **No change to enum representation, package metadata or `type_sig_hash`.**
  The ordinal is already what the value holds and what the ABI hash encodes
  (`src/docs/spec/package/03_metadata-encoding.md:196`: "enums as their member
  names + ordinals … Reordering … enum members … changes the hash").
- `toInt` stays overridable for every non-enum type it rejects today. A user
  `FUNC toInt(m AS Cash)` over a **record**
  (`tests/rt-behavior/functions/func_override_toint_user`) keeps dispatching to
  the override.

## 2. Current State

- `toInt(<enum>)` is rejected: `toInt(Layer.Gas)` →
  `TYPE_CALL_ARGUMENT_MISMATCH` (measured 2026-09-20 with a scratch project in
  `/tmp/inplace_probe`; `toString(Layer.Gas)` is rejected the same way).
- Enum values are held inline as their 0-based ordinal (plan-140-A §2,
  "Verified properties").
- `lower_to_int` already has an infallible move path for `Byte`/`Scalar`
  (`builder_conversions.rs:24`), and the codegen fallibility census exempts
  exactly those two (`module_analysis.rs:1109`).
- The IR-level census (`src/ir/fallible.rs:80`, `call_is_fallible`) is
  name-keyed for `toInt`: its arg-dependent rule list
  (`builtins/mod.rs:300`, `inline_builtin_arg_fallibility_rule`) holds only
  `toString` and `replace`. So `toInt(Byte)` is already *fallible* at the IR
  level (an over-approximation) while codegen treats it as infallible.
  `toInt(enum)` inherits the same pairing (§Open Decisions).

### Measured populations

| What | Count | Command |
|---|---|---|
| Goldens quoting `toInt`'s expected-overload list (will change) | 1 — `tests/syntax/general/toInt_invalid/golden/build.log` | `grep -rl 'expected String\[, Integer\], Byte, Float, Fixed, Money, or Scalar' tests src \| wc -l` → 1 |
| User `FUNC toInt(` declarations in the tree | 1 — `func_override_toint_user`, over a record (unaffected) | `grep -rlE '^\s*(PUBLIC \|PRIVATE \|EXPORT )?FUNC toInt\s*\(' --include='*.mfb' --exclude-dir=target . \| wc -l` → 1 |
| User `toInt` overrides over an **enum** (would be shadowed) | 0 | same grep, read → the one hit takes `Cash`, a `TYPE` |

### Verified properties

- The override precedence rule is "built-in authoritative for the types it
  supports" (spec `18_builtin-functions.md:123`, monomorph
  `resolve_general_builtin_override`). So once the built-in accepts enums, an
  enum override is dead code. Census: 0 such overrides in the tree (above).
  Out-of-tree code cannot be measured (§Open Decisions).

## 3. Design Overview

Four small changes on top of plan-140-A's seam:

1. **Resolver** (`general/mod.rs:397`): the `TO_INT` arm also accepts one
   argument for which `kinds.is_enum(&arg_types[0])`, returning `Integer`.
   `expected_arguments(TO_INT)` becomes
   `"String[, Integer], Byte, Float, Fixed, Money, Scalar, or an enum"`.
2. **Typing fallback**: the typing sites left on `NoTypeKinds` by plan-140-A
   must still type `toInt(enum)` as `Integer`. Where such a site yields no type
   for a rejected `general` call, it falls back to
   `general::nominal_return_type(name)` (`general/mod.rs:310`, `TO_INT →
   Integer`). That is sound because acceptance was already decided by
   `ir::shape`/`ir::verify`; typing only needs the result type.
   `type_utils.rs:60` already does this (`.or_else(call_return_type)`); the
   Phase 1 fixture shows which of the others need it.
3. **Lowering** (`builder_conversions.rs:24`): extend the `Byte | Scalar` move
   branch to `|| self.type_model.is_enum_type(&value.type_)`. The value already
   is the ordinal, so this is a register move with no parse, no error path and
   no data object.
4. **Codegen fallibility** (`module_analysis.rs:1109`): the 1-arg `toInt`
   verdict is also infallible when the argument's static type is an enum. The
   function there has only `types`/`fields` maps; Phase 2's task names the enum
   source it uses (the module's `TypeModel` or an enum-name set passed in).

**Correctness gate: runtime behavior, not byte-identity.** This plan changes
behavior on purpose.

- Expected diffs: `tests/syntax/general/toInt_invalid/golden/build.log` (the
  expected-overloads string) and the new fixtures' goldens.
- Every other golden must be byte-identical. A diff anywhere else is a bug to
  root-cause from one fixture's dump.

**Where the risk concentrates:**

- **Override precedence:** a record override must still win. The fixture in
  Phase 1 pins both directions.
- **Inline TRAP on `toInt(enum)`:** `builder_values.rs:2280` routes every
  inline-TRAPped `toInt` to `lower_inline_conversion_raw`. It must take the
  infallible path for an enum the way it does for `Byte`. Covered by a fixture
  case.

Rejected alternatives:

- **A separate built-in (`ordinal(e)` / `enumIndex(e)`).** It avoids the
  override-precedence question, but adds a new always-in-scope name for what
  every other conversion calls `toInt`, and the user asked for
  `toInt(Layer.Gas)`.
- **Make `toInt` accept any declared type in lenient mode.** It would silently
  shadow record overrides like `func_override_toint_user` through monomorph's
  gap-fill check (plan-140-A §2, "Dispatch").

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. `- [~]` for partial, with what remains. Moot tasks are
> struck through with evidence, never deleted. Fill `Commit:` the moment a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — Failing fixtures first

Lands RED fixtures, so every later phase has a runtime check. The goldens are
authored by hand from the specification, not captured from a build.

- [x] `tests/rt-behavior/general/toInt_enum/` (project.json per
      `func_override_toint_user`'s): a user `ENUM Color Red, Green, Blue`;
      prints `toInt` of each member (`0`, `1`, `2`). It also covers:
      `LET n = toInt(c)` for an inferred binding; `toInt` of a value read back
      from a `List OF Color` and of a record field; `toInt(e)` inside an inline
      `TRAP` (must not trap); Integer → enum through a `List OF Color` lookup
      round-tripping every member; and `toInt(app::Mode.<member>)` or another
      built-in package enum (name it after reading the registry).
      `golden/*.run` hand-written. (Built-in enum: `datetime::Weekday`,
      Monday..Sunday → 0..6, `datetime/mod.rs:405`.)
- [x] `tests/rt-behavior/general/toInt_enum_package/`: an enum declared
      `PUBLIC` in a second source file and one imported from a local package
      fixture (mirror an existing package fixture under
      `tests/rt-behavior/packages/`), proving the package-IR path.
- [x] `tests/rt-behavior/functions/func_override_toint_enum_precedence/`: a
      record override `FUNC toInt(m AS Cash)` **and** an enum in one program;
      `toInt(cash)` → the override's value, `toInt(Color.Blue)` → `2`.
- [x] `tests/syntax/general/toInt_invalid/src/main.mfb`: add
      `toInt(Color.Red, 16)` (the `base` form stays `String`-only) and a
      union-typed argument (a `UNION` is not an enum). Hand-edit the
      golden's expected lines.

Acceptance: the new fixtures fail for the right reason.
  Check: `scripts/test-accept.sh target/release/mfb target/accept-actual 'toInt_enum*' 'func_override_toint_enum_precedence' 'toInt_invalid'`
  → the rt-behavior fixtures fail with `TYPE_CALL_ARGUMENT_MISMATCH` on the
  `toInt(<enum>)` lines; `toInt_invalid` differs only on the new lines and on
  the expected-overloads string (est. 2 min).
  Result: `4 mismatch(es) (4 test(s) ran)`; every rt-behavior failure is
  `TYPE_CALL_ARGUMENT_MISMATCH` on a `toInt(<enum>)` line (`Color`,
  `datetime.Weekday`, the package's `Suit` at `lib.mfb:11`); `toInt_invalid`
  differs only in the expected-overloads string.
Commit: 643866b0e

### Phase 2 — Resolve, type, lower, and mark infallible

- [x] `src/codegen/builtins/general/mod.rs:397`: `TO_INT` accepts one enum
      argument via `kinds.is_enum`; `expected_arguments(TO_INT)` (`:265`) gains
      `", or an enum"`; unit tests: `rt_kinds(TO_INT, &["Color"], enum_oracle)`
      → `Integer`, same with a non-enum oracle → `None`, and
      `(Color, Integer)` → `None`. (`resolve_to_int_enum`, plus
      `only_to_int_consults_the_kind_oracle`, which replaces plan-140-A's
      `..._does_not_consult_the_kind_oracle_yet` — see B-C2;
      `cargo test --bin mfb codegen::builtins::general` → 28 passed.)
- [x] Typing fallback in the `NoTypeKinds` sites the Phase 1 fixture shows
      untyped (start with `ir/lower.rs:3959`): on `None` for an `is_general_call`
      name, fall back to `general::nominal_return_type`. List each site touched
      in the commit message. (Only `ir/lower.rs:3959` needed it, measured:
      with it on `NoTypeKinds` the `toInt_enum` build fails at `main.mfb:18`
      `TYPE_CALL_ARGUMENT_MISMATCH` on the enclosing `toString(toInt(..))`; with
      it wired every fixture passes. Wired through a `TypeIndex` oracle, not the
      nominal fallback — B-C1.)
- [x] `src/codegen/engine/convert/builder_conversions.rs:24`: the move branch
      also takes an enum-typed value (`self.type_model.is_enum_type`).
- [x] ~~`src/codegen/engine/value/builder_values.rs:2280`: the inline-TRAP raw
      path treats `toInt(<enum>)` as infallible, whatever `toInt(Byte)` does
      there.~~ — moot: that path calls `lower_inline_conversion_raw` →
      `lower_to_int` (`builder_values.rs:3018`), which now takes the move
      branch for an enum exactly as for `Byte`, so no error exit exists to
      capture. Evidence: `toInt_enum`'s `toInt(Color.Green) TRAP(e)` prints `1`
      and never `trapped` (`.run` golden line 7, passing).
- [x] `src/codegen/engine/analysis/module_analysis.rs:1109`: the 1-arg `toInt`
      verdict is infallible for an enum argument. (Enum source: `FieldTypes`
      now carries the enum table, filled from the builder's `TypeModel`, which
      is built before the string pre-pass — B-C3.)

Acceptance: the Phase 1 fixtures pass.
  Check: `cargo build --release && scripts/test-accept.sh target/release/mfb target/accept-actual 'toInt*' 'func_override_toint*' 'bug155_toInt_named_args' 'scalar-conversions-rt' 'codegen-conversion-edges-rt'`
  → all pass (est. 3 min; the listed fixtures are every existing `toInt`
  fixture plus the new ones, per `grep -rl toInt tests/rt-behavior/general/*/src/main.mfb`).
  Result: `acceptance tests passed (13 test(s) ran)` (the three new fixtures
  listed as run). Whole corpus: `artifact-gate [all]: 1470 tests, 1645
  build(s), 2072 golden(s) checked, 0 diff(s)` — no golden outside this
  plan's changed (`toInt_invalid` was hand-edited in Phase 1).
Commit: efbbf57d5

### Phase 3 — Docs and spec

- [x] `src/codegen/builtins/general/func_to_int.rs`: DESC gains a short
      paragraph. From an enum value, `toInt` returns the member's position in
      its `ENUM` declaration, counting from 0; it never fails; reordering the
      members changes the numbers. EX gains an enum example with its printed
      output. Say "you get the position" and never "ordinal"/"tag"
      (`.ai/man-content.md` bans compiler vocabulary). The `value` parameter's
      description mentions enums.
- [x] `src/docs/spec/language/18_builtin-functions.md:20`: add "any enum (the
      member's 0-based declaration index; infallible)" to the `toInt` row, and
      in the `:123` override paragraph note that enums are among the types the
      built-in supports, so a `toInt` override over an enum is never selected.
      Cite `[[src/codegen/builtins/general/mod.rs:resolve_call]]` and
      `[[src/codegen/engine/convert/builder_conversions.rs:lower_to_int]]`.
- [x] `src/docs/spec/language/04_types.md` §4.5: one sentence that a member's
      `toInt` is its declaration index, pointing at §18.

Acceptance: the man page renders and its examples run; the spec builds and its
citations resolve.
  Check: `scripts/man-run-examples.sh general --run` → every `toInt` example
  prints its documented output (est. 2 min).
  Check: `cargo test --bin mfb spec && scripts/spec-census.sh --citations` →
  pass, 0 dangling (est. 3 min).
  Check: `scripts/man-census.sh --memory-scope` → 0 unclassified hits (est. 1 min).
  Result: `man-run-examples.sh general --run` → `examples: 35 built: 35 ran:
  35 not run: 0 failed: 0`; `toInt example 3` prints `0`, `2`, `TRUE` as
  documented. `cargo test --bin mfb spec` → 43 passed; `spec-census.sh
  --citations` → MISS-PATH 0, MISS-LINE 0, MISS-SYMBOL 0.
  `MFB=./target/release/mfb scripts/man-census.sh --memory-scope` →
  `unclassified memory-vocabulary hits: 0`. (The §4.5 sentence points at
  §18.1, where the `toInt` row lives; the plan's "§18" is that section.)
Commit: 6daf882b7

## Validation Plan

- Tests: the Phase 1 fixtures cover valid use of the new overload (user,
  imported and built-in enums; inferred binding; list element; record field;
  inline TRAP; round trip), invalid use (`base` with an enum, a union argument),
  and override precedence in both directions. Unit tests in `general/mod.rs`
  cover the resolver arm.
- Coverage check: `scripts/test-accept.sh … 'toInt*'` must list the three new
  fixtures as run (not skipped). A fixture with no golden is not in the
  denominator.
- Runtime proof: `examples/brogue` can replace its tile-type Integer constants
  with an `ENUM` and index `tileCatalog` by `toInt(t)`. Not part of this plan;
  the `toInt_enum` fixture's `.run` golden is the proof here.
- Doc sync: Phase 3.
- Final gate (once, after Phase 3):
  `scripts/test-accept.sh target/debug/mfb target/accept-actual` → all pass,
  and `bash scripts/artifact-gate.sh target/release/mfb all` → diffs only in
  `tests/syntax/general/toInt_invalid` and the new fixtures (est. per the
  harnesses; the full acceptance suite is required after compiler work by
  `.ai/compiler.md`). plan-140-C runs the same gate again at its end.

## Open Decisions

- **Existing out-of-tree `toInt` overrides over an enum become dead code.**
  Recommended: accept it, and state it in the spec's override paragraph (Phase
  3). This is the same rule `toString(42)` already follows, and the in-tree
  census is 0. vs. keep enum overrides winning, which would make the built-in
  non-authoritative for a type it supports, a new exception to the gap-fill
  rule.
- **IR-level fallibility for `toInt(enum)`.** Recommended: leave it name-keyed
  (fallible at the IR level, infallible in codegen), exactly as `toInt(Byte)`
  and `toInt(Scalar)` are today, and file a follow-up to make all three
  arg-aware in `inline_builtin_arg_fallibility_rule`. vs. fix it here for enums
  only, which would leave `Byte`/`Scalar` inconsistent with enums.

## Corrections

- **B-C1 (Phase 2, typing):** §3 step 2 proposed the name-keyed
  `nominal_return_type` fallback at `ir/lower.rs:3959`. That would also type an
  INVALID call (`toInt(TRUE)`) as `Integer` and drop the
  `TYPE_UNKNOWN_VALUE` follow-on the `toInt_invalid` golden pins. The site
  instead passes `ir::lower`'s `TypeIndex` (which holds declared and imported
  enums) as the kind oracle: exact, and no invalid call changes. No other
  typing site needed a change (Phase 2 task line has the measurement).
- **B-C2 (tests):** plan-140-A's unit test pinned "no arm consults the oracle
  yet", a property B ends by design. It is replaced by
  `resolve_to_int_enum` plus `only_to_int_consults_the_kind_oracle`, which keeps
  A's guarantee for every other built-in.
- **B-C3 (Phase 2, fallibility census):** §3 step 4 left the census's enum
  source open. `module_analysis::value_may_return_invalid_format` only sees the
  string pre-pass's `types`/`fields`, and that pre-pass ran before the
  `TypeModel` existed. `FieldTypes` became a struct that also carries each
  enum's members in declaration order (`FieldTypes::with_enums_of`), and
  `builder/mod.rs` now builds the type model first and hands it to
  `string_symbols`. plan-140-C's pre-pass reads the same table, which settles
  its UNVERIFIED row.
- **B-C4 (Final gate):** the per-letter full `test-accept.sh` run is
  consolidated into plan-140's single end-of-plan run (plan-140-C's final
  gate). The whole-corpus artifact gate above already ran on B's tree.

## Summary

The engineering risk is precedence, not arithmetic. The conversion itself is a
register move of a value the enum already holds. What must stay exactly right
is that only a genuine enum takes the new path: a record override keeps its
dispatch, a union argument is still rejected, and the `base` form stays
`String`-only. All of those are pinned by fixtures written before the change.
Untouched: enum representation, package metadata and hashing, and Integer →
enum. `toString(enum)` follows in plan-140-C.
