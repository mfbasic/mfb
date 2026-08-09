<!-- Bug document. See .claude/skills/write-bug/template.md -->

# bug-434: `List`/`Set`/`Map` are wrongly non-defaultable when their element type is non-defaultable

Last updated: 2026-08-08
Effort: medium (1h–2h)
Severity: LOW
Class: Footgun (over-restrictive rule — blocks valid programs; no wrong runtime output)

Status: FIXED (6ff17fd39, 3df1d5d6f)
Regression Test: tests/syntax/types/mut-default-collection-of-nondefaultable-valid/ (new); ir/verify unit tests (new)

STATUS: FIXED (6ff17fd39 fix+tests+fixture, 3df1d5d6f spec sync)

The core fix was exactly as designed: the three collection arms of
`is_defaultable` (`src/ir/verify/resources.rs`) now `return true`
unconditionally. Full validation green: `cargo test --bin mfb` (3790 passed),
artifact-gate `all` (1202 tests, 0 diffs), test-accept `tests/syntax/*` (594
tests), original reproduction compiles+links+runs (exit 0), and a runtime proof
(`MUT xs AS List OF <union>`, append, `len(xs)==1`).

Two deviations from the plan, both because the plan was wrong on a detail:

1. **`Set OF File` does NOT become accepted** (Phase 2 said to update
   `rejects_mut_set_of_resource_not_defaultable` to "assert acceptance"). The
   fix does make `Set OF T` defaultable, but `Set OF File` stays rejected on two
   INDEPENDENT axes the bug's own Non-goals kept: `TYPE_COLLECTION_OWNERSHIP_VIOLATION`
   (an ordinary collection cannot own a resource) and `TYPE_REQUIRES_COMPARABLE`
   (File is not comparable). Verified against the release binary that
   `MUT s AS Set OF File = []` is likewise rejected on those axes — disproving the
   Open-Decision premise that "`= []` is already legal, so it is safe." The test
   was therefore renamed `rejects_mut_set_of_resource_ownership_and_comparable`
   and asserts (a) the defaultability rule no longer fires and (b) the ownership
   rule still does — preserving the "Set OF File is rejected" protection.

2. **`accepts_recursive_record_through_list` needed no change** (Phase 2 flagged
   it "may need adjustment"): it only declares the recursive-through-`List`
   record and never binds a `MUT` of it, so it never exercised defaultability and
   stays green untouched.

Tree-wide `cargo fmt --all` (skill §9) was NOT run: main carries pre-existing
unformatted files (e.g. `src/target/win_x86_64/app/mod.rs` fails
`rustfmt --check` at HEAD, untouched by this fix), so a full-repo format would
sweep unrelated churn into this bug. The two changed Rust files pass
`rustfmt --check` individually, satisfying §9's intent.

A `MUT` binding may omit its initializer only when its type has a defined default value. The
current rule makes a `List OF T` defaultable **only when `T` is defaultable** (same for `Set OF T`
and `Map OF K TO V`), so `List OF <union>`, `List OF <enum>`, `List OF FUNC(...)`, etc. are rejected
as non-defaultable — and that rejection **cascades into any record that embeds such a field**. This
is wrong: the default of a collection is the *empty* collection, which materializes **no element**,
so the element type's defaultability is irrelevant. The empty value is already a legal, constructible
value (`MUT xs AS List OF Attribute = []` compiles today), so the compiler refusing to *supply* it
as the default is the defect, not a missing capability.

The single correct behavior a fix produces: **`List OF T`, `Set OF T`, and `Map OF K TO V` are
always defaultable — default `[]` / empty set / empty map — regardless of `T` / `K` / `V`.**

References:

- Spec: `src/docs/spec/language/04_types.md` §4.7 (Collections, the `List OF FUNC` note) and §4.10
  (Default Values — the predicate prose at line 426 and the table at 419-421); §4.5 Pair/Partition
  (line 371); `src/docs/spec/language/05_bindings-and-scope.md:41-43`.
- Diagnostic: `2-203-0060 TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE`
  (`src/docs/spec/diagnostics/01_rule-codes.md:377`).
- Found while designing the `AttributedString` feature (`List OF Attribute`, where `Attribute` is a
  union): an opaque container could be granted a default, but a plain record with a `List OF
  <union>` field cannot be, which surfaced the rule as the real blocker.

## Failing Reproduction

