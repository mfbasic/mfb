# Migrate all native `collections::` members to `Implementation::Native` Plan

Last updated: 2026-08-10
Effort: x-large (1d–3d)

<!-- Single file, matching plan-95's precedent; phases are independently-landable
     and byte-verified so it can also be lifted into lettered sub-plans later. -->

Apply the plan-95 machinery to the rest of `collections::`: move each native
member's target-generic lowering out of `src/target/shared/code` into its own
`src/codegen/builtins/collections/func_<name>.rs`, wired through
`BuiltinFunction::native`, and delete the old `CodeBuilder` method plus its
`builder_values.rs` ladder arm(s). The single behavioral outcome: **the compiler
emits byte-for-byte identical native code before and after, on all five
byte-identity targets, for a program exercising every migrated member — while each
member's lowering is reached through `BuiltinFunction.implementation` and no
`lower_collection_*` (or `lower_set_*`) method for a migrated member remains in
`src/target`.**

`collections::get` is already migrated (plan-95). This plan does the remaining
**20 collections-only native members**. `find`/`mid`/`replace` are deferred to an
Open Decision (they share a lowering with `strings::`). The source-generic members
are explicitly out of scope (they have no native lowering to move).

References:

- `planning/completed/plan-95-codegen-extraction-get.md` — the proven pattern and
  the machinery (`Implementation::Native`, `try_native_lower` seam,
  `BuiltinFunction::native`, `func_get.rs`).
- `.ai/testing-gates.md` — artifact-gate / byte-identity (the acceptance gate).
- `src/target/shared/code/builder_values.rs` — the two dispatch sites (normal path
  ~`:722`; inline-raw `lower_inline_builtin_raw` ~`:1795`).
- `src/builtins/mod.rs:native_builtin_target` — the shared-bare-name mapping that
  makes `find`/`mid`/`replace` cross-package.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-95 complete (machinery landed) | `ls planning/completed/plan-95-*` → present | MET |
| Working tree clean | `git status --porcelain` → empty | verify before start |
| Collections byte-identity gate GREEN at HEAD | `bash scripts/artifact-gate.sh target/release/mfb collections` → `0 diff(s)` | verify before start |
| Full suite green at HEAD | `cargo test --bin mfb` → `0 failed` | verify before start |

## 1. Goal

- Every one of the **20 collections-only native members** below is lowered by a
  fn pointer in `BuiltinFunction.implementation` (`Implementation::Native`),
  reached through the dual-path seam — not by a `native == Some("<m>")` arm.
- Each member's lowering body lives in `src/codegen/builtins/collections/func_<name>.rs`
  (a free fn), with its descriptor entry (`BuiltinFunction::native(...)`) co-located.
- `grep -rn 'fn lower_collection_' src/target` and `grep -rn 'fn lower_set_' src/target`
  return **only** methods still used by an unmigrated member (ideally none for the
  20 covered here).
- Byte-identical `.ncode` on all 5 targets throughout; `cargo test --bin mfb` green.
- Runtime proof: a program calling each migrated member prints expected values.

### Non-goals (explicit constraints)

- **No behavior change** — provably-neutral; byte-identity is the gate.
- **`find`/`mid`/`replace` are NOT migrated here** — they share `lower_find`/
  `lower_mid`/`lower_replace` with `strings::` (both `collections.find` and
  `strings.find` map to bare `"find"` via `native_builtin_target`). Removing their
  ladder arm or moving the body would break `strings::`. See Open Decisions.
- **Source-generic members are out of scope** — `sort`, `sortBy`, `distinct`,
  `flatten`, `zip`, `chunks`, `window`, `partition`, `groupBy`, `merge`,
  `mapValues`, `take`, `drop`, `all`, `any`, `findIndex`, `findLastIndex`, `toSet`,
  and the set-algebra ops are implemented in `collections/package.mfb` and
  dispatched via monomorph targets (`#collections_<m>$Type`), not the native
  ladder. They have no single native lowering to carry in `Implementation::Native`.
  (`findLastIndex`'s String fast-path `lower_collection_find_last_index_call` stays
  a `src/target` fast path.)
