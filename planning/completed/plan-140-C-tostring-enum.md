# plan-140-C: `toString(ENUM)` returns the member's name

Last updated: 2026-09-20
Effort: medium (1h–2h)
Depends on: plan-140-B

`toString(e)` for any enum value `e` (user-declared, imported from a package, or
a built-in package's enum) returns the member's name as it is spelled in the
`ENUM` declaration: `toString(Color.Green)` is `"Green"`. It never fails. The
result is indistinguishable from the string literal `"Green"`.

The single behavioral outcome:

```basic
ENUM Color
  Red, Green, Blue
END ENUM
' io::print(toString(Color.Green))  → Green
' io::print(toString(c)) for a runtime c → that member's name
```

References:

- plan-140-A — the `TypeKinds` seam (the `TO_STRING` arm reads it here).
- plan-140-B — `toInt(ENUM)`, whose fixtures and `TypeModel::is_enum_type` this
  reuses.
- `src/codegen/builtins/general/mod.rs:366` — the `TO_STRING` resolver arm;
  `expected_arguments(TO_STRING)` (`:262`); `func_to_string.rs` — the man page.
- `src/codegen/string/repr/builder_strings.rs:788` — `lower_to_string`.
- `src/codegen/memory/data/data_objects.rs:1363` — the `typeName` fold, the
  precedent for a built-in whose String result is a compile-time data object.
- `src/codegen/engine/builder/builder_emit_helpers.rs:177` —
  `load_string_constant`, which fails if the string has no registered data
  object (`"native code string literal '…' has no data object"`).
- `src/codegen/memory/value/builder_value_semantics.rs:1607` — enum `MATCH`
  lowering (compare the ordinal, branch), the precedent for selecting by
  ordinal at runtime.

## Prerequisites

See plan-140-A §Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-140-B complete (archived) | `ls planning/completed/plan-140-B-*` → one file | MET (2026-09-20: `planning/completed/plan-140-B-toint-enum.md`; B's whole-corpus gate at its end: 2072 goldens, 0 diffs) |

If plan-140-B is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- `toString(<enum value>)` type-checks as `String` in source and in package IR,
  is infallible, and at runtime returns the member's declared name. Proven by a
  new `tests/rt-behavior/general/toString_enum` fixture whose `.run` golden
  prints every member's name for a user enum, an imported enum and a built-in
  package enum.

### Non-goals (explicit constraints)

- **No string → enum parse** (`toEnum("Green")`) and no `typeName` change.
- **No qualified spelling.** The result is the bare member name (`"Green"`),
  not `"Color.Green"`; `typeName(c)` already gives the type (§Open Decisions).
- **No change to enum representation, package metadata or `type_sig_hash`.**
  Member names are already part of the ABI hash (`src/docs/spec/package/
  03_metadata-encoding.md:196`); this plan only reads them.
- **No new data for programs that never call `toString` on an enum.** Member
  names become data objects only for enums a program actually passes to
  `toString`, so every existing fixture's binary is byte-identical.
- Every existing `toString` override keeps dispatching. All of them take a
  non-enum type (census below).

## 2. Current State

- `toString(<enum>)` is rejected with `TYPE_CALL_ARGUMENT_MISMATCH` (measured
  2026-09-20, scratch project `/tmp/inplace_probe`, `toString(Layer.Gas)`).
- The `TO_STRING` arm accepts `Integer`, `Float`, `Fixed`, `Money`, `Boolean`,
  `String`, `Byte`, `Scalar`, `AttributedString`, `List OF Byte`, and the
  `(Float|Fixed|Money, Byte)` precision form (`general/mod.rs:366–395`, read).
- `lower_to_string` (`builder_strings.rs:788`) spills the value and formats it
  through a runtime helper selected by type. There is no enum case.
- A String result may be a static data object: `typeName` returns
  `load_string_constant(name)` with `origin: None` (`builder_values.rs:2121`),
  which is how every string literal loads (`builder_emit_helpers.rs:177`).
  A string's data object must be registered before codegen. The `typeName` fold
  registers its string in the data-object pre-pass (`data_objects.rs:1363`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Goldens quoting `toString`'s expected-overload list (will change) | 4 — `tests/syntax/general/toString_invalid`, `tests/syntax/color/color_to_string_unrelated_record_invalid`, `tests/syntax/resources/resource-state-bare-param-read-invalid`, `tests/syntax/tcp/local-address-field-binding-without-net-import` | `grep -rl 'Integer, Float\[, Byte\], Fixed\[, Byte\], Boolean, String, Byte, Scalar, or List OF Byte' tests src \| grep -v general/mod.rs` → 4 |
| User `FUNC toString(` declarations in the tree | 4, over `Tag` (a `TYPE`), `Integer`, `Point` (a `TYPE`), `List OF Point`; **0 over an enum** | `grep -rnE '^\s*(PUBLIC \|PRIVATE \|EXPORT )?FUNC toString\s*\(' --include='*.mfb' --exclude-dir=target .` → 4, each read |
| Package-provided `toString` overrides (registry) | 3 registrations: `color.Color`, `net.Url`, the vector records (one per `VEC_TYPES` entry); **0 over an enum** | `grep -rn 'add_override(' src/codegen \| grep -v 'fn add_override'` → 4 lines, one of them a registry unit test (`registry/mod.rs:5159`); each read |

### Verified properties

- **`toString(enum)` needs no fallibility change.** `toString` is infallible for
  every argument type except `List OF Byte`: `ArgFallibility::
  ToStringOverAListOfByte` (`builtins/mod.rs:275`) is the only arg-dependent
  `toString` rule, and both the IR census (`ir/fallible.rs:80`, via
  `inline_builtin_is_infallible`) and codegen read that same list (read).
- **Member order is available in codegen.** `TypeModel.enum_members` maps
  `(enum type, member name)` → ordinal (`builder/mod.rs:996`). Sorting one
  enum's entries by ordinal yields the declared order (read `validation.rs:288`).
- **The pre-pass can see enum member names** (resolved in Phase 1, via
  plan-140-B Correction B-C3): `FieldTypes` now carries every enum's members
  in declaration order, filled from the builder's `TypeModel`
  (`FieldTypes::with_enums_of`, `type_utils.rs`), and `string_symbols` receives
  that model (`builder/mod.rs` builds it first). Local, imported-package and
  built-in-package enums are all in `TypeModel.enum_members`.
- **Whether a `toString` argument is enum-typed** is answered by
  `static_type_name_for_fold_with_types` over `types`/`fields` for constants,
  locals, fields, list reads and built-in calls. It does **not** type a call to
  a user function (a NIR `Call` carries no result type, and the pre-pass has no
  function-return table), so `toString(favorite())` needs one more route: the
  module's function return types (`NirModule.functions[*].returns`). Read
  (`data_objects.rs:1388–1445`, `nir/mod.rs:298`). Phase 2's predicate adds it.
- **A user type named like a built-in package's override type is hijacked.**
  `registry::general_override_target` (`registry/mod.rs:2429`) also matches the
  descriptor's BARE `arg_type`, so a user `ENUM Color` (or `TYPE Color`) passed
  to `toString` routes to the `color` package's `#color_toString` in
  `ir/lower.rs:4965`, which does not resolve in a program that never imports
  `color` (measured on main's binary with a user `TYPE Color`:
  `error: NIR call target '#color_toString' does not resolve`). Pre-existing
  and wider than enums (bug filed, see Corrections C-C2). For this plan the
  routing must obey the gap-fill rule: an argument the built-in accepts never
  routes to a package override.

## 3. Design Overview

1. **Resolver** (`general/mod.rs`, `TO_STRING` arm): one argument for which
   `kinds.is_enum(&arg_types[0])` → `String`. The 2-arg precision form stays
   `Float|Fixed|Money` only. `expected_arguments(TO_STRING)` gains
   `", or an enum"`.
2. **Data objects** (`data_objects.rs` pre-pass): when a `toString` call's
   single argument is enum-typed, register each member name of that enum as a
   string data object, deduplicated by the existing interning. Programs that
   never do this register nothing new, which is why the byte-identity promise in
   the non-goals holds.
3. **Lowering** (`lower_to_string`, before the spill): when
   `self.type_model.is_enum_type(&value.type_)`, emit a compare-and-branch chain
   over the enum's members in ordinal order. Each arm does
   `emit_load_string_constant(result, name)` then branches to a shared end
   label. The last member is the fall-through, so there is no "no match" arm:
   the value is always a valid ordinal by construction, and a default arm would
   be the placeholder `.ai/compiler.md` forbids. The result has `origin: None`,
   the same as `typeName` and string literals.
4. **Typing**: `toString` already has a name-keyed nominal return (`String`,
   `general/mod.rs:1413` lists `"typeName" | "toString" => String`). plan-140-B's
   typing fallback covers the `NoTypeKinds` sites unchanged.

**Correctness gate: runtime behavior, not byte-identity.** Expected diffs:
the 4 goldens that quote the `toString` expected list, plus the new fixtures.
Every other golden must be byte-identical (non-goal 4). A diff elsewhere means
member names were registered for a program that never asked for them. That is a
bug, not a design failure; root-cause it from one fixture's data section.

**Where the risk concentrates:**

- **The pre-pass and lowering must agree** on which enums need names. If the
  pre-pass misses one, `load_string_constant` fails the build with "has no data
  object". That is the same two-places-must-agree hazard the `typeName` fold
  documents (`data_objects.rs:1649`). Mitigation: both call one shared predicate.
- **In-place string mutation on a static result.** `s = toString(c)` followed by
  `s = s & "x"` must copy before appending, exactly as it does for a literal.
  Covered by a fixture case; relies on the result being a literal in every
  observable respect.

Rejected alternatives:

- **A data table of string pointers indexed by ordinal.** This is O(1), but it
  needs a new data-object kind with one relocation per entry, and changes the
  data-section layout machinery for one built-in. The compare chain is O(member
  count) using existing primitives: `examples/brogue`'s largest enum has 215
  members (tile types), a few hundred cycles at worst per call.
- **A runtime helper taking a member-name array.** A new helper ABI and runtime
  table for what the compare chain does inline.
- **Register member names for every declared enum.** It is simpler, but it
  changes every binary that declares an enum, breaking non-goal 4.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. `- [~]` for partial, with what remains. Moot tasks are
> struck through with evidence, never deleted. Fill `Commit:` the moment a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — Locate the pre-pass's enum facts; failing fixtures

- [x] Read `src/codegen/memory/data/data_objects.rs` around the `typeName`
      fold (`:1363`, `:1510`, `:1649`) and record in §2 how the pre-pass can
      learn (a) that a `toString` argument is enum-typed and (b) that enum's
      member names in order. If it has no route, the task is to pass the
      `TypeModel`'s enum table into the pre-pass; name the exact signature.
      (Done in plan-140-B: `string_symbols(module, &TypeModel)`; §2 updated.)
- [x] `tests/rt-behavior/general/toString_enum/`: a user `ENUM Color Red, Green,
      Blue`. Print `toString` of each member; `toString` of a runtime value read
      from a `List OF Color` and from a record field; `MUT s = toString(c)` then
      `s = s & "!"` (in-place append on the result); `toString(c)` as a
      `Map OF String TO Integer` key; inside an inline `TRAP` (must not trap);
      a built-in package enum (name it after reading the registry).
      `golden/*.run` hand-written. (Built-in enum: `datetime::Weekday`. Also a
      user function's enum result, `toString(favorite())`, per §2.)
- [x] `tests/rt-behavior/general/toString_enum_package/`: a `PUBLIC` enum from
      a second source file and one from a local package fixture.
- [x] `tests/rt-behavior/functions/func_override_tostring_enum_precedence/`: a
      record override `FUNC toString(p AS Point)` and an enum in one program;
      both dispatch correctly.
- [x] `tests/syntax/general/toString_invalid/src/main.mfb`: add
      `toString(Color.Red, 2)` (the precision form rejects an enum). Hand-edit
      the golden.

Acceptance: §2 has no UNVERIFIED row; the new fixtures fail for the right reason.
  Check: `grep -c '^- \*\*UNVERIFIED' planning/plan-140-C-tostring-enum.md` → 0 (1 min).
  (Anchored to §2's row form, as plan-140-A's A-C4: the bare word also
  matches this check line.)
  Check: `scripts/test-accept.sh target/release/mfb target/accept-actual 'toString_enum*' 'func_override_tostring_enum_precedence' 'toString_invalid'`
  → rt-behavior fixtures fail with `TYPE_CALL_ARGUMENT_MISMATCH` on the
  `toString(<enum>)` lines (est. 2 min).
  Result: `grep -c '^- \*\*UNVERIFIED' planning/plan-140-C-tostring-enum.md` → 0.
  `test-accept … 'toString_enum*' 'func_override_tostring_enum_precedence'
  'toString_invalid'` → `4 test(s) ran`, all mismatched: `toString_invalid`
  on the expected-overloads string only; `toString_enum` with
  `TYPE_CALL_ARGUMENT_MISMATCH` at `main.mfb:55`/`:56`
  (`datetime.Weekday`); `toString_enum_package` with it at the package's
  `lib.mfb:11` (`Suit`). The user `Color` lines were NOT rejected: they were
  hijacked by the `color` override (§2), and the precedence fixture fails its
  build with `NIR call target '#color_toString' does not resolve`. That is a
  failure for a real reason this plan must fix (C-C1), so it counts as RED.
Commit: 3d6e78d3f

### Phase 2 — Resolve, register names, lower

- [x] `src/codegen/builtins/general/mod.rs`: the `TO_STRING` arm accepts one
      enum argument via `kinds.is_enum`; `expected_arguments(TO_STRING)` gains
      `", or an enum"`. Unit tests: enum oracle → `String`; non-enum oracle →
      `None`; `(Color, Byte)` → `None`. (`resolve_to_string_enum`;
      `cargo test --bin mfb codegen::builtins::general` → 29 passed.)
- [x] One shared predicate, `to_string_needs_enum_names(arg_type, enum
      table) -> Option<&[member names]>`, used by both the pre-pass and the
      lowering. (Landed as `type_utils::to_string_enum_members` for the
      pre-pass's "which argument" question, over ONE member-order table,
      `TypeModel.enum_names`, that the lowering reads directly
      (`enum_member_names`) and the pre-pass receives via
      `FieldTypes::with_enums_of` — C-C4.)
- [x] `src/codegen/memory/data/data_objects.rs`: register the member names
      for each enum-typed `toString` argument, via the predicate.
- [x] `src/codegen/string/repr/builder_strings.rs:788`: the enum
      compare-and-branch chain, before the numeric spill; `origin: None`.
      (`lower_enum_to_string`; returns a fresh marked copy of the selected
      name, not the read-only pointer — C-C3.)
- [x] `src/ir/lower.rs` package-override routing (C-C1): route a general
      built-in call to a package override helper only when the built-in,
      given the `TypeIndex` kind oracle, rejects the argument types.
      (`func_override_tostring_enum_precedence` prints `(3,4 Green)`, `Blue`,
      `42`; the IR keeps `toString` on the built-in for `Color`.)
- [x] Update the 4 goldens that quote the `toString` expected list: the one
      line each, hand-edited to the new string, and nothing else in those files.
      (`toString_invalid` in Phase 1; the other three here: 4 lines in 3
      files, since `color_to_string_unrelated_record_invalid` quotes it twice
      — C-C5.)

Acceptance: the Phase 1 fixtures and every existing `toString` fixture pass.
  Check: `cargo build --release && scripts/test-accept.sh target/release/mfb target/accept-actual 'toString*' 'func_override_tostring*' 'func_override_visibility' 'func_override_no_hijack_valid' 'color_to_string*' 'resource-state-bare-param-read-invalid' 'local-address-field-binding-without-net-import'`
  → all pass (est. 3 min).
  Check: `bash scripts/artifact-gate.sh target/release/mfb all` → diffs only in
  the 4 goldens above and the new fixtures (est. 10–20 min; the byte-identity
  promise for programs without an enum `toString` covers the whole corpus, so
  nothing smaller proves it).
  Result: `acceptance tests passed (12 test(s) ran)`, with the three new
  fixtures run. Whole corpus: `artifact-gate [all]: 1473 tests, 1648 build(s), 2078 golden(s) checked, 0 diff(s)`. The only goldens this plan
  changed (4 expected-list goldens, hand-edited) now match, and every other
  golden is byte-identical.
Commit: f5c029b00

### Phase 3 — Docs and spec

- [x] `src/codegen/builtins/general/func_to_string.rs`: DESC gains a short
      paragraph: from an enum value, `toString` gives the member's name as
      written in the `ENUM`, without the type name, and never fails. EX gains
      an enum example with its output. Use no compiler vocabulary
      (`.ai/man-content.md`).
- [x] `src/docs/spec/language/18_builtin-functions.md`: the `toString` row
      lists "any enum (the bare member name; infallible)". The override
      paragraph (`:123`) extends plan-140-B's sentence to `toString`. Cite
      `[[src/codegen/string/repr/builder_strings.rs:lower_to_string]]` and the
      shared predicate.
- [x] `src/docs/spec/language/04_types.md` §4.5: extend plan-140-B's sentence:
      a member's `toString` is its name.

Acceptance: the man page renders and its examples run; the spec builds and its
citations resolve.
  Check: `scripts/man-run-examples.sh general --run` → every `toString` example
  prints its documented output (est. 2 min).
  Check: `cargo test --bin mfb spec && scripts/spec-census.sh --citations` →
  pass, 0 dangling (est. 3 min).
  Check: `scripts/man-census.sh --memory-scope` → 0 unclassified hits (1 min).
  Result: `man-run-examples.sh general --run` → `examples: 36 built: 36 ran:
  36 not run: 0 failed: 0`; `toString example 3` prints `Green`,
  `Color.Green` as documented. `cargo test --bin mfb spec` → 43 passed;
  `spec-census.sh --citations` → MISS-PATH 0, MISS-LINE 0, MISS-SYMBOL 0
  (the new citation of `type_utils.rs:to_string_enum_members` resolves).
  `man-census.sh --memory-scope` → `unclassified memory-vocabulary hits: 0`.
Commit: 74b84c368

## Validation Plan

- Tests: Phase 1's fixtures cover valid use (user, imported and built-in enums;
  runtime values; in-place append on the result; map key; inline TRAP),
  invalid use (the precision form with an enum), and override precedence. Unit
  tests cover the resolver arm and the shared predicate.
- Coverage check: the accept run must list the three new fixtures as run, not
  skipped.
- Runtime proof: the `toString_enum` `.run` golden.
- Doc sync: Phase 3.
- Final gate (once, after Phase 3 — and it is also plan-140's final gate):
  `scripts/test-accept.sh target/debug/mfb target/accept-actual` → all pass, and
  `bash scripts/artifact-gate.sh target/release/mfb all` → diffs only in
  plan-140-B's and this plan's expected goldens and new fixtures.

## Open Decisions

- **Bare member name vs. qualified.** Recommended: bare (`"Green"`). It matches
  how the member is written in its declaration and in `CASE` arms after the
  type, and `typeName(c) & "." & toString(c)` builds the qualified form if
  wanted. vs. `"Color.Green"`: unambiguous in logs, but it can't be undone
  without string surgery, and for a package enum it raises the question of
  whether the result is `"crypto.Hash.SHA256"` or `"Hash.SHA256"`.
- **Out-of-tree `toString` overrides over an enum become dead code.** Same
  recommendation as plan-140-B's `toInt` decision: accept and document it. In-tree
  census is 0 (user and registry).

## Corrections

- **C-C1 (Phase 1):** the fixtures exposed `ir/lower.rs`'s package-override
  routing (`package_override`, around `:4965`). It routes a general built-in
  call by the argument's type to a package helper without asking whether the
  built-in accepts that type. Phase 2 gains a task: route to a package
  override only when the built-in (with the `TypeIndex` kind oracle) rejects
  the arguments, which is the §18.3 gap-fill rule. Only an enum argument
  changes: no registered override type is one the built-in accepts
  (`color.Color`, `net.Url`, vector records).
- **C-C2 (bug found, outside scope):** `general_override_target`'s bare-name
  match hijacks any user type named `Color`/`Url`/`Float2`/… passed to
  `toString` (see §2). C-C1 fixes the enum case, where the built-in is
  authoritative. A user *record* named `Color` still misroutes, and that
  belongs to its own bug document: filed as
  `bugs/bug-668-user-type-named-like-package-override-is-hijacked.md`.
- **C-C3 (Phase 2, ownership):** §3 step 3 and "Where the risk concentrates"
  assumed a read-only result is safe "exactly as a literal" under in-place
  append. That is false. A literal is safe only because `static_string_value`
  recognizes it and the owning bind copies it. A `toString` call result gets no
  such copy, and `MUT label = toString(<enum>)` then `label = label & "!"`
  SIGBUSed (exit 138): the in-place append's regrow `arena_free`d read-only
  data. The same crash exists today for `toString(<Boolean>)` on main (filed
  as `bugs/bug-667-bound-tostring-boolean-self-append-sigbus.md`).
  `lower_enum_to_string` therefore selects the name's string data, then
  returns a `copy_flat_block` copy marked with `mark_fresh_string`, like the
  `AttributedString` arm. The result is an ordinary fresh `String`, and no
  second ownership predicate can disagree with the lowering. The string data
  is still registered only for enums passed to `toString` (it is the copy
  source), so non-goal 4 stands.
- **C-C4 (Phase 2, pre-pass typing):** the pre-pass's static typing did not type
  `Enum.Member` itself (`Local(T).member`), and NIR calls carry no result type.
  The first build of `toString_enum_package` failed with
  `native code string literal 'Ground' has no data object`. In `toString_enum`
  the same miss was masked because `toString(c)` in the `FOR EACH` had
  already registered every `Color` name. `to_string_enum_members` now mirrors
  the builder's order: an `Enum.Member` literal first (as `lower_value`'s
  `MemberAccess` arm does), then the static type, then a module function's
  declared return (`FieldTypes::with_function_returns`).
- **C-C5 (goldens):** the §2 population counts FILES (4). The expected-list
  string occurs on 5 lines: `color_to_string_unrelated_record_invalid` has two
  mismatch diagnostics. Each quoted line was hand-edited and nothing else.
- **C-C6 (fixture):** the plan's inline-`TRAP` case on `toString(<enum>)` draws
  the `TYPE_INLINE_TRAP_DEAD_HANDLER` advisory, which is correct per spec §8
  rule 11 (`toString` is infallible for every argument but `List OF Byte`). The
  hand-written `build.log` golden carries that warning in both build steps. It
  shows directly that the call cannot fail.

## Summary

The resolver and fallibility parts are free: the seam exists (plan-140-A) and
`toString` is already infallible for everything but a byte list. The real work
is making the member names exist as data exactly when needed. The data-object
pre-pass and the lowering must agree on that, and no other program may change
by a byte. Untouched: enum representation, package metadata, `typeName`, and
any string → enum conversion.