Verified today against `target/release/mfb` on macos-aarch64 (temp project `mfb init`):

```
' src/main.mfb
IMPORT io
ENUM AttrTypeFlag { Bold  Italic END ENUM       ' (multi-line in real source)
TYPE AttrFlag   { attr AS AttrTypeFlag }
UNION Attribute { AttrFlag }
TYPE Doc        { text AS String  attrs AS List OF Attribute }

SUB main()
  MUT xs AS List OF Attribute      ' (1) bare collection
  MUT d  AS Doc                    ' (2) record embedding it
END SUB
```

- Observed (1): `error[2-203-0060 TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE]: ... type 'List OF Attribute'
  does not have a defined default value.`
- Observed (2): `error[2-203-0060 ...]: ... type 'Doc' does not have a defined default value.` — the
  non-defaultability cascades into any containing record.
- Expected: both compile; `xs` defaults to `[]`, `d` defaults to `Doc["", []]`.

Contrast cases that work correctly today (bounding the bug):

- `MUT xs AS List OF Attribute = []` — compiles and links. Proves `[]` is a legal value of the type;
  only its *automatic supply as a default* is refused.
- `LET d = Doc["hi", []]` — compiles and links. Explicit empty construction is fine.
- `MUT n AS Integer`, `MUT s AS String`, `MUT xs AS List OF Integer` — already defaultable (element
  is defaultable), unaffected.

| Environment | arch | Result |
| --- | --- | --- |
| macOS | aarch64 (release binary) | fails ✗ |

(Not platform-specific — this is a front-end semantic check in `ir::verify`, identical on every
target.)

## Root Cause

`src/ir/verify/resources.rs:is_defaultable` (line 298) decides defaultability structurally, and the
collection arms **recurse into the element type** instead of short-circuiting to the empty value:

```
302  if let Some(element) = type_.strip_prefix("List OF ") { return self.is_defaultable(element, seen); }
305  if let Some(element) = type_.strip_prefix("Set OF ")  { return self.is_defaultable(element, seen); }
309  if let Some((k, v)) = parse_map(type_) { return self.is_defaultable(k, seen) && self.is_defaultable(v, seen); }
```

So `List OF Attribute` → recurse into `Attribute` → union → line 320-324 returns `false`. The record
arm (line 329-330, `fields.iter().all(...)`) then propagates that `false` up to `Doc`. The rule is
conservative-by-uniformity ("non-defaultable element ⇒ non-defaultable collection"), but an empty
collection never materializes an element, so the recursion is unwarranted. The spec text even
contradicts itself: §4.10 (line 426) says "a `Set OF T` is defaultable when `T` is defaultable **(the
empty set)**" — the parenthetical concedes the default has zero elements while the condition still
demands a defaultable `T`.

Consumers of the predicate (all route through the one function):
- `src/ir/verify/ops.rs:271` — local `MUT` without initializer → `TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE` (target).
- `src/ir/verify/mod.rs:424` — global `MUT` without initializer → same (target).
- `src/ir/verify/ops.rs:237` — `RES … STATE T` requires `T` defaultable → `TYPE_STATE_INVALID` (ripple).
- `src/ir/verify/calls.rs:286` — `FUNC … return STATE T` → `TYPE_STATE_INVALID` (ripple).

Codegen is **already correct** and needs no change: `src/target/shared/code/builder_value_semantics.rs:142-149`
(`lower_default_value`) lowers any collection type via `lower_empty_collection`, and record-field
default synthesis (line 250-251) recurses field-by-field — collections lower to empty *without*
recursing into the element type, so recursive-through-`List` records (`Tree`) terminate. The block is
purely the front-end predicate refusing to *ask* for the default that codegen can already produce.

## Goal

- `is_defaultable` returns `true` for every `List OF T`, `Set OF T`, and `Map OF K TO V`,
  unconditionally.
- `MUT xs AS List OF <non-defaultable>` and any record embedding such a field compile, defaulting to
  the empty collection.
- The corresponding `RES … STATE <collection>` and `FUNC … return STATE <collection>` become valid
  (intended ripple, allowed — falls out of the shared predicate at no extra cost).
- Every collection is covered with **no exceptions**, including resource-element collections
  (`Set OF File`, `List OF RES X`) — they default to the empty collection.
- No change to bare non-defaultable types: `MUT` of a bare enum, union, `FUNC(...)`, resource, or a
  record with a *direct* non-defaultable (non-collection) field still errors with `2-203-0060`.