- **`CodeBuilder` does not move** (plan-95 Open Decision stands) — the `func_*`
  bodies call `CodeBuilder` methods promoted to `pub(crate)`, the accepted
  temporary `codegen → target` edge.

## 2. Current State

A native `collections::<m>` call dispatches by the bare name `m` (from
`native_builtin_target`) at **two** `builder_values.rs` sites — the normal path
(a `if native == Some("<m>")` ladder) and `lower_inline_builtin_raw` (a
`match … { Some("<m>") => … }` for the fallible/callback members that support an
inline `TRAP`). Each arm calls a `CodeBuilder` method (`lower_collection_*` /
`lower_set_*` / etc.). plan-95 removed `get`'s arms and moved its body to
`func_get.rs`; the seam `CodeBuilder::try_native_lower` already routes any member
whose descriptor is `Implementation::Native` ahead of both ladders.

### Measured populations

| What | Count | Command |
|---|---|---|
| `NATIVE_MEMBERS` (descriptor) | 24 | `awk '/const NATIVE_MEMBERS/,/];/' src/codegen/builtins/collections/mod.rs | grep -c '"'` |
| Already migrated | 1 (`get`) | plan-95 |
| Collections-only natives to migrate (this plan) | 20 | 24 − `get` − `find`/`mid`/`replace` |
| Shared with `strings::` (deferred) | 3 | `native_builtin_target` strings branch = find/mid/replace |
| Members with an inline-raw arm too | 8 of the 20 | set, insert, removeAt, forEach, transform, filter, reduce, reduceRight (grep `lower_inline_builtin_raw`) |

**The 20 members → their lowering method (verified from the two dispatch sites):**

| Member | Method | inline-raw arm? |
|---|---|---|
| getOr | `lower_collection_get_or` | no |
| set | `lower_collection_set` | yes |
| append | `lower_collection_append` | no |
| prepend | `lower_collection_prepend` | no |
| insert | `lower_collection_insert` | yes |
| removeAt | `lower_collection_remove_at` | yes |
| removeKey | `lower_collection_remove_key` | no |
| keys | `lower_collection_keys` | no |
| values | `lower_collection_values_builtin` | no |
| hasKey | `lower_collection_has_key` | no |
| contains | `lower_collection_contains` | no |
| sum | `lower_collection_sum` | no |
| add | `lower_set_add` | no |
| remove | `lower_set_remove` | no |
| toList | `lower_set_to_list` | no |
| forEach | `lower_collection_for_each_call` | yes |
| transform | `lower_collection_transform_call` | yes |
| filter | `lower_collection_filter_call` | yes |
| reduce | `lower_collection_reduce_call` | yes |
| reduceRight | `lower_collection_reduce_right_call` | yes |

### Verified properties

