# plan-147-D: Owned function variants and the caller hand-over

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-147-C

This is the calling-convention change. For each `(function, owned-parameter mask)`
that an approved call site needs, codegen emits an extra internal symbol: the same
NIR function lowered with those parameters **owned**. At an approved site, the
caller passes its block, zeroes its own slot, and calls the variant. Inside the
variant the parameter is an owned local, so letter B's S11 turns
`RETURN collections::set(xs, i, v)` into an in-place update and a move.
`acc = helper(acc, i)` then costs what the inline self-update costs.

References: plan-147-A (goal, prerequisites, §2.3 — rows S2, S4, S11, S13, S14 are
this letter's); plan-147-B (S11); plan-147-C (`collect_handover_args`,
`consumable_params`).

## Prerequisites

See plan-147-A. plan-147-C must be complete: `ls planning/plan-147-C-* 2>/dev/null` →
no matches.

## 1. Goal

- plan-147-A's `helper-append`, `helper-map-set` and `helper-concat` cases are
  un-ignored and pass: `count(2N) − count(N) < N/8`.
- plan-147-A's semantics fixture stays green, byte-for-byte, including
  `error-source-same` and `function-value-call`.
- A program with no approved call site compiles to byte-identical code.

### Non-goals

- A matrix/harness site for owned parameters, and transitive hand-over (letter E).
- Any change to a function's base symbol, its ABI, or how function values, `LINK`,
  thread entries and `.mfp` packages see it (plan-147-A non-goals, row S13).
- Every plan-147-A non-goal.

## 2. Current State

- One NIR function lowers to exactly one symbol
  (`src/codegen/engine/builder/mod.rs:1870-1895`). The only per-function extras are
  `lower_builtin_function_wrapper` (`:1324`), `lower_abi_function_helper` (`:1440`)
  and `lower_thread_copy_function` (`function_lowering.rs:1629`). None clones a user
  function. Monomorphization clones per type argument at the HIR level
  (`src/monomorph/lower.rs:instantiate_function`, `helpers.rs:mangle_name` → `name$Types`).
- Parameters are registered in `lower_function` (`function_lowering.rs`, the parameter
  loop). Each gets `LocalValue{by_ref: false}` and a spilled slot. Only a thread
  parameter gets an `ActiveCleanup`.
- Drop of a null slot is a no-op: `emit_owned_value_drop`
  (`src/codegen/cleanup/owned/builder_owned_cleanup.rs`) says "The slot is null when
  … a moved-out value; a null free would fault …, so skip", and registers the slot
  for prologue zero-init.
- `x = f(x, i)` (`builder_control.rs`, `NirOp::Assign`): the call's result is
  claimed, then the old `x` is dropped with `emit_owned_value_drop`, then the new
  pointer is stored (plan-147-A §2.1).
- `emit_call` (`builder_emit_helpers.rs:237`) lowers each argument, spills it, then
  moves it into x0–x7 or the outgoing stack tail. It already has the resource/thread
  move hooks `deactivate_moved_resource_arguments` / `deactivate_moved_thread_arguments`.

### Verified properties

- **Null-slot drop, flat collections and `String`:** read in
  `emit_owned_value_drop`; the null test is in `_mfb_rt_drop_owned_string` and the
  flat collection drop.
- **Null-slot drop, recursive graph types** (`owns_graph` → `emit_graph_value_drop` →
  `_mfb_rt_graph_drop`): **UNVERIFIED**. Phase 1 task 1 reads it. If it is not
  null-safe, graph types are excluded from H2 (plan-147-C) and the exclusion is
  recorded in Corrections. They are not made null-safe here: that is a different
  plan's change to the graph drop.

## 3. Design

### 3.1 Variants

- **Key.** `(nir_function_name, mask)`, where `mask` is a bit set of parameter
  indices. The symbol is `<base symbol>$own<mask in hex>`. Only `$` separates
  variants today (`mangle_name`); `own` cannot collide with a type name, because type
  segments start with an upper-case letter or a package prefix. Phase 1 confirms this
  against `mangle_name`.