### Non-goals (must NOT change)

- Enums, unions, functions, threads, resources, and the internal Result type remain non-defaultable
  as *bare* types (§4.10). This bug does **not** touch the enum-defaults-to-first-member idea — that
  was considered and dropped; it would not even fix this case, since `Attribute` is a *union*.
- Runtime/codegen default materialization (`lower_default_value` / `lower_empty_collection`) — it is
  already correct; do not modify it to "support" this.
- Comparability, map-key-comparability, and Set-element-comparability rules are independent and must
  stay put (an uncomparable element still can't be a `Set`/`Map`-key element — a different check).
- **Tempting wrong fix, forbidden:** do not special-case the opaque `AttributedString` container to
  paper over this. The reproduction case (2) proves a *plain* record inherits the block, so the fix
  must be the general collection rule, not a per-type default grant.
- Do not silently re-baseline the existing diagnostic goldens/tests. The primary golden
  (`mut-default-eligibility-invalid`) must remain red on all five of its lines — none of them are
  collections; if any flips, the fix is too broad.

## Blast Radius

Found by search over `is_defaultable` callers, `2-203-0060`, and the defaultability tests.

- `src/ir/verify/resources.rs:is_defaultable` (List/Set/Map arms) — **fixed by this bug**.
- `src/ir/verify/ops.rs:271`, `src/ir/verify/mod.rs:424` (local/global MUT) — target consumers;
  behavior expands to accept more programs. Additive.
- `src/ir/verify/ops.rs:237`, `src/ir/verify/calls.rs:286` (STATE) — **in-scope intended ripple**:
  `STATE List OF <non-defaultable>` becomes valid (an empty list is a valid initial state). Needs a
  STATE test to lock it in.
- `Partition OF T` (spec §4.5, a record `{matched: List OF T, unmatched: List OF T}`) — becomes
  *always* defaultable (both fields are lists). Spec text update only; no code (handled by field
  recursion once list arms return true). `Pair OF A, B` is unaffected (its fields are `A`/`B`
  directly, not lists).
- `src/target/shared/code/builder_value_semantics.rs:lower_default_value` + `lower_empty_collection`
  and callers (`builder_control.rs:333,500`) — **unaffected / already correct**; more types now reach
  them, but empty-collection lowering is already implemented. No infinite recursion for
  recursive-through-`List` records (collections lower to empty without recursing into the element).
- `mut-default-eligibility-invalid` golden and `types-declaration-shapes-invalid` golden
  (`MUT handle AS File`, a bare resource) — **unaffected**; both remain red for the right reasons
  (bare non-defaultable types, not collections).

## Fix Design

In `src/ir/verify/resources.rs:is_defaultable`, replace the three recursing collection arms with:

```rust
if type_.starts_with("List OF ") { return true; } // empty list, no element to default
if type_.starts_with("Set OF ")  { return true; } // empty set
if parse_map(type_).is_some()    { return true; } // empty map
```

(Keep them ahead of the FUNC/RES/STATE and union/enum arms so the collection short-circuit wins.)
Codegen already materializes these; no other code changes are required for the core fix.

Rejected alternatives:
- *Grant only the opaque `AttributedString` an empty default.* Rejected: reproduction (2) proves the
  block hits any record with a `List OF <non-defaultable>` field, not just our type; it would leave
  the general footgun in place.
- *Make enums default to their first member.* Rejected: separate concern, has a reorder footgun, and
  does not fix this case (`Attribute` is a union, still non-defaultable even with defaultable enums).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add a new valid golden `tests/syntax/types/mut-default-collection-of-nondefaultable-valid/`
      binding `MUT` of `List OF <union>`, `Map OF String TO <union>`, and a record embedding a
      `List OF <union>` field; confirm it currently FAILS with `2-203-0060`. (Dropped a `Set OF
      <union>` binding: a Set element must be comparable, so it fails on `2-203-0061`, not `-0060`.)
- [x] Add ir/verify unit tests (`src/ir/verify/tests.rs`) for the same, plus a `STATE List OF
      <non-defaultable>` case; confirm current failures. (All 4 RED on `2-203-0060` / `TYPE_STATE_INVALID`.)
- [x] Confirm the existing diagnostic golden `mut-default-eligibility-invalid` and unit tests
      `rejects_mut_enum_not_defaultable`, `rejects_mut_func_not_defaultable` (bare FUNC),
      `rejects_mut_record_with_nondefaultable_field` (direct field) still red for the right reasons.

Acceptance: new tests fail with `2-203-0060`; the untouched-behavior tests documented.
Commit: 6ff17fd39 (tests landed with the fix)

### Phase 2 — the fix

- [x] Edit `src/ir/verify/resources.rs:is_defaultable` — List/Set/Map arms `return true`.
- [x] Update `rejects_mut_set_of_resource_not_defaultable` — renamed
      `rejects_mut_set_of_resource_ownership_and_comparable`. NOT "assert acceptance" (the plan was
      wrong): `Set OF File` stays rejected on the independent ownership + comparability axes; the
      test now asserts the defaultability rule no longer fires AND the ownership rule still does.
- [x] Re-check `accepts_recursive_record_through_list` (`tests.rs:3111`): no change needed — it only
      declares the record, never binds a `MUT` of it, so it never exercised defaultability; stays green.

Acceptance: Phase 1 tests pass; the Non-goals tests remain red; `cargo test --bin mfb` green.
Commit: 6ff17fd39

### Phase 3 — spec sync + regenerate goldens + full validation

- [x] Update `src/docs/spec/language/04_types.md`: §4.10 predicate prose and table to "always
      defaultable (empty)"; the §4.7 `List OF FUNC` note; §4.5 Partition. Updated
      `05_bindings-and-scope.md` with a collection-always-defaultable note. (`2-203-0060` unchanged.)
- [x] Generated the new positive fixture's goldens (`sync-goldens.sh` refreshes existing files; the
      three golden files were seeded empty then populated). The two invalid goldens
      (`mut-default-eligibility-invalid`, `types-declaration-shapes-invalid`) are byte-unchanged
      (neither references a collection; test-accept `syntax/*` = 594 pass).