- **`find`/`mid`/`replace` are the only strings-shared bare names** — verified by
  reading `native_builtin_target` (`src/builtins/mod.rs:210`): its `strings.` branch
  maps exactly `find`/`mid`/`replace`. `contains` (and all others) are collections-
  only despite `strings::contains` existing (strings' contains lowers elsewhere).
- **Each member's method is not called by any other descriptor member** except the
  shared three — verified per-method at migration time (Phase task).
- **The seam is order-correct**: `try_native_lower` runs before both ladders, so a
  migrated member never reaches its (now-deleted) arm; unmigrated members still
  fall through. Proven by plan-95's `get`.

## 3. Design Overview

Repeat the plan-95 Phase-4 shape, once per member — but in **batches** gated
byte-identical, not one commit per member (20 commits would be noise; group by
shape). For each member `m` with method `lower_<m>`:

1. Measure the method body's `self.<method>(` call set; promote exactly that
   `pub(super)` set to `pub(crate)` (many overlap across members — `emit`,
   `lower_value`, `allocate_stack_object`, the collection-loop helpers — so later
   batches promote fewer).
2. Create `func_<name>.rs`: the body moved verbatim (`self.`→`builder.`) as
   `pub(crate) fn lower_<name>`, plus the member's `BuiltinFunction` entry via
   `BuiltinFunction::native(...)` (doc consts moved from `mod.rs`).
3. `mod.rs`: `mod func_<name>;`; table references `func_<name>::<UPPER>`; drop the
   member's inline entry + doc consts.
4. Delete the `CodeBuilder` method and its ladder arm(s) — the normal-path arm
   always, and the `lower_inline_builtin_raw` arm for the 8 inline-raw members.

**Byte-identity is the gate** (provably-neutral). A diff = a bug to root-cause
(objdump one fixture), never a stop.

**Where risk concentrates.** The callback members (`forEach`/`transform`/`filter`/
`reduce`/`reduceRight`) have the largest bodies and the widest visibility surface
(loop helpers, callback-failure exits) — schedule last among the migratable set.
The **only design uncertainty is the shared three** — deferred to an Open Decision,
not on this plan's critical path.

**Rejected alternative:** dispatch-only migration (Native points at a shim, body
stays in `src/target`). Rejected as the default — it doesn't achieve "lowering in
codegen." (It IS the fallback shape offered for the shared three; see Open Decisions.)

## Phases

Ordered simplest/lowest-surface first, callback (widest surface) last; each phase
is a byte-verified batch.

### Phase 1 — Simple collections-only members (no inline-raw, no callback)

`getOr, append, prepend, removeKey, keys, values, hasKey, contains, sum, add,
remove, toList` (12). Single normal-path arm each.

- [x] All 12 migrated to their own `func_*.rs` (entry + docs + lowering via `BuiltinFunction::native`), methods + both arms deleted, surface promoted, citations repointed. Landed in 3 byte-verified sub-batches: Set (`add`/`remove`/`toList`), delegators (getOr/append/prepend/removeKey/keys/values/hasKey), heavy inline (contains/sum).

Acceptance: MET — `artifact-gate.sh collections` 0 diffs (all 5); `cargo test --bin mfb` 3836 passed; 0 warnings.
Commit: 15cde0ffb (Set), a50ecf6bd (delegators), 1f52593e9 (contains/sum)

### Phase 2 — Fallible list mutators (inline-raw arm too)

`set, insert, removeAt` (3). Each has a normal-path AND a `lower_inline_builtin_raw`
arm — delete both; the seam already fires inside the raw-capture wrapper (plan-95).

- [x] Migrated set/insert/removeAt to `func_set`/`func_insert`/`func_remove_at`; both arms deleted each; promoted the list-mutation surface + `CollectionValueSlot`/`PayloadSlot`.
Acceptance: MET — byte-identical (all 5); 3836 tests pass; 0 warnings.
Commit: c9775642a

### Phase 3 — Callback members (widest surface, last)

`forEach, transform, filter, reduce, reduceRight` (5). Callback loops +
`emit_callback_failure_exit`; both dispatch sites.

- [x] Migrated the 5 to `func_for_each`/`func_transform`/`func_filter`/`func_reduce`/`func_reduce_right`; forEach/transform/filter inline, reduce/reduceRight delegate to the kept `lower_collection_reduce_impl`; promoted the callback/loop surface; deleted both arms each. `func_transform` is `pub(crate)` for the source-generic reuse (Corrections).
Acceptance: MET — byte-identical (all 5); 3836 tests pass, 0 warnings. (The collections byte-identity fixture exercises transform/filter/reduce/forEach with lambdas, so the 0-diff gate IS the runtime-behavior proof.)
Commit: 21ecd98fd

### Phase 4 — Resolve `find`/`mid`/`replace` (per Open Decision)

Only after the Open Decision is settled. Not started until then.

- [ ] Per the chosen approach (Open Decisions): either dispatch-only Native shims (bodies stay, arms stay for strings), or a coordinated collections+strings migration to a shared codegen home.
Acceptance: byte-identical (all 5) + suite green; `collections::find`/`mid`/`replace` AND `strings::find`/`mid`/`replace` all correct.
Commit: —

## Validation Plan

- Tests: existing `collections::` unit suites stay green each phase; no golden re-baselined.
- Coverage: `tests/byte-identity/collections` exercises the members (confirm each migrated member appears in the fixture `main.mfb`; add to the fixture if a member is uncovered — a green gate must mean "its bytes are unchanged", not "untested").
- Runtime proof: per-phase `.mfb` program covering that phase's members, output identical to a pre-phase build.
- Acceptance: full `cargo test --bin mfb` + one clean `artifact-gate.sh collections` per phase; end `cargo fmt --all` (both workspaces).

## Open Decisions

- **How to handle the strings-shared `find`/`mid`/`replace`** —
  *Recommend: a small companion sweep that migrates BOTH `collections::` and
  `strings::` `find`/`mid`/`replace` together*, moving the shared `lower_find`/
  `lower_mid`/`lower_replace` bodies to a shared codegen home (e.g.
  `src/codegen/builtins/shared/` or each stays a `pub(crate)` fn both packages'
  `func_*` reference), then removing the ladder arm. *Alternative:* dispatch-only —
  give `collections.find` etc. `Implementation::Native(shim)` but keep the body and
  the ladder arm in `src/target` (strings still needs the arm), a partial migration.
  *Alternative:* leave `find`/`mid`/`replace` on the ladder entirely (out of scope)
  until a dedicated `strings::` migration plan. (§Non-goals, Phase 4)
- **Batch vs per-member commits** — *Recommend: one commit per phase-batch* (12/3/5),
  each byte-verified, vs 20 commits. (§3)
- **Fixture coverage** — confirm every migrated member is exercised by
  `tests/byte-identity/collections/src/main.mfb`; extend it (its own byte-verified
  change) for any uncovered member so the gate is meaningful. (Validation)

## Corrections

- **THIRD dispatch site (plan-95 machinery gap).** There are **three** dispatch
  sites, not two: normal path (`builder_values.rs` ~`:722`), `lower_inline_builtin_raw`
  (fallible inline-TRAP), and **`lower_infallible_member`** (~`:1884`, infallible
  inline-TRAP). plan-95 only added the `try_native_lower` seam to the first two
  (`get` is fallible, so the infallible site was never exercised). Fixed: added the
  seam at the top of `lower_infallible_member`. Every infallible member has an arm
  at BOTH the normal site AND this one — delete both.
- **Set-group visibility promotions** (`pub(super)→pub(crate)`): `observe_float`,
  `materialize_value`, `allocate_register`, `copy_collection_tight`,
  `lower_map_set_in_place`, `lower_map_remove_key`, `lower_map_projection`, and the
  free `type_utils::set_element_type`.
- **Brittle test generalized.** plan-95's `only_get_carries_native_lowering` (get is
  the sole Native member) breaks every batch; renamed to
  `native_lowering_only_in_collections` — the stable invariant is that every
  Native-lowered function is a `collections` member and `get` is among them.
