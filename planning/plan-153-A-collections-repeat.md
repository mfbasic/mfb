# plan-153-A: `collections::repeat` — the builtin (value path)

Last updated: 2026-09-24
Overall Effort: large (3h–1d)
Effort: medium (1h–2h)
Depends on: nothing

Add `collections::repeat(list AS List OF T, value AS T, count AS Integer, startAt AS Integer = 0) AS List OF T`.
It returns `list` with the `count` positions starting at `startAt` holding
`value`. Existing elements in that range are overwritten, and the list grows
when the range runs past its end. That covers three everyday shapes with one
call:

- **build:** `collections::repeat([], 0.0, n)`, where today you write a loop of
  `append`s (`examples/wind/src/flow.mfb:seedSwarm` has 11 of them);
- **reset:** `collections::repeat(xs, 0.0, len(xs))`;
- **fill a range:** `collections::repeat(trailLon, g.lon, TRAIL_LENGTH, base)`.
  That last one replaces the loop in wind's `placeParticle`.

This sub-plan ships the builtin through the ordinary value path: it is
correct, every error is raised, and it is documented. **plan-153-B** adds the
in-place arm, so that `xs = collections::repeat(xs, …)` allocates nothing.
Letter order is implementation order.

**Correct behavior.** For `n = len(list)`, `r = collections::repeat(list, v, count, startAt)`:
- `len(r) = max(n, startAt + count)`;
- `r[i] = v` for `startAt <= i < startAt + count`;
- `r[i] = list[i]` for every other `i < n`;
- `list` itself is unchanged, because the builtin is pure.

References:

- `mfb spec language collections` (`src/docs/spec/language/12_collections.md`) — the member list, and the update-helpers-are-pure rule.
- `mfb spec memory collections` (`src/docs/spec/memory/05_collections.md`) — *Capacity Headroom and Growth*, the per-operation sections.
- `.ai/collections.md`, `.ai/compiler.md` (runtime completion gate), `.ai/man-content.md` (the man page this adds).
- Came out of the wind review (bug-689 conversation, 2026-09-24).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| No `repeat` member in `collections` yet | `ls src/codegen/builtins/collections/func_repeat.rs` → no such file | MET (2026-09-24) |
| The registry count test reads 49 | `grep -n "functions().len()" src/codegen/builtins/collections/mod.rs` → `assert_eq!(pkg.functions().len(), 49)` | MET (2026-09-24) |

Everything below assumes both hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue, and again before you decide to stop.
> If you stop, report the status of *all* prerequisites.

## 1. Goal

- `collections::repeat` type-checks for any element type `T`, defaults
  `startAt` to 0, and returns the result defined above.
- It raises `ErrIndexOutOfRange` when `startAt < 0` or `startAt > len(list)`.
  A range that starts past the end would leave a hole with no value to put in it.
- It raises `ErrInvalidArgument` when `count < 0`.
- Both errors are raised before anything is built.
- `mfb man collections repeat` renders a complete page whose examples run.

### Non-goals (explicit constraints)

- **No in-place arm in this sub-plan.** That is plan-153-B. Until B lands, the
  census row for `repeat` is not added. Adding it here with a `deferred:`
  status would braid the two sub-plans together.
- **No `Map` or `Set` overload.** `repeat` means positions, and only `List`
  has positions.
- **`strings::repeat` is unchanged:** its signature, lowering, and its
  in-place row in `STRING_GROW_FNS`.
- **No change to list memory layout or growth constants.**

## 2. Current State

- **Registry entry.** A native member is a `RegistryFunction` registered by
  `func_*.rs::register`. The closest precedent is
  `src/codegen/builtins/collections/func_set.rs` (`lower_set`, `Body::abi_inline`,
  `errors: vec!["ErrIndexOutOfRange"]`, `return_type: ParameterType::Arg(0)`).
  The structs are `Parameter`, `DefaultValue`, `Implementation` and
  `RegistryFunction` in `src/codegen/registry/mod.rs`.
- **The default parameter.** `DefaultValue::Fill { type_name: ParameterType::Integer, expr: "0" }`
  is how `func_find_index.rs` defaults `start`. The IR pads the argument
  (`registry::default_argument_padding`, called from `src/ir/lower.rs`), so
  codegen always sees 4 arguments.
- **Where the call lands.** `registry::abi_inline_lower` dispatches on the
  qualified name, so `collections.repeat` and `strings.repeat` lower
  independently. The short-name collision only matters to the self-update
  matcher, which is plan-153-B's concern.