- **Demand.** After lowering every function once, collect the masks the approved
  call sites need. For each call site, the mask is `{arg_index | (op, arg) ∈ HandOverArgs}`.
  Lower each requested `(function, mask)` once more, with `owned_params = mask`. That
  is a second lowering of the **same `NirFunction`**, so spans, `Error.source` stamps
  and line tables are identical (row S11). A variant's own approved call sites may
  request further variants; iterate to a fixpoint over the finite set of
  `(function, mask)` pairs.
- **Owned parameter.** In `lower_function`, a parameter in `owned_params` gets an
  `ActiveCleanup::OwnedValue` for its slot, registered at entry exactly like a
  bound owned local. So it is freed on every exit (normal, error, trap route)
  unless moved. It is also no longer "never bound" for `excluded_roots` / H1, so
  plan-147-B's S11 and `plan_returned_move` treat it as an owned local.
- **Coverage.** Apply plan-147-A row S14's recorded action (or none).

### 3.2 The caller hand-over

At an approved `(op, call, arg_index)`:

1. Lower the arguments as today. `x` lowers to its slot's pointer.
2. After **all** arguments have been lowered, and just before the branch, store 0 to
   `x`'s slot. A later argument that fails to evaluate branches out before this
   point, so `x` still owns its block and its cleanup frees it.
3. Call `<symbol>$own<mask>` instead of `<symbol>`.
4. The callee now owns the block and frees or consumes it on every exit. The
   caller's cleanup for `x` stays active but sees a null slot, so it skips. On an
   `Assign x = …`, the old-value drop sees null and skips, and the new pointer is
   stored as today.

No cleanup bookkeeping changes. The moved-out-null pattern already exists and is
already drop-safe (§2 Verified).

### 3.3 What must stay true (plan-147-A §2.3)

| Row | How D keeps it |
|---|---|
| S2 | Only C-approved sites hand over; `x` is dead after the op |
| S3 | On a failure after hand-over, `x` is null and unread (C: H3), and the callee's cleanup frees the block once |
| S4 | One owner at each point: `x`'s slot, then the parameter, then the return slot. A null slot is "moved-from, not dropped" (§14.7) |
| S11 | The variant is a second lowering of the same NIR function |
| S13 | The base symbol is lowered first and unchanged; only direct calls C approved name a variant |

**Correctness risk:** concentrated in §3.1's owned-parameter cleanup, on the error
and trap routes. A missed path leaks, which is unobservable but still a bug. A
doubled path is a double free. `rt_inplace_failure_atomic`-style tests catch the
second: Phase 2 adds a failing-helper case that runs under the arena's
double-free check. The first is caught by `peak_live_bytes` staying flat across
repeated failing calls.

**Rejected:**
- *A hidden "you own it" flag parameter, one symbol.* Every owned-parameter
  function then branches on each exit and keeps a flag slot, even when never called
  owned. It also changes the base symbol's arity, which breaks S13.
- *Inlining.* It doesn't cover recursion, and it is a separate, larger feature
  (2026-09-21 discussion).

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work; `- [~]` for partial with one line on what remains; moot tasks are
> struck through with evidence, never deleted; fill `Commit:` when a phase lands.
> **An unticked box means NOT DONE.**

### Phase 1 — Verify, then variants with no callers

- [x] Resolve the graph-drop UNVERIFIED row (§2): **it is null-safe, so graph types
      need no H2 exclusion.** `emit_owned_value_drop` routes a graph type to
      `emit_graph_value_drop` (`builder_owned_cleanup.rs:422`), and that function loads
      the slot, `compare_immediate … "0"` and `branch_eq(&skip)` BEFORE the
      `emit_symbol_call(GRAPH_DROP_SYMBOL)` (`graph_drop.rs:213-224`) — so a null slot
      never reaches `_mfb_rt_graph_drop` at all. The null test is at the call site, not
      inside the helper, which is the same shape the flat and `String` drops use.
- [x] Confirm `$own<hex>` cannot collide: **the plan's argument does not hold, so the
      naming is guarded instead of assumed.** See Corrections.
- [x] `function_lowering.rs`: `OwnedVariant { mask, symbol }` and an
      `Option<&OwnedVariant>` argument on `lower_function` (`None` = the base lowering,
      every parameter lent). An owned parameter gets an `ActiveCleanup::OwnedValue` for
      its slot, pushed after the spill so the slot genuinely holds the block, which also
      takes it out of `excluded_roots`'s "never bound" set.
