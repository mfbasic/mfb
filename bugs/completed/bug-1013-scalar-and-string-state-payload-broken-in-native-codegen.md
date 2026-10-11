# bug-1013: a `STATE` type that is not a record is accepted, then fails to build (scalar) or crashes (`String`)

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Fixed (bs19-agent-a)
Regression Test: `tests/syntax/resources/bug1013_state_requires_record_invalid`, `tests/syntax/resources/bug1013_state_import_invalid`, `tests/syntax/native/bug1013_native_state_requires_record_invalid`, `tests/rt-behavior/resources/bug1013_state_record_rt`, and the `ir::verify::tests` cases listed under Resolution

`RES f AS fs::File STATE T` is accepted by the front end for any copyable, defaultable
`T`. Native code generation handles only a `T` that is a *block*: a record, a `List`, a
`Map` or a `Set`. For a scalar `T` (`Integer`, `Boolean`, `Float`, and — since
plan-172-N made enums defaultable — a bare `ENUM`) the build fails with an internal
lowering error instead of a diagnostic. For `T = String` the build succeeds and the
program dies with a bus error on the first `f.state = …`.

**The single correct behavior a fix produces** (owner's ruling, 2026-10-10: "a STATE must
always be a Record and the compiler should error if not"): a `STATE` type that is not a
record `TYPE` is rejected at compile time with `TYPE_STATE_INVALID`, at every position a
`STATE` clause can appear. A record that is copyable and defaultable stays accepted and
unchanged.

References:

- `src/docs/spec/language/15_resource-management.md` — "`T` must be an ordinary
  **copyable, defaultable data type** (`TYPE_STATE_INVALID` otherwise)", which admits
  `Integer` and `String`.
- `src/docs/man/variable/package.md`, "A handle can carry its own data: STATE" — which
  says the value "is an ordinary record".
- Found while validating plan-172-N (`planning/plan-172-N-enum-default-first-variant.md`)
  in the `P-172` worktree. Not caused by it: see the matrix.

## Failing Reproduction

The three programs are `bugs/repro/bug-1013/`. Each is a single-file console project.

```
mfb build <project with src/main.mfb = state_integer.mfb>
mfb build <project with src/main.mfb = state_string.mfb> && ./build/<name>.out
```

`state_integer.mfb`, the part that matters:

```basic
RES g AS fs::File STATE Integer = fs::open("/tmp/mfb-np.txt", "read")
io::print("stateenum " & toString(g.state))
g.state = 5
io::print("stateenum set " & toString(g.state))
```

- Observed (`Integer`): `error: native inlined field size not available for type
  'Integer' while lowering eval call io.print`, no executable.
- Observed (`String`): the build succeeds; the run prints `stateenum ` and then
  `Bus error: 10`, exit 138.
- Expected, both: no build — `TYPE_STATE_INVALID` at the `RES` line, naming the type and
  saying a `STATE` type must be a record.

The contrast case that works today and stays a regression guard: `STATE <record>`
(`state Fast pos=0` / `state set Slow pos=7`), including a record with an enum field.

One case works today and **stops** working under the ruling: `STATE List OF Integer`
(`0`, then `3` after assigning `[1, 2, 3]`; `bugs/repro/bug-1013/state_list_works.mfb`).
A collection is not a record, so it becomes `TYPE_STATE_INVALID`; a program wraps it in
a one-field record.

Measured 2026-10-10 on macOS aarch64, release build:

| `STATE` type | Compiler | Result |
| --- | --- | --- |
| `Integer` | untouched main checkout (`target/release/mfb`, built 2026-10-06) | fails to build ✗ |
| `Integer`, `Boolean`, `Float` | `P-172` at `fc530a22e` | fails to build ✗ |
| bare `ENUM` | before plan-172-N | rejected, `TYPE_STATE_INVALID` ✓ |
| bare `ENUM` | `P-172` at `fc530a22e` (plan-172-N) | fails to build ✗ — the one row plan-172-N moved |
| `String` | `P-172` at `fc530a22e` | builds, bus error at run time ✗ |
| record, record with an `ENUM` field, `List OF Integer` | `P-172` at `fc530a22e` | works ✓ |

Other targets were not run; nothing in the mechanism below is target-specific.

Two existing fixtures are red until this is fixed, and need no edit when it is:
`tests/syntax/resources/resource-state-invalid` and
`tests/syntax/resources/resource-return-state-invalid`. Each uses a bare `ENUM` as its
illegal `STATE` type and expects `TYPE_STATE_INVALID`; since plan-172-N that type passes
the check (`scripts/test-accept.sh` at `fc530a22e`: both `build.log` mismatches).

## Root Cause

A resource record has one `STATE` word at `RESOURCE_OFFSET_STATE`
(`src/codegen/error/constants/error_constants.rs`). Every site that touches it treats the
word as **a pointer to an arena block the resource owns, with 0 meaning "no payload
yet"**:

- `builder_value_semantics.rs:emit_resource_state_init` stores `lower_default_value(T)`
  when the word is 0. For a scalar that is the value 0 itself; for `String` it is the
  shared empty-string constant, which is not an arena block.
- `builder_value_semantics.rs:lower_field_access_of` (`member == "state"`) loads the word
  and returns it typed `T`. A consumer that copies the value then asks for its block
  size.
- `builder_resource_cleanup.rs:emit_free_resource_state_block` sizes the block with
  `emit_inlined_block_size_from_ptr_slot(T, …)` and frees it.
  `builder_collection_layout.rs:emit_inlined_block_size_from_ptr_slot` has arms for
  `String`, a record, a data union, a `Result` and a collection, and for anything else
  returns the error in the reproduction. **That is the scalar failure**: a scalar has no
  block to size.
- `builder_control.rs`, the `ident.state = expr` path (bug-644, bug-678), frees the
  displaced block after publishing the new one. **Hypothesis for the `String` crash**,
  to confirm in Phase 1 under a debugger: the displaced "block" on the first assignment
  is the empty-string constant the init stored, and freeing a constant faults.

Records and collections are immune because their default is a freshly built arena block
and their size is readable from the block.

## Goal

- A `STATE` type that is not a record `TYPE` is `TYPE_STATE_INVALID`: on a `RES` binding,
  a parameter, a return type, a record field, a collection element and a thread's
  resource plane.
- No program reaches `native inlined field size not available` through a `STATE` type.

### Non-goals (must NOT change)

- The resource record layout and `RESOURCE_OFFSET_STATE`.
- The behaviour of record and collection payloads, including the in-place field update
  (`f.state.pos = 7`) and bug-644 / bug-678's ownership rules.
- plan-172-N's rule that an `ENUM` is defaultable. A bare enum as `STATE` is rejected
  because it is not a record, by the same check as `Integer`, not by a special case.
- A record with an enum field, a collection field or a `String` field stays a legal
  `STATE` type.
- Native code generation for record payloads. The fix is in the verifier.

## Blast Radius

Found with `grep -rn RESOURCE_OFFSET_STATE src --include='*.rs'`:

The code-generation sites, all unaffected once the verifier admits only records, because
a record payload is what each was written for: `emit_resource_state_init`,
`lower_field_access_of`, the `.state` assignment sites in `builder_control.rs`,
`emit_free_resource_state_block`, `inplace_dest.rs`, `builder_arena_transfer.rs`,
`src/debugger/values.rs`, and the builtin producers that store 0.

The verifier sites that decide `TYPE_STATE_INVALID`, found with
`grep -rn 'is_defaultable(&state' src/ir/verify` — each is fixed by this bug:

- `src/ir/verify/ops.rs` — a `RES` binding's `STATE`.
- `src/ir/verify/mod.rs` — a parameter's `STATE`.
- `src/ir/verify/calls.rs` — a `FUNC` return's `STATE`.
- `src/ir/verify/types.rs` — a record field's `STATE`.

Tests that pin the old rule and need the four answers before they change:
`src/ir/verify/tests.rs:accepts_state_list_of_nondefaultable` (a `List` as `STATE`), and
any fixture found by `grep -rnE 'STATE (List|Map|Set|Integer|String|Float|Boolean)\b'
tests` (0 on 2026-10-10).

## Fix Design

One predicate, `is_state_type`, used by the four verifier sites in place of the bare
`is_defaultable` call: the type is a declared record `TYPE` (in `record_field_lists`)
**and** defaultable. The diagnostic text says a `STATE` type must be a record.

Rejected: supporting scalar and `String` payloads in code generation. It was this
document's first recommendation; the owner ruled for the narrower contract, which is
also what `mfb man variable` already says ("an ordinary record").

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/syntax/resources/bug1013_state_requires_record_invalid`: `STATE` of
      `Integer`, `Boolean`, `Float`, `String`, a bare `ENUM`, `List OF Integer` and a
      `UNION`, at a binding, a parameter, a return and a record field. Confirm that
      today only the `UNION` rows are diagnosed.
- [x] Unit cases in `src/ir/verify/tests.rs` for the same matrix.
- [x] Answer the four questions for `accepts_state_list_of_nondefaultable` (under Resolution).

Acceptance: the fixture and unit cases fail for the documented reason.
Commit: `bug-1013 RED: a STATE type that is not a record must be TYPE_STATE_INVALID at every position`, `bug-1013 RED: add the lambda-parameter and AttributedString rows to the fixture`

### Phase 2 — the fix

- [x] Add the predicate (landed as `state_type_fault`, applied by the walk `check_state_clauses`) and use it at the four verifier sites; reword the diagnostic.

Acceptance: the Phase 1 fixture and unit cases pass; a record payload still builds and runs.
Commit: `bug-1013: a STATE type must be a record TYPE (TYPE_STATE_INVALID at every position)`

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate any golden the fix moves and attribute each delta. **Left to the coordinator's gate** (batch rule: an agent regenerates no golden); the eight goldens and the reason for each are listed under "Golden impact".
- [ ] `scripts/test-accept.sh target/release/mfb <actual-dir>` green. **Not run in full** (batch rule: targeted checks only); the targeted run is under "Checks run".
- [x] Update `src/docs/spec/language/15_resource-management.md` and
      `src/docs/man/variable/package.md` to say what a `STATE` type may be.

Acceptance: full suite green; the reproductions pass.
Commit: —

## Validation Plan

- Regression test: the Phase 1 fixture and unit cases.
- Runtime proof: the three programs in `bugs/repro/bug-1013/` each fail to build with
  `TYPE_STATE_INVALID`; a record-payload fixture still runs.
- Doc sync: the spec and man passages named in Phase 3.
- Full suite: `scripts/test-accept.sh`, plus `scripts/artifact-gate.sh … all`.

## Open Decisions

None. "Support scalar and `String` payloads, or reject them" was answered by the owner
on 2026-10-10: reject — a `STATE` type is always a record.

## Summary

The front end admits `STATE` types the code generator was never written for. Nothing in
the tree uses one (every `STATE` in `tests/` and the docs names a record), which is how
it went unseen. The fix is a verifier predicate; the only behaviour it removes that works
today is a collection used directly as a payload.

## Resolution

A `STATE` type is a record `TYPE` that is copyable and defaultable. Everything else is
`TYPE_STATE_INVALID` (2-203-0085), at every position a clause can be written. The rule's
title is now "a STATE type must be a record TYPE that is copyable and defaultable".

**The rule.** `src/ir/verify/resources.rs:state_type_fault` decides the kind from what is
positively known. A scalar, a `String`, a collection, a `FUNC`, a thread handle or a `RES`
slot is not a nominal and is refused outright. A nominal is refused when it is
`AttributedString` (spelled as a nominal, but a built-in value type), an `ENUM`, a `UNION`,
a resource or a resource union. What is left is a record, which then has to pass the
existing `is_defaultable`. A union's member is a record `TYPE` in its own right and stays
legal, and so do the built-in records `Error` and `ErrorLoc`.

**One walk for every position.** `check_state_clauses` walks the declared type and judges
every `Stateful` node in it. The four verifier sites this document named checked only a
*top-level* clause, so these positions were unchecked and are fixed by the same walk. Each
row is a line of `tests/syntax/resources/bug1013_state_requires_record_invalid` or of
`tests/syntax/native/bug1013_native_state_requires_record_invalid`, accepted by the base
compiler (`scripts/test-accept.sh /tmp/main-dbg-target/release/mfb <dir> 'bug1013_*'`:
6 diagnostics where the three goldens hold 29, and the native fixture builds):

| Position | Example accepted before |
| --- | --- |
| collection element / map value | `MUT files AS List OF RES fs::File STATE Integer = []` |
| record field | `scalar AS RES fs::File STATE Integer` |
| element inside a field or a parameter | `many AS List OF RES fs::File STATE Mode` |
| thread resource plane | `ThreadWorker OF Integer RES fs::File STATE Mode TO Integer` |
| thread view | `ThreadWorker OF Integer TO Integer STATE Integer` |
| native `LINK` parameter and return | `FUNC open(path AS String) AS RES Db STATE Integer` |
| top-level binding | `RES topBad AS fs::File STATE Float = fs::createTempFile()` |
| lambda parameter | `LAMBDA(x AS fs::File STATE Integer) -> 1` |

The last two are decided in `ir::shape` (`check_erased_state_type`), not in the verifier,
because lowering does not carry either clause into the IR, so the verifier has no clause to
judge. Both are pre-existing gaps of their own; see "Found and not fixed".

**Imported types.** The kind of an imported type is read from the package's type table.
The verifier already seeded imported `ENUM`s and `UNION`s (bug-554), so those were refused
by `is_defaultable` already; what was missing was the positive half, knowing that an
imported name IS a record, which `TypeEnv::record_decls` now holds (seeded in
`collect_diagnostics_with`). A built-in package's types are answered by the registry
(`registry().resolve_type`): `http::PendingState` is a record and `http::Stream` is not
(probe: `STATE http::PendingState` builds with `-ast -ir`, `STATE http::Stream` is
`TYPE_STATE_INVALID`). A name no table can place falls through to `is_defaultable`, which
keeps bug-258's source/package split unchanged.

**The `String` crash, confirmed under the debugger.** `mfb build --debug -O0` of
`bugs/repro/bug-1013/state_string.mfb` with the base compiler, then
`printf 'be any\nrun\nbt\nquit\n' | mfb dbg build/ss.out`: the stop is
`_mfb_arena_free+0x15c`, called from `main`, "a bad memory access at 0x…ee0430" with `pc`
0x…ed6fd0. The faulting address is inside the executable image, 0x9460 past the faulting
instruction, not in any arena. That is this document's hypothesis: the first
`g.state = "five"` frees the displaced payload, and the displaced payload is the shared
empty-string constant `emit_resource_state_init` stored.

**Tests.** Fixtures: `bug1013_state_requires_record_invalid` (every kind × every position
in one file, with the record contrast), `bug1013_state_import_invalid` (imported `ENUM`
and `UNION` refused, imported record and an imported union's member accepted, with a
local record that shares the imported enum's name), `bug1013_native_state_requires_record_invalid`
(the `LINK` positions), `bug1013_state_record_rt` (the three reproductions' payloads as
fields of a record: default, in-place update, whole assignment, run). Unit, in
`ir::verify::tests`: `rejects_state_type_that_is_not_a_record`,
`accepts_state_type_that_is_a_record`, `rejects_non_record_state_at_every_position`,
`imported_types_are_judged_by_their_kind`, `rejects_state_collection_not_a_record`.

RED before the fix: `cargo test --release --bin mfb -- ir::verify::tests::rejects_state_type_that_is_not_a_record
ir::verify::tests::rejects_non_record_state_at_every_position` fails both
("STATE Integer must be refused once", "parameter fs.File STATE Integer must be refused
once"); the other two new unit tests and `bug1013_state_record_rt` are guards and pass
before and after. The `AttributedString` and `Error` rows of the two unit tests were added
with the fix and were not shown red on their own; their fixture rows (lines 98 to 100 of
the fixture) were.

**The one test that pinned the old rule.** `accepts_state_list_of_nondefaultable` is now
`rejects_state_collection_not_a_record`. The four answers:

1. Written in `6ff17fd39` (2026-08-09, bug-434; `git log -S accepts_state_list_of_nondefaultable
   -- src/ir/verify/tests.rs`), as an "intended STATE ripple" of collections becoming
   always defaultable.
2. It protected: a collection `STATE` is accepted because a collection is defaultable.
3. Nothing depends on it. `grep -rnE 'STATE (List|Map|Set|Integer|String|Float|Boolean|Fixed|Money|Byte)\b'`
   over `src tests examples packages tools benchmark` (`*.rs`, `*.mfb`, `*.md`, goldens
   excluded) finds only this bug's fixtures, the verifier tests, a comment in each of
   `src/ir/shape.rs` and `src/ir/verify/resources.rs`, and the parser and type-spelling
   tests in `src/ast/tests.rs` and `src/types.rs`, which test the grammar and the spelling,
   not the rule, and are unchanged.
4. Proof it is wrong: the owner's ruling of 2026-10-10 at the top of this document, which
   names this test in "Blast Radius".

bug-434's own subject, collection defaultability, is untouched and stays covered by
`accepts_mut_list_of_union_defaultable_empty`,
`accepts_mut_map_value_union_defaultable_empty` and
`accepts_mut_record_with_list_of_union_field`. No other existing test was edited: the five
verifier tests that use `STATE Integer` as an incidental payload while testing another rule
still pass unmodified (`cargo test --release --bin mfb -- ir::verify::`).

### Corrections to this document

- "Two existing fixtures are red until this is fixed" does not hold at this batch's base.
  plan-172-N (an `ENUM` is defaultable) is not on it, so a bare `ENUM` is still refused by
  `is_defaultable` and both fixtures are green before the fix
  (`scripts/test-accept.sh /tmp/main-dbg-target/release/mfb <dir> resource-state-invalid
  resource-return-state-invalid`). After the fix a bare `ENUM` is refused because it is
  not a record, which keeps holding once plan-172-N lands; their goldens change only in
  wording (below).
- For the same reason "a record with an enum field stays a legal `STATE` type" is not
  true at this base (an enum field makes the record non-defaultable here). This fix does
  not change that either way.
- `bugs/repro/bug-1013/state_list_works.mfb` is now refused, as the ruling says.

### Found and not fixed (reported to the coordinator)

Each is outside what this document settles, so none was changed here. Each reproduces with
the base compiler, `/tmp/main-dbg-target/release/mfb build -ast -ir`.

1. **A top-level `RES` binding's `STATE` is dropped.** `lower_binding` (`src/ir/lower.rs`)
   ignores `HirTopLevelBinding::state_type`, so `RES g AS fs::File STATE Cur =
   fs::createTempFile()` at the top level followed by `g.state.n` in a function is
   `TYPE_STATE_INVALID` "`fs.File` here has no STATE to read". The clause's type is
   validated now but the clause is still not attached.
2. **A lambda parameter's `STATE` is dropped.** `LET j = LAMBDA(x AS fs::File STATE Cur) ->
   x.state.n` reports the same "`fs.File` here has no STATE to read", plus
   `TYPE_UNKNOWN_VALUE` for `j`. Same shape as item 1.
3. **`RES` on a non-resource is not refused at a top-level binding or a return.**
   `RES h AS Integer = 1` at the top level and `FUNC mk() AS RES Integer` both build.
   `TYPE_RES_REQUIRES_RESOURCE` is checked for a local binding, a field and an element
   only.

## Checks run

- `cargo test --release --bin mfb -- ir::verify:: ir::shape:: rules:: spec docs::`:
  630 passed, 0 failed.
- `scripts/test-accept.sh target/release/mfb <dir> 'bug1013_*'`: passed, 4 fixtures.
- `scripts/test-accept.sh target/release/mfb <dir> <names>` over every fixture whose
  source spells `STATE`, every fixture under a `resources`, `threads`, `http`, `native`,
  `packages`, `net`, `tcp` or `audio` directory, and every fixture that imports `http` or
  `thread` (435 distinct fixtures, in two runs of 88 and 348): the 8 `build.log` mismatches
  listed below and nothing else.
- `scripts/spec-census.sh --render` (0 leaks) and `--citations` (0 missing).
- `mfb man variable` rendered and read; `MFB=target/release/mfb
  scripts/man-run-examples.sh variable --topic --run`: 11 of 12 run, and the one failure
  (`variable::guide#2`, `SYMBOL_SHADOWS_TOP_LEVEL_BINDING`) is identical with the base
  compiler.
- `packages/libsnd`, `tools/thread-package-sources/state_xfer_workers` and
  `union_xfer_workers` build with `-ast -ir`; no `TYPE_STATE_INVALID`.

## Golden impact

No golden was regenerated. Eight existing `build.log` goldens change, all for the reworded
rule title (`STATE must be a copyable, defaultable data type` → `a STATE type must be a
record TYPE that is copyable and defaultable`) and, where marked, the reworded detail
line. No code generation moved: the fix is in `ir::verify` and `ir::shape`.

- `tests/syntax/resources/resource-state-invalid`: title + detail ("is not a record; …",
  a bare `ENUM`).
- `tests/syntax/resources/resource-return-state-invalid`: title + detail (a bare `ENUM`).
- `tests/syntax/resources/record-res-field-state-invalid`: title + detail (a resource).
- `tests/syntax/resources/resource-param-state-invalid`: title + detail ("must be a
  copyable, defaultable record TYPE": a record that is not defaultable).
- `tests/syntax/resources/resource-state-assign-no-state-invalid`,
  `resource-state-bare-param-read-invalid`, `resource-state-bare-param-write-invalid`,
  `tests/syntax/threads/threadworker-resource-rules-invalid`: title only (these report
  the "no STATE to read/assign" form of the same rule).