- **List primitives** live in `src/codegen/collection/list/list_mutate.rs`:
  - `lower_list_set_in_place` (`ErrIndexOutOfRange` raised before any write);
  - `lower_list_append_in_place`;
  - `lower_list_bulk_append_in_place`, which grows once for a whole batch.
  
  `lower_set`'s value path copies the argument first
  (`copy_collection_tight`, `src/codegen/collection/layout/builder_collection_layout.rs`)
  and then runs the in-place primitive on the copy. `repeat` mirrors that.
- **Growth** is `emit_geometric_step` (`src/codegen/collection/buffer/collection_buffer.rs`).
  For fixed-width elements, data capacity is tied to slot capacity by
  `emit_fixed_width_data_capacity`.
- **Error helper.** `raise_error(function_id, "ErrX")`
  (`src/codegen/error/emission/builder_error_emission.rs`) asserts that the
  descriptor declares the code.

### Measured populations

| What | Count | Command |
|---|---|---|
| collections members (registry count test) | 49 | `ls src/codegen/builtins/collections/func_*.rs \| wc -l` → 49 |
| append calls that wind's `seedSwarm` makes per slot | 11 | `awk '/^SUB seedSwarm/,/^END SUB/' examples/wind/src/flow.mfb \| grep -c "collections::append"` → 11 |

### Verified properties

- **`Fill` defaults are padded before codegen.** Read in
  `registry::default_argument_padding`; `func_find_index.rs`'s `start` relies
  on it.