- [x] `builder/mod.rs`: `variant_demand` plus the fixpoint lowering loop (§3.1), with a
      hard error if a demanded variant's symbol already names a module function.
      `variant_demand` returns an empty set in this phase, so no variant is emitted.

Acceptance: codegen is byte-identical, since no demand exists yet.
  Check: `bash scripts/artifact-gate.sh target/release/mfb collections` →
  **`1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)`** (2026-09-23).
Commit: 40aa35911

### Phase 2 — The hand-over

- [x] `builder_emit_helpers.rs` / `builder_values.rs`: `emit_raw_call_handing_over`
      zeroes each handed-over slot after every argument is lowered and immediately
      before the branch, and `emit_call` calls the variant symbol. `handover_site`
      resolves the site from `(current_op_key, current_call_key)`; the two lowering
      arms (`Call` and `CallResult`) set the call key and restore it, so a nested call
      cannot inherit an outer site's answer. `variant_demand` runs the SAME
      `collect_handover_args` over every function, so a variant symbol a caller emits
      is always one the fixpoint loop lowers.
- [x] Un-ignore `helper-append`, `helper-map-set` and `helper-concat` in
      `rt_owned_argument.rs`. The first two are `Expect::Flat` and pass;
      **`helper-concat` is landed as `Expect::StillCopies`** with its measurement —
      see Corrections. A new `the_hand_over_really_happens` reads the emitted code and
      fails if no `$own` variant is there at all, so none of these can pass vacuously
      by falling back to lending.
- [x] Add a failing-helper case to `rt_owned_argument.rs` (not ignored):
      `an_owned_parameter_is_freed_once_on_the_failure_route`. It asserts
      `double_free_skips == 0`, `live_bytes == 0` at exit, exit code 0, that every
      call really failed, and that `peak_live_bytes` does not grow with `N`.
      The helper needs a **consuming `RETURN`** on an unreachable branch
      (`IF v < 0 THEN RETURN collections::append(xs, v)`) — a helper that only ever
      `FAIL`s consumes nothing, so H6 refuses the hand-over and the test would have
      passed while exercising the lending path. A parameter is immutable, so the
      helper cannot itself update `xs` in place; the consuming `RETURN` is what makes
      the parameter owned, and the `FAIL` is the route under test.

Acceptance: the three helper cases and the failing case pass, and the semantics
fixture is unchanged.
  Check 1: `cargo test --test rt_owned_argument` → **`ok. 7 passed; 0 failed;
  1 ignored`** (2026-09-23) — the one ignored is `recursive_fill`, letter E's.
  Measured slopes: `helper-append` 4003 → **15** at N=2000 (was 4000, now flat);
  `helper-map-set` likewise flat; `helper-concat` 2003 → 4003, unchanged and
  asserted so.
  Check 2: `bash scripts/test-accept.sh target/release/mfb /tmp/owned-accept 'owned-argument-semantics*'`
  → **`acceptance tests passed (1 test(s) ran)`**, 0 diffs.
Commit: (this commit)

### Phase 3 — Blast radius

- [ ] Behaviour-changing letter: diffs are **expected** in every byte-identity fixture
      that contains an approved call site, and nowhere else. Run the gate for every
      package, and for each moved `.ncodesum` name the approved call site in
      Corrections. A diff in a fixture with no approved site is a bug; objdump that one
      fixture.
