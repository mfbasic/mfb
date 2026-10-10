# plan-172-N: An `ENUM` defaults to its first variant

Last updated: 2026-10-10
Effort: medium (1h–3h)
Depends on: plan-172-A

Appended during execution of plan-172-A (its Open Decision 1). The owner's answer was
that a shape is built from a default-initialised record refined with `WITH`, and that
"all fields have defaults in the language". Measured, that holds for every field type
the shape records use except `ENUM`: `MUT p AS canvas::Paint` is rejected. This letter
makes it hold.

Behavioral outcome: `MUT r AS canvas::Circle` compiles, every field of `r` is its zero
(`paint.cap` is `CapStyle`'s first variant), and
`WITH r { position := …, radius := …, paint := … }` is a usable shape.

References:

- `planning/plan-172-A-canvas-shapes-premises.md` — Open Decision 1 and its ANSWER, with
  the probe `/tmp/plan-172-A/d1.mfb`.
- `src/docs/spec/language/04_types.md` §4.10 "Default Values" — the rule this changes.
- `src/ir/verify/resources.rs:is_defaultable` — the one predicate that enforces it.
- `src/codegen/memory/value/builder_value_semantics.rs:lower_default_value_inner` — the
  code generator's defaults, which already has an `ENUM` arm.
- `.ai/compiler.md` (Hard Completion Gate), `.ai/specifications.md`.

## Prerequisites

The plan-wide gate is in `plan-172-A` §Prerequisites. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| Open Decision 1 of 172-A is answered | `grep -c 'ANSWER: no constructor functions' planning/plan-172-A-*.md planning/completed/plan-172-A-*.md 2>/dev/null \| grep -c ':1'` → 1 | MET — measured 1 |
| The code generator already materialises an `ENUM` default as ordinal 0 | `grep -c 'An enum value IS its ordinal at run time' src/codegen/memory/value/builder_value_semantics.rs` → 1 | MET — measured 1 |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**

## 1. Goal

- A bare `ENUM` type is defaultable; its default is its first declared variant.
- A record is defaultable when every field is, as before, so a record holding an `ENUM`
  field now is.
- `canvas::Paint`, `canvas::MouseEvent` and every canvas shape record with no `UNION` or
  `RES` field can be declared with `MUT` and no initializer.

### Non-goals (explicit constraints)

- **A `UNION` gets no default.** There is no zero "which variant", and the owner's
  examples (a `TopLeft` bind, a square corner, `use*` false) are all enum, number and
  Boolean zeros. `AutoPaint` and `Picture` hold a `UNION` and stay non-defaultable.
- **A `RES` handle, a function value, a thread and a `STATE` type get no default.**
  `canvas::Text` holds a `RES` font and stays non-defaultable.
- **No new syntax.** A constructor still names every field (172-A P6).
- **No change to what an existing valid program does.** This only accepts programs that
  were rejected.

## 2. Current State

`is_defaultable` returns `false` for any type in `self.enums`. The code generator does
not agree with it: `lower_default_value_inner` has an `ENUM` arm that emits ordinal 0,
written for the temporary an inline `TRAP` binds. So the front end is the only thing in
the way, and the default it would produce is already defined and already exercised.

### Measured populations

| What | Count | Command |
|---|---|---|
| Sites that decide defaultability | 1 | `grep -rn 'fn is_defaultable' src \| wc -l` → 1 |
| Fixtures whose golden names the rule | 2 | `grep -rl TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE tests --include=build.log \| wc -l` → 2 |
| Lines of `mut-default-eligibility-invalid` that become valid | 2 of 5 | `enumValue AS Flag` (line 26) and `nestedEnum AS WrappedFlag` (line 29); the `UNION`, function and recursive-record lines stay errors |

### Verified properties

- **An `ENUM`'s run-time value is its ordinal, and ordinal 0 is its first variant.**
  VERIFIED by reading `lower_default_value_inner`'s enum arm and its comment.
- **UNVERIFIED — that nothing else (the monomorphiser, the `.mfp` decoder's verifier
  path, `SELECT CASE` exhaustiveness) assumed an uninitialised enum binding cannot
  exist.** Phase 1's run-time fixture is the test.

## 3. Design Overview

One predicate changes: the `self.enums.contains_key(type_)` test leaves
`is_defaultable`'s rejecting arm and becomes an accepting one. Everything else follows
from the rule records already have.

**Gate class: a newly accepted program.** No existing program's output changes, so no
run-time golden may move. One *diagnostic* golden moves, deliberately, and the four
answers for editing it are:

1. *When and why it was written:* `tests/syntax/types/mut-default-eligibility-invalid`
   pins spec §4.10's list of non-defaultable types (find its commit with `git log
   --diff-filter=A -- tests/syntax/types/mut-default-eligibility-invalid`).
2. *What it protects:* that `MUT x AS T` with no initializer is rejected for a `T` with
   no default.
3. *Who else depends on it:* spec §4.10 and `05_bindings-and-scope.md`; both are updated
   here.
4. *Proof the two enum lines are wrong:* the owner's ruling recorded in 172-A Open
   Decision 1 — the rule itself is changed by its owner, and the code generator already
   defines the value. Only the two disproved lines' diagnostics go; the fixture keeps
   its three other errors and stays an `-invalid` fixture.

### Rejected alternatives

- **Constructor functions for the shapes.** The owner's "no" in 172-A Open Decision 1.
- **Letting a constructor omit a field.** New syntax, a second way to say what
  `MUT` + `WITH` already says, and not asked for.
- **Defaulting a `UNION` to its first variant's default.** Not asked for, and a
  defaulted `AutoPaint.shape` would be a zero-size rectangle drawn nowhere — a value
  with no use that hides a forgotten field.

## Compatibility / Format Impact

- **Newly accepted:** `MUT x AS E` for an `ENUM` `E`, and for any record whose only
  obstacle was an `ENUM` field.
- **Unchanged:** every program that compiled before; the `.mfp` format; codegen.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. **An unticked box means NOT DONE.**

### Phase 1 — The rule, its tests and its spec

- [x] `src/ir/verify/resources.rs:is_defaultable`: an `ENUM` is defaultable.
      → `cargo test --bin mfb defaultable` → `19 passed; 0 failed`.
- [x] Unit cases in `src/ir/verify/tests.rs`: a bare enum binding and a record with an
      enum field are accepted; a `UNION` binding and a record with a `UNION` field are
      still `TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE`.
- [x] ~~Extend `tests/rt-behavior/types/default-values-rt`~~ — done as a new fixture,
      `tests/rt-behavior/types/enum-default-first-variant-rt` (see Corrections): a defaulted enum equals its
      first variant; a defaulted record's enum field does; `WITH` over the defaulted
      record sets another field and leaves the enum at its first variant.
- [x] `tests/syntax/types/mut-default-eligibility-invalid`: answer §3's four questions
      from evidence in the commit message, then remove only the two enum diagnostics
      from `build.log`, after the full suite has run.
- [x] `src/docs/spec/language/04_types.md` §4.10: the table row, the prose list and the
      predicate paragraph say an `ENUM` defaults to its first variant;
      `src/docs/spec/language/05_bindings-and-scope.md` where it restates the list.
- [x] A canvas proof: `/tmp/plan-172-A/d1.mfb` builds and prints `default cap is ordinal
      0` and `default kind is None`.

Acceptance: the unit cases, the run-time fixture and the canvas probe pass, and no
golden other than the one named moves.
  Check: `cargo test --bin mfb defaultable` → pass; `scripts/artifact-gate.sh
  target/release/mfb types` → only `mut-default-eligibility-invalid` differs, in the two
  lines named (est. 10 min).
  → `sh /tmp/plan-172-A/run_d1.sh target/release/mfb` printed `default cap is ordinal 0`
    and `default kind is None`. Full `scripts/test-accept.sh` at `fc530a22e`
    (`/tmp/p172-suite-accept.log`, 1678 tests): three fixtures differ — the one named,
    and two under `syntax/resources` (see Corrections).
Commit: fc530a22e (rule, unit cases, spec §4.10); the golden, the fixture's module and
`STATE`-record cases and the remaining doc sync in the commit after it

## Validation Plan

- Tests: unit cases on the predicate; the extended run-time fixture; the narrowed
  invalid fixture.
- Runtime proof: the canvas probe above.
- Doc sync: spec §4.10 and `05_bindings-and-scope.md`; `mfb spec language types`
  rendered and read.
- Final gate: none in this letter; 172-M.

## Open Decisions

None. The one decision is the owner's, recorded in 172-A.

## Corrections

- **A new fixture, not an extension of `default-values-rt`.** That fixture exists to
  reach codegen's defaults through an inline `TRAP` and says so; adding `MUT` cases
  would have re-baselined its 85 KB of `.ast` / `.ir` for a different purpose. Its
  comment, which said the front end rejects an enum, is corrected in place (line-neutral).
- **`rejects_mut_enum_not_defaultable` pinned the old rule.** Written in `b20ab6b56`
  (plan-12 wave 2, one case per `is_defaultable` branch). It is now
  `mut_single_variant_enum_is_defaultable`, asserting acceptance of the same program.
- **The scoped check was swapped for the full suite**, run at `fc530a22e` in a detached
  worktree (`/tmp/p172-suite`) because the golden edit needed it. It took about 100
  minutes on a loaded machine.
- **Two more fixtures moved, and they stay red: `syntax/resources/resource-state-invalid`
  and `resource-return-state-invalid`.** Both use a bare enum as their example of an
  illegal `STATE` type. `STATE`'s check is this same predicate, so a bare enum now passes
  it and then fails in native lowering exactly as `STATE Integer` and `STATE Float`
  already did on main. That is **bug-1013**, a pre-existing bug this letter widened by
  one type. The owner ruled (2026-10-10) that a `STATE` must be a record, and that
  plan-172 continues without fixing it; the two fixtures are correct as written and go
  green, unedited, when bug-1013 lands. Their goldens are not touched here.
- **Spec and man statements this letter also corrected:** §4.5 (an enum has a default),
  `05_bindings-and-scope.md`, `15_resource-management.md` (an enum field does not block
  a `STATE` record), and `mfb man variable` ("no enum or handle fields" → "no handle
  fields").

## Summary

The code generator has always known what a defaulted enum is; only the front end
refused to ask. Lifting that one refusal is what turns the owner's "use `WITH`" from
true for a `Group` into true for every shape that has a zero.
