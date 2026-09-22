# bug-679: `toInt(<enum>)` is classified fallible, so a total program reads as error-propagating

Last updated: 2026-09-22
Effort: medium (1h–2h)
Severity: LOW
Class: Footgun

Status: FIXED
Regression Test: `tests/rt-behavior/general/toInt_enum` (extend), plus a
`cargo test --bin mfb` unit case on `inline_builtin_is_infallible`

`toInt(e)` for an enum `e` cannot fail: plan-140-B specifies it as "a register
move of the value the enum already holds", and the resolver types it `Integer`
unconditionally. The infallibility census does not know that. `toInt` is absent
from [`inline_builtin_is_infallible`]'s name-keyed infallible set — correctly,
because `toInt(<String>)` raises on a bad parse and `toInt(<Float>)` on
overflow — and nothing narrows the verdict for the enum overload, so every
`toInt(<enum>)` call is treated as fallible.

Nothing miscomputes: the fallibility verdict is a deliberate **safe
over-approximation** (`src/ir/fallible.rs:11` — "a call is fallible unless it is
*proven* otherwise"), so the failure mode is not a wrong answer but a wrong
*description*, in two places an author actually reads:

- `mfb audit` reports the whole call chain above a `toInt(<enum>)` as fallible.
- An inline `TRAP` on a `toInt(<enum>)` is silently accepted instead of raising
  `TYPE_INLINE_TRAP_DEAD_HANDLER` — the author is told their dead handler is
  live, which is the inverse of the warning's purpose.

`toString(<enum>)` is correct on both counts (plan-140-C), so the two halves of
the same enum bridge disagree. **The single correct behavior a fix produces:**
`toInt(<enum value>)` is classified infallible, exactly as `toString(<enum
value>)` is — while every other `toInt` overload stays fallible.

References:

- `planning/completed/plan-140-B-toint-enum.md:8-10` — `toInt(<enum>)` returns
  the ordinal and "compiles to a register move of the value the enum already
  holds"; it declares no error.
- `planning/completed/plan-140-C-tostring-enum.md` — the sibling that is
  classified correctly, and whose `TO_STRING` arm reads the plan-140-A
  `TypeKinds` seam.
- `bugs/completed/bug-486-tostring-bytes-inline-trap-not-caught.md` — the bug
  that made this census overload-aware in the first place, in the opposite
  direction (name infallible, one overload fallible). This bug is its mirror
  (name fallible, one overload infallible) and reuses its machinery.
- Found while refactoring `examples/dungeon/src/main.mfb` onto `toInt`/
  `toString`/enum `MATCH` (commit `df455c0e`).

## Failing Reproduction

```
$ mkdir -p /tmp/tointtrap/src && cat > /tmp/tointtrap/src/main.mfb <<'EOF'
IMPORT io

ENUM Color
  Red, Green, Blue
END ENUM

FUNC main AS Integer
  LET n AS Integer = toInt(Color.Green) TRAP(e)
    RECOVER -1
  END TRAP
  LET s AS String = toString(Color.Green) TRAP(e)
    RECOVER "?"
  END TRAP
  io::print(toString(n) & s)
  RETURN 0
END FUNC
EOF
$ mfb build /tmp/tointtrap          # console-mode project.json
```

- Observed (2026-09-22, macos-aarch64): exactly one
  `TYPE_INLINE_TRAP_DEAD_HANDLER` warning, on **line 11** (`toString`). The
  `toInt` handler on line 8 is accepted in silence.
- Expected: the same warning on **both** lines 8 and 11 — neither call can fail,
  so both handlers are dead.

The audit half, same day, from `/tmp/tointfall` (`FUNC ordinalOf(c AS Color) AS
Integer / RETURN toInt(c)` beside `FUNC nameOf(c AS Color) AS String / RETURN
toString(c)`):

```
$ mfb audit /tmp/tointfall
Control flow:
  ordinalOf at src/main.mfb:7 (fallible)
    fallible call toInt at src/main.mfb:8 -> return
  main at src/main.mfb:15 (fallible)
```

- Observed: `ordinalOf` fallible; `nameOf` absent from the list.
- Expected: neither listed — both are total.

Contrast cases that are correct today and must stay correct:

| Call | Verdict today | Correct? |
| --- | --- | --- |
| `toString(<enum>)` | infallible | ✓ |
| `toString(<List OF Byte>)` | fallible (bug-486) | ✓ |
| `toInt(<String>)` | fallible (bad parse) | ✓ |
| `toInt(<Float>)`, `toInt(<Money>)` | fallible (overflow) | ✓ |
| `toInt(<enum>)` | **fallible** | **✗ — this bug** |

Blast-radius scale, measured on `examples/dungeon/src/main.mfb` at commit
`df455c0e` (`mfb audit examples/dungeon | grep -c "(fallible)"`): **38**
fallible functions, where the same file before it used `toInt` reported **9**.
The single `toInt(f)` inside `turned` accounts for all 29, through
`turned` → `clockwise`/`counterclockwise`/`opposite` → the rest of the program.

## Root Cause

`builtins::inline_builtin_is_infallible` (`src/codegen/builtins/mod.rs:399`) is
the one census behind both symptoms — `ir::fallible::Fallibility::call_is_fallible`
(`src/ir/fallible.rs:81`) and the dead-handler check in
`ir::verify::resources` (`src/ir/verify/resources.rs:995`) both call it.

It decides in three steps, and `toInt` falls through all of them:

1. `arg_type_makes_inline_builtin_fallible` (`:268`) — the bug-486 hook. It only
   ever *removes* infallibility from an otherwise-infallible name. There is no
   inverse hook that *grants* infallibility to one overload of an otherwise
   fallible name.
2. `native_member_declares_error` — registry data for migrated common-native
   members. `toInt` is a general built-in, not a native member, so this does not
   answer.
3. `matches!(target, "len" | "toString" | "typeName")` (`:412`) — the name-keyed
   infallible set. `toInt` is correctly absent, because most of its overloads
   really can fail.

So the function returns `false`, and `call_is_fallible` falls through to its
final `true` (`src/ir/fallible.rs:85`) — the documented over-approximation.

Why the enum overload cannot simply be added to step 1's table: an enum is not
its own `ParameterType`. It arrives as an opaque `ParameterType::Named`
(`src/types.rs:155` has no enum variant), and `src/codegen/builtins/mod.rs:476`
says so outright — "the resolver cannot tell an enum from a record by the type
alone; a site that holds the declarations answers for it". Deciding this needs
the plan-140-A oracle, `builtins::TypeKinds::is_enum`
(`src/codegen/builtins/mod.rs:483`), which `inline_builtin_is_infallible` does
not take.

`toString` is immune precisely because it never needs the oracle: it is
infallible under step 3 for *every* type including enums, so plan-140-C had to
add nothing here.

## Goal

- `inline_builtin_is_infallible` answers `true` (infallible) for `toInt` with a
  single enum argument, and `false` for every other `toInt` overload, with the
  enum verdict taken from a `TypeKinds` oracle, never from a name or spelling
  heuristic.
- `mfb audit` on the reproduction lists neither `ordinalOf` nor `nameOf`.
- An inline `TRAP` on `toInt(<enum>)` warns `TYPE_INLINE_TRAP_DEAD_HANDLER`,
  matching `toString(<enum>)`.
- `examples/dungeon/src/main.mfb` returns to 9 fallible functions.

### Non-goals (must NOT change)

- **No change to any other `toInt` overload.** `toInt(<String>)`,
  `toInt(<Float>)`, `toInt(<Fixed>)`, `toInt(<Money>)`, `toInt(<Byte>)`,
  `toInt(<Scalar>)` and the two-argument radix form stay fallible. Marking the
  name infallible outright would delete the only guard between a bad parse and
  a dead process — the exact hazard bug-486 was filed for, in reverse.
- **No enum variant added to `ParameterType`.** `src/codegen/builtins/mod.rs:476`
  pins that as deliberate (it keys the registry and `TypeModel`, `Hash`/`Eq`
  pinned). The oracle is the sanctioned answer.
- **No spelling heuristic.** `TypeKinds::is_enum`'s own doc requires the answer
  come from the declaration's kind, never from "is a declared type".
- **No change to enum representation, package metadata or `type_sig_hash`**
  (inherited from plan-140-B §Non-goals).
- **Tempting wrong fix, forbidden:** silencing the dungeon symptom by rewriting
  `turned` to avoid `toInt` (e.g. restoring a hand-written `facingIndex`). That
  hides the defect in one caller and leaves every other `toInt(<enum>)` user
  with the same wrong description.

## Blast Radius

Call sites of the census, from `grep -rn "inline_builtin_is_infallible" src/`:

- `src/ir/fallible.rs:81` (`call_is_fallible`) — **fixed by this bug.** It is
  the source of the `mfb audit` symptom. An oracle is already reachable:
  `fallible::analyze` takes a `LowerContext`, and `src/ir/lower.rs:5863` impls
  `TypeKinds for TypeIndex`.
- `src/ir/verify/resources.rs:995` (dead-handler check) — **fixed by this bug.**
  Source of the `TYPE_INLINE_TRAP_DEAD_HANDLER` symptom. An oracle is reachable
  here too: `src/ir/verify/compat.rs:102` impls `TypeKinds for TypeEnv`.
- `src/codegen/builtins/mod.rs:227` (inside the raw-support decision) —
  **must be checked, verdict pending Phase 1.** If this path can see a
  `toInt(<enum>)`, the change shifts lowering, not just diagnostics.
- `src/codegen/builtins/crypto/mod.rs:1302,1306` and
  `src/codegen/builtins/mod.rs:1218,1248` — unit-test assertions over crypto and
  constant targets. **Unaffected:** none names `toInt` or passes an enum.

Users of the shifted verdict:

- Any program calling `toInt` on an enum. Measured corpus reach:
  `grep -rn "toInt(" tests/ examples/` — enumerate in Phase 1. Expected to be
  near-empty outside `tests/rt-behavior/general/toInt_enum` and
  `examples/dungeon`, because `toInt(<enum>)` only shipped in plan-140-B.
- A fallible→infallible flip changes lowering: `ir::lower`'s inline-`TRAP`
  desugar emits a plain `Call` instead of `CallResult` + `If ResultIsOk`
  (`src/ir/fallible.rs:3-9`). So `.ir`/`.nir`/`.ncode` goldens for any fixture
  containing `toInt(<enum>)` will move — intended, and the extent must be
  exactly that set.

## Fix Design

Mirror bug-486's hook in the infallible direction, and thread the oracle the
census already lacks:

1. Give `inline_builtin_is_infallible` a `&dyn TypeKinds` parameter (or a
   `_with_kinds` twin beside `resolve_call_return_type_with_kinds`, keeping the
   current signature delegating through `NoTypeKinds` for typing-only sites).
   `NoTypeKinds::is_enum` is `false`, so every unconverted caller keeps today's
   over-approximating answer — safe by construction.
2. Add the narrow rule: `target == "toInt"` and exactly one argument whose
   `is_enum` is true ⇒ infallible.
3. Pass a real oracle at the two sites that need it (`ir::fallible`,
   `ir::verify::resources`), reusing `TypeIndex` and `TypeEnv`.
4. `inline_builtin_fallibility_depends_on_args` (`:318`) must now also answer
   true for `toInt`, or the verify site will pass an empty `arg_types` and never
   reach the new rule.

Rejected alternatives:

- *Add `toInt` to the step-3 name list and subtract the fallible overloads via
  `arg_type_makes_inline_builtin_fallible`.* Rejected: it inverts the safe
  default for a name whose overloads are overwhelmingly fallible, so any
  overload the subtraction table forgets silently becomes an un-trappable
  failure. The over-approximating direction must stay the default.
- *Give `ParameterType` an `Enum` variant.* Rejected by Non-goals; plan-113 and
  `mod.rs:476` pin the current representation.

Expected output shift: `.ir`/`.nir`/`.ncode` goldens only for fixtures calling
`toInt(<enum>)`. Anything else moving is a bug in the change, not a re-baseline.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add a `cargo test --bin mfb` case asserting
      `inline_builtin_is_infallible("toInt", &[<an enum Named>], &<oracle>)` is
      `true` and that `toInt(<String>)` / `toInt(<Float>)` stay `false`. Reuse
      the `ColorIsAnEnum` oracle already in
      `src/codegen/builtins/general/mod.rs:1025`. Confirm it fails today.
- [x] Extend `tests/rt-behavior/general/toInt_enum` with the inline-`TRAP`
      shape, so the missing `TYPE_INLINE_TRAP_DEAD_HANDLER` is a golden diff.
- [x] Enumerate every `toInt(<enum>)` in `tests/` and `examples/`; record the
      exact golden set expected to move in Phase 3.
- [x] Settle the `src/codegen/builtins/mod.rs:227` verdict left open above.

Acceptance: the new test(s) fail for the documented reason; the blast-radius
list has a verdict per site and a named golden set.
Commit: `0692e121` (the unit case and the threading), `f1a21b2b` (the audit case)

### Phase 2 — the fix

- [x] Thread `TypeKinds` into `inline_builtin_is_infallible` (or its
      `_with_kinds` twin), defaulting unconverted callers to `NoTypeKinds`.
- [x] Add the `toInt` + single enum argument rule.
- [x] Add `toInt` to `inline_builtin_fallibility_depends_on_args`.
- [x] Pass real oracles at `src/ir/fallible.rs:81` and
      `src/ir/verify/resources.rs:995`.

Acceptance: Phase 1 tests pass; every contrast-case row in the table above still
holds; nothing in Non-goals changed.
Commit: `0692e121` (the IR census), `f1a21b2b` (the audit census — a SECOND root
cause the document did not identify; see Corrections)

### Phase 3 — regenerate expected outputs + full validation

- [x] Regenerate only the goldens named in Phase 1; diff each and confirm the
      delta is the `CallResult` → `Call` collapse and nothing else.
- [x] `scripts/artifact-gate.sh <exe> all` (full, once) + `test-accept.sh`, the
      latter being the only harness that sees the diagnostic prose change.
- [x] Re-run both reproductions; confirm two warnings, and
      `mfb audit examples/dungeon | grep -c "(fallible)"` → 9.

Acceptance: full gate green; golden deltas are exactly the intended set; both
reproductions behave as Expected.
Commit: —

## Validation Plan

- Regression test(s): the `inline_builtin_is_infallible` unit case, and the
  inline-`TRAP` arm added to `tests/rt-behavior/general/toInt_enum`.
- Runtime proof: `mfb build /tmp/tointtrap` warns on both lines;
  `mfb audit examples/dungeon | grep -c "(fallible)"` returns 9, not 38.
- Doc sync: the `inline_builtin_is_infallible` doc comment (`:379-398`) lists
  the infallible set by name and must gain the `toInt(<enum>)` exception;
  `src/ir/fallible.rs:15-24`'s module doc names `toString` as the sole
  overload-sensitive case and must name `toInt` too.
- Full suite: `scripts/artifact-gate.sh <exe> all`, `scripts/test-accept.sh`,
  `cargo test --bin mfb --tests`.

## Open Decisions

- Signature vs. twin — add the `TypeKinds` parameter to
  `inline_builtin_is_infallible` directly (4 call sites to touch, no silent
  under-approximation left behind) vs. add an `_with_kinds` twin beside
  `resolve_call_return_type_with_kinds` (precedent-matching, but leaves a
  wrong-answer overload of the same name available). **Recommend the direct
  parameter**, since `NoTypeKinds` already makes the migration mechanical and
  the twin is the shape `mod.rs:476` warns about. (§Fix Design)

## Summary

The engineering risk is not the rule — it is three lines — but the **oracle
threading** and the **golden delta**. `NoTypeKinds` makes the threading safe by
construction (an unconverted caller keeps today's answer), so the real work is
proving in Phase 1 that the set of goldens containing `toInt(<enum>)` is small
and known, and in Phase 3 that nothing outside it moved. Runtime behavior,
enum representation, and every non-enum `toInt` overload are untouched.

## Corrections

Four things this document asserted turned out to be wrong. All were found by
measurement, and each changed the work.

**1. "The one census behind both symptoms" — there are two, and they are
independent.** §Root Cause names `builtins::inline_builtin_is_infallible` as the
single cause of both the dead-handler symptom and the `mfb audit` symptom. Only
the first is true. `mfb audit` never calls that function: it has its own
AST-level census in `src/audit/collect/source.rs`, where `is_fallible_builtin`
lists a bare `"toInt"` by name and `block_escapes`'s visitor discards the call's
arguments outright (`_arguments: &[CallArg]`). This was proved, not inferred —
after the Phase 2 fix landed, `mfb build /tmp/tointtrap` warned on both lines
while `mfb audit /tmp/tointfall` still reported `ordinalOf` as fallible,
unchanged. The document's own References half-knew this: `src/ir/fallible.rs`'s
module doc says the two censuses are "deliberately separate". The fix is
therefore two independent changes in disjoint files, not one.

The audit half was also the harder one. The IR census had a `TypeKinds` oracle
one accessor away; the AST census had no type information at all, so it needed a
declared-`ENUM` table and a per-function map of bindings that provably hold one.
That map fails closed on shadowing: a name bound anywhere in the function to
something that is not an annotated enum is dropped, so a `LET n AS String` in one
branch cannot be read as the enum parameter of the same name in another and
silently stop reporting a real `toInt(<String>)`.

**2. "`.ir`/`.nir`/`.ncode` goldens for any fixture containing `toInt(<enum>)`
will move" — none did.** §Blast Radius predicts a `CallResult` → `Call` collapse.
It does not happen, and should not: the inline-`TRAP` desugar emits `CallResult` +
`ResultIsOk` for its *own scrutinee* regardless of fallibility. The census decides
the warning and the nested-call hoist, not the scrutinee's shape. Verified against
the known-correct sibling rather than assumed — `toString(Color.Green) TRAP(e)`,
classified infallible since plan-140-C, lowers to the identical `callResult`
shape. `toInt_enum.ir` is byte-identical after the fix.

The only golden that moved is `tests/rt-behavior/general/toInt_enum/golden/build.log`,
gaining the two warning blocks and nothing else; the `.ast`, `.ir` and `.run`
goldens and the program's output are untouched. That fixture's source already
carried the comment "the conversion never fails, so the handler never runs" — the
golden was recording the bug.

**3. "`examples/dungeon` returns to 9 fallible functions" — it returns to 12, and
12 is correct.** The `9` was measured on the file as it stood *before* commit
`df455c0e`, which is not the file being fixed. Measured with
`mfb audit examples/dungeon | grep -c "(fallible)"`: 38 before, 12 after — both
re-measured after merging `main`, which had meanwhile split that example into
four files (`7cc6a5ca`); the split changes neither number. The
difference from 9 is three functions that the same commit `df455c0e` introduced
or changed, none of which reaches `toInt`: `insetX` and `insetY` are new and
fallible via `toFloat`, and `generateDungeon` gained an explicit `FAIL`. Confirmed
by auditing `df455c0e~1`'s version of the file with the fixed binary — it reports
9, and diffing the two name sets yields exactly those three. So the document's
"the single `toInt(f)` accounts for all 29" is right in substance: 26 of the 38
were that chain.

**4. "`mfb audit` on the reproduction lists neither `ordinalOf` nor `nameOf`" — it
also still lists `main`, correctly.** `main` calls `io::print`, which genuinely
raises `ErrOutput`. Only the `ordinalOf`/`nameOf` half of that expectation is
about this bug.

### Open Decision, settled

Signature vs. twin: took the **direct parameter**, as recommended. The document
estimated 4 call sites; there are 5 production ones — it missed
`src/codegen/engine/value/builder_values.rs` and
`src/codegen/engine/control/builder_control.rs` (plus ~22 test assertions, all
mechanical). Both new sites keep `NoTypeKinds`, which is sound in the way the
document argues: `builder_control`'s is a plan-145-C NIR optimizer with no
declaration table in scope, where a `false` merely forgoes an optimization, and
`builder_values`'s is unreachable for `toInt` anyway — every `toInt` form returns
earlier through `lower_inline_conversion_raw`. The `mod.rs:227`
(`inline_trap_unsupported`) verdict left open in §Blast Radius resolves the same
way: reached only from that unreachable path, and asserted in the new unit test.

## STATUS: FIXED

Landed on `worktree-B-679`, merged to `main`.

**What was wrong, in one line:** `toInt` is fallible by name and nothing told
either fallibility census that its enum overload — a register move of an ordinal
the value already holds — is the one exception, so a total program was described
as error-propagating in both `mfb audit` and the dead-handler warning.

**What changed:** two censuses, in disjoint files, each given the argument-aware
rule in its own terms. `inline_builtin_is_infallible` gained a `&dyn TypeKinds`
parameter and the granting rule, with real oracles threaded to `ir::fallible`
(via `LowerContext`'s `TypeIndex`) and `ir::verify::resources` (`TypeEnv` is
itself the oracle); `audit::collect::source` gained a declared-`ENUM` table, a
per-function enum-binding map, and the same rule over the AST.

**Deviation from the plan as written:** the document scoped this to one census;
it ships as two, because the audit symptom has a separate root cause the document
misattributed. See Corrections 1. Everything in §Non-goals held: no other `toInt`
overload changed, no `Enum` variant was added to `ParameterType`, no spelling
heuristic was used (both rules key on the declaration's kind), and `turned` in
`examples/dungeon` was not rewritten.