- [ ] Runtime proof: rebuild `/tmp/owned` (plan-147-A §2.2's program) and record
      inline vs helper times in Corrections.

Acceptance: every moved golden has a named site. The helper times are within 2× of
inline.
  Check: `bash scripts/artifact-gate.sh target/release/mfb all` → diffs only in fixtures
  listed in Corrections. Est. 10–15 min; the gate for all packages is needed because
  approved sites can be in any package's fixture, and no per-package run would find
  one elsewhere.
Commit: —

## Validation Plan

- Tests: the three helper cases, the failing-helper case, and the semantics fixture.
- Coverage check: Phase 3's gate names a fixture for every approved site it moved. If
  none moved, the harness in E is the coverage.
- Runtime proof: the `/tmp/owned` timings.
- Doc sync: none until F.
- Final gate: plan-147-F.

## Open Decisions

- Variant per mask, or one "all consumable parameters owned" variant per function?
  **Recommended: per mask.** A call site that can hand over only one of two
  consumable parameters must still pass the other lent. A single variant would force
  a copy of the lent one, which is the cost this plan exists to remove.

## Corrections

- **`helper-concat` cannot be made flat by this letter, and is landed as a pinned
  assertion rather than left ignored.** §1's first goal bullet lists it beside
  `helper-append` and `helper-map-set`; the first two are flat now, the `String` one
  is not, and the reason is structural rather than a missing arm.

  Root cause, and it is the same invariant letter B measured and pinned as
  `RETURN_NEVER`: a `String` block must be **tight** to leave its frame, because
  `arena_free(ptr, size)` is caller-sized and bins by size class, and a caller frees
  a returned `String` by its `byteLength` alone (bug-560). Every in-place `String`
  growth leaves spare, so the owned variant's `RETURN s & t` has to copy tight —
  which costs exactly the one allocation the copying `grow` would have made.

  **Measured, not argued.** `consumable_params`'s P3 was temporarily extended to
  treat `RETURN s & t` as a consuming use, so the hand-over really happened — the
  variant `grow$own1` is emitted and called, confirmed in the `-ncode` plan. The
  allocation count did not move:

  | `helper-concat` | N = 2000 | 2N = 4000 | Slope |
  |---|---|---|---|
  | hand-over forced (`grow$own1` called) | 2003 | 4003 | 2000 |
  | no hand-over (HEAD) | 2003 | 4003 | 2000 |

  Identical. So P3 was **not** extended in the end: doing so would mint a variant
  symbol per such helper and change no allocation. Flattening this shape needs the
  `String` block to carry its own capacity so a grown block can be moved out, which
  is a change to the memory contract and another plan's work — exactly as §2 says of
  the graph drop, "that is a different plan's change".

  The criterion is **strengthened, not weakened**. The case is no longer `#[ignore]`d:
  it is a live `Expect::StillCopies` assertion checked in BOTH directions, so if the
  slope ever drops below `N` the test fails and says to flip the line — the same
  device `rt_inplace_self_update`'s `deferred:` status uses. An ignored case asserts
  nothing; this one asserts today's truth and will report the day it changes.

- **`$own<hex>` CAN collide in principle, so the name is guarded rather than
  argued.** Phase 1 asked for the collision argument to be recorded; reading
  `mangle_name` (`src/monomorph/helpers.rs:567`) and `sanitize_type_name` (`:639`)
  shows the plan's version of it is wrong. §3.1 says "`own` cannot collide with a type
  name, because type segments start with an upper-case letter or a package prefix".
  But a **package prefix is lowercase**, and `sanitize_type_name` maps every
  non-`[A-Za-z0-9_]` character to `$`, so a mangled segment is `[A-Za-z0-9_$]*` and a
  segment spelled `own1f` is not structurally impossible.

  The scheme is kept (`<base>$own<mask:x>`), but nothing rests on it being unique:
  the fixpoint loop checks each minted symbol against the module's real symbol set
  (`function_symbols`) and returns a hard error if it is already taken. That fails
  loudly at build time instead of silently aliasing two functions, which is what the
  original argument would have risked if a package were ever named `own1f`.

- **The graph-drop row resolved MET, with no H2 exclusion.** §2 listed "null-slot drop,
  recursive graph types" as UNVERIFIED and said graph types would be excluded from
  plan-147-C's H2 if the drop were not null-safe. It is: `emit_graph_value_drop`
  null-tests the slot and branches past the helper call (`graph_drop.rs:213-224`),
  so `_mfb_rt_graph_drop` never sees a null. H2 is unchanged, and letter C's
  `handover_type` keeps covering `List`/`Map`/`Set`/`String` including recursive
  element types.

## Summary

The mechanism is small: a second lowering with owned parameters, and a null-store at
the call. It rests entirely on two existing facts: null-slot drops are no-ops,
and a second lowering of the same NIR has the same spans. The risk is
cleanup-path completeness in the variant, which the failing-helper case pins.