- [x] Full suite: `cargo test --bin mfb` (3790 passed), artifact-gate `all` (1202 tests, 0 diffs),
      test-accept `syntax/*` (594 pass).
- [x] Re-ran the original reproduction end-to-end; both bindings compile, link, and run (exit 0).
      Plus a runtime proof: `MUT xs AS List OF <union>`, append, `len(xs)==1`.

Acceptance: full suite green; the invalid diagnostic goldens are byte-unchanged; the reproduction
passes.
Commit: 3df1d5d6f (spec sync); goldens+fixture in 6ff17fd39

## Validation Plan

- Regression test(s): the new valid golden + ir/verify unit tests (list-of-union, map-value-union,
  record-with-list-field, STATE-of-collection).
- Runtime proof: build+link a program that binds `MUT xs AS List OF <union>` and appends an element,
  proving the empty default is a usable list, not just an accepted type.
- Doc sync: `04_types.md` §4.5/§4.7/§4.10, `05_bindings-and-scope.md`; rule-code table unchanged.
- Full suite: `cargo test --bin mfb` + artifact-gate + `tests/syntax` test-accept.

## Open Decisions (RESOLVED)

- **Resource-element collections — RESOLVED: uniform.** *All* collections are defaultable to their
  empty form, with **no exceptions** — including `Set OF File` / `List OF RES X` (empty owns nothing
  → cleanup is a no-op; `= []` is already legal, so it is safe). No element guard. Phase 2 therefore
  *updates* `rejects_mut_set_of_resource_not_defaultable` to assert acceptance rather than rejection.
- **STATE ripple — RESOLVED: allow (trivial).** `RES … STATE <collection>` and `FUNC … return STATE
  <collection>` become valid, and this costs zero extra code: the STATE checks (`ops.rs:237`,
  `calls.rs:286`) call the same `is_defaultable` predicate, and STATE default materialization uses the
  same `lower_default_value` that already lowers empty collections. An empty collection is a valid
  initial state, so it is kept. (Suppressing it would have been the *complex* path — a separate
  guard — and is explicitly not done.)

## Summary

The engineering risk is not the fix (three arms → `return true`; codegen already materializes empty
collections) but the *ripple accounting*: the STATE consumers and resource-element collections change
behavior through the shared predicate, and the spec + a handful of unit tests that encode today's
deliberate rejections must be updated with the spec edit as justification — never silently
re-baselined. The invalid diagnostic goldens for bare non-defaultable types stay untouched and are
the guard that the change didn't over-reach.