- **Doc-sync per member.** Each migrated member's man page cites its lowering
  symbol; repoint `[[…collection_mutate.rs:lower_set_*]]` → `[[…func_*.rs:lower_*]]`
  (caught by `man_citations_resolve`). So the citation test DOES check the symbol.
- **`func_transform` must be `pub(crate)`, not `mod func_transform;` private.**
  `lower_transform` has internal callers *outside* the descriptor table: the
  source-generic fast paths for `sortBy`/`mapValues`/`groupBy` in
  `src/target/shared/code/builder_collection_queries.rs` reuse it. Migrating it to
  `func_transform.rs` broke those three callers (private-mod resolution). Fix:
  declared `pub(crate) mod func_transform;` and repointed the three callers to
  `crate::codegen::builtins::collections::func_transform::lower_transform(self, args)`.
  The other four callback bodies have no such cross-caller and stay private mods.
- **`reduce`/`reduceRight` share one kept helper.** Both delegate to
  `CodeBuilder::lower_collection_reduce_impl(args, reverse)` (still in `src/target`,
  promoted `pub(crate)`); only the boolean direction differs. `func_reduce`/
  `func_reduce_right` are one-line wrappers, so the fold machinery was NOT duplicated
  into codegen — the seam moves the *dispatch*, the shared impl stays put (mirrors
  the plan-95 CodeBuilder-stays-in-target decision).

## Summary

Mechanically this is plan-95 Phase 4 repeated 20 times, batched into 3 byte-verified
phases by shape (simple → fallible → callback). The one genuine design question —
the strings-shared `find`/`mid`/`replace` — is quarantined behind an Open Decision
and a final phase, off the critical path. Left untouched: the source-generic members
(no native lowering to move) and `CodeBuilder`'s body (plan-95 defers its move).