- **No name conflict inside `collections`.** The reserved names are only
  `toMap`, `zipWith` and `filterEntries` (`12_collections.md`, "reserved but
  not exported").

## 3. Design Overview

Put `lower_repeat` in a new `src/codegen/builtins/collections/func_repeat.rs`.
Validation comes first, in this order:

1. `count < 0` → `ErrInvalidArgument`;
2. `startAt < 0 OR startAt > len` → `ErrIndexOutOfRange`.

Then build:

- **Fixed-width element (kind 2):**
  1. Allocate a tight copy sized for `max(n, startAt + count)` slots, using
     the `copy_collection_tight` shape with the bigger count.
  2. Copy the `n` existing payloads.
  3. Store `value`'s payload into slots `[startAt, startAt + count)` with one
     store loop.
  4. Set `count`/`dataLength`.
- **Variable-width element** (`String`, record, union, nested collection):
  1. Copy the list (`copy_collection_tight`).
  2. Loop `i` from `startAt` to `startAt + count - 1`. When `i < n`, run the
     same overwrite as `lower_list_set_in_place`; otherwise run
     `lower_list_append_in_place`.
  3. Each written element is an owned copy of `value`. This is the same
     `materialize_value` path `lower_set` uses for its item.

**Correctness risk** concentrates in the variable-width path. An overwrite
with a differently-sized payload leaves holes (the `set` rules, bug-627), and
every copy of `value` must be owned, never aliased. Reusing the `set`/`append`
primitives, rather than writing new byte-shuffling code, keeps that risk where
it is already tested.

**Gate class:** behavior may change (this is a new builtin). The gate is
rt-behavior and rt-error tests, not byte-identity. No existing `.ncode` golden
should change. A golden diff anywhere means something leaked into shared code,
and it must be root-caused (objdump one fixture).

Rejected alternatives:

- **`repeat(value, count)` (build only).** It can't reset or fill a range, and
  it has no list to update in place. The signature chosen here does both, and
  the build case is simply `repeat([], v, n)`.
- **A `Body::Mfb` source generic** (a loop over `set`/`append`). It is correct
  but costs an interpreted call per element, and plan-153-B's arm would still
  need native code.
- **Allowing `startAt > len` and filling the gap with the default value.** Not
  every `T` has a default (`TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE` exists for this
  reason), so the rule would depend on the element type.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial, with what remains; moot tasks are
> struck through with evidence, never deleted; fill `Commit:` the moment a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — failing tests

These are the tests that define the contract. They fail at HEAD because the
member doesn't exist.

- [ ] Add `tests/rt-behavior/collections/func_collection_repeat/`
      (project.json + `src/main.mfb` + golden), following the directory shape
      of `tests/rt-error/collections/func_collection_set_out_of_range/`. Cover:
      - build from `[]`;
      - reset the whole list;
      - fill the middle;
      - fill running past the end;
      - `count = 0`;
      - `startAt = len` (a pure extend);
      - the `startAt` default;
      - element types `Integer`, `Float`, `String` (a value longer than the
        overwritten one, and a shorter one), a record with a `String` field,
        and `List OF Integer`;
      - the input list printed after the call, to show it is unchanged.
- [ ] Add `tests/rt-error/collections/func_collection_repeat_start_out_of_range/`
      (`startAt = len + 1`, and `startAt = -1`) and
      `…/func_collection_repeat_negative_count/`. Each golden shows the error
      code and that nothing else printed.
- [ ] Add `tests/syntax/collections/func_collection_repeat_invalid/`, the
      mandatory invalid fixture (`.ai/compiler.md` *Validation*). Cover:
      - a wrong argument count (2 args, and 5);
      - a `value` whose type is not `T` (`repeat([1, 2], "x", 3)`);
      - a non-`Integer` `count`;
      - a `Map` as the first argument.

Acceptance: all four directories exist, and they fail at HEAD. The valid
fixtures fail because the member is unknown; the invalid fixture's golden is
not yet the new diagnostic.
  Check: `scripts/test-accept.sh target/debug/mfb target/accept-actual 'func_collection_repeat*'` → the 4 fixtures fail (est. 2 min).
Commit: —

### Phase 2 — registry entry and lowering

- [ ] Add `src/codegen/builtins/collections/func_repeat.rs`, containing:
      - `register`, with the four `Parameter`s (`startAt` defaulted by
        `DefaultValue::Fill { expr: "0" }`);
      - `errors: vec!["ErrIndexOutOfRange", "ErrInvalidArgument"]`;
      - `return_type: ParameterType::Arg(0)`;
      - `Body::abi_inline(lower_repeat)`;
      - `lower_repeat` as designed in §3.
- [ ] Wire it into `collections/mod.rs`: `mod func_repeat;`, and
      `func_repeat::register(&mut pkg)` in the native group.
- [ ] Bump the registry count test from 49 to 50 and its native-member comment
      from 24 to 25.
- [ ] Prose fields per `.ai/man-content.md`: `intro`, `desc` and `example`,
      plus each `Parameter.desc`. The examples show build, reset and
      fill-a-range. Mention `COLLECTIONS_DESC` in the package intro.

Acceptance: the Phase 1 tests pass, and the page renders with runnable
examples.
  Check: `scripts/test-accept.sh target/debug/mfb target/accept-actual 'func_collection_repeat*'` → 4/4 pass (est. 3 min);
  `scripts/man-run-examples.sh collections --run repeat` → every example compiles and runs (est. 1 min);
  `scripts/man-census.sh --memory-scope` → 0 unclassified (est. 1 min).
Commit: —

### Phase 3 — spec sync

- [ ] `src/docs/spec/language/12_collections.md`: add `collections::repeat` to
      the **Native members** list.
- [ ] `src/docs/spec/memory/05_collections.md`: add a `### repeat` section with
      the value path, the growth rule (one grow to `max(n, startAt + count)`),
      error order, and element ownership. Cite `lower_repeat`.

Acceptance: the citations resolve and the spec builds.
  Check: `scripts/spec-census.sh --citations` → `MISS-SYMBOL 0` (est. 1 min); `cargo test --bin mfb spec` → pass (est. 3 min).
Commit: —

## Validation Plan

- Tests: the rt-behavior, rt-error and syntax fixtures from Phase 1, including
  the negative cases.
- Coverage check: `grep -rln "collections::repeat" tests/rt-behavior tests/rt-error tests/syntax` → the four fixture sources.
- Runtime proof: the Phase 1 program's golden (build, reset, fill-past-end
  output).
- Doc sync: the spec pages in Phase 3, and the man page in Phase 2.
- Final gate: runs once, at the end of plan-153-B (the feature is B's to
  finish). This sub-plan's per-phase checks are scoped.

## Open Decisions

- **Name: `repeat`** (the user's proposal) **vs `fill`.** Recommend `repeat`,
  because it mirrors `strings::repeat` and the build case reads naturally.
  `fill` describes the range case better. Decide before Phase 2.

## Corrections

(none yet)

## Summary

The risk is the variable-width path, and the design keeps it inside the
existing `set`/`append` primitives. Nothing about the list layout,
`strings::repeat`, or any other member changes.
