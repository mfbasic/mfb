# bug-1013: a `STATE` type that is not a record is accepted, then fails to build (scalar) or crashes (`String`)

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: none yet — Phase 1 adds `tests/rt-behavior/resources/bug1013_scalar_string_state_rt`

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

- [ ] Add `tests/syntax/resources/bug1013_state_requires_record_invalid`: `STATE` of
      `Integer`, `Boolean`, `Float`, `String`, a bare `ENUM`, `List OF Integer` and a
      `UNION`, at a binding, a parameter, a return and a record field. Confirm that
      today only the `UNION` rows are diagnosed.
- [ ] Unit cases in `src/ir/verify/tests.rs` for the same matrix.
- [ ] Answer the four questions for `accepts_state_list_of_nondefaultable`.

Acceptance: the fixture and unit cases fail for the documented reason.
Commit: —

### Phase 2 — the fix

- [ ] Add `is_state_type` and use it at the four verifier sites; reword the diagnostic.

Acceptance: the Phase 1 fixture and unit cases pass; a record payload still builds and runs.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate any golden the fix moves and attribute each delta.
- [ ] `scripts/test-accept.sh target/release/mfb <actual-dir>` green.
- [ ] Update `src/docs/spec/language/15_resource-management.md` and
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
