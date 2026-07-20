# plan-57-E: retire the dead paths and decide the remaining element types

Last updated: 2026-07-19
Effort: small (<1h)
Depends on: plan-57-D (the representation is live)

Closes plan-57 out. Three jobs, none of which should ride inside a sub-plan that
changes behavior:

1. Delete the code that plan-57-D made unreachable, rather than leaving it behind
   a "consumed by a later phase" comment.
2. Decide, with an audit rather than a guess, whether function values and pointer
   payloads join the fixed-width set.
3. Reconcile plan-57 with `bugs/bug-365-*` and `bugs/bug-333-*`, both of which
   this feature partially resolves and neither of which it closes.

The single behavioral outcome: no dead entry-table code remains on a fixed-width
path, `cargo check --all-targets` is clean with no new `#[allow(dead_code)]`, and
the two open bugs' scopes are accurate.

References (read first):

- `planning/plan-57-D-kind-2-drop-the-entry-table.md` §4.4 — the list of what
  became dead.
- `bugs/bug-365-linear-data-region-readers-ignore-entry-order.md` — still open for
  `List OF String` after plan-57-C. Its §Scope worklist should have been triaged
  by plan-57-B Phase 1.
- `bugs/bug-333-string-collection-builder-duplication.md` — plan-57-A/B retire
  several of its items; the rest stand.
- `src/target/shared/code/builder_search.rs:876-908` — `mid`'s order probe.
- `src/target/shared/code/codegen_utils.rs:8-127`
  (`lower_sort_string_list_helper`) — permutes entry records deliberately for
  `fs::listDirectory`; the clearest reason bug-365 survives plan-57.
- `AGENTS.md` — the dead-code rule: no blanket suppression, no "consumed by a
  later phase" promises (bug-326 found a dozen that had gone stale).

## 1. Goal

- Every path made unreachable by plan-57-D is **deleted**, not suppressed.
- `mid`'s order probe and `slice`'s normalizing repack keep only their
  variable-width arms.
- A written decision, backed by an ownership audit, on whether function-value and
  pointer payloads (both 8-byte fixed) join the fixed-width set.
- bug-365's scope reduced to variable-width lists, with its per-operation table
  updated and the fixed-width rows marked resolved-by-plan-57-C.
- bug-333's item list updated to reflect what plan-57-A/B actually collapsed.
- The spec's Payload Order contract (drafted in bug-365) states which
  representation guarantees order, now that one of them does by construction.

### Non-goals (explicit constraints)

- **Do not fix bug-365's variable-width half here.** That is its own change, with
  its own tests. This sub-plan only corrects the bug's scope.
- Do not widen the fixed-width predicate without the ownership audit. A wrong
  answer here is a double free, not a wrong number.
- No further representation changes. Maps keep the entry table permanently; that
  is not a deferred item, it is the design.

## 2. Current State (expected, after plan-57-D)

Dead or half-dead on fixed-width paths:

| item | site | why dead |
|---|---|---|
| order probe | `builder_search.rs:876-908` | fixed-width lists are always ordered |
| `slice` offset rewrite | `builder_collection_queries.rs:1275-1310` | nothing to normalize |
| entry-fill loop | `emit_alloc_list` fixed arm | no entries |
| `flags` USED guard | `builder_arena_transfer.rs:727-735` | never reachable; nothing clears USED |
| `emit_offset_compaction_fixup` | `builder_collection_mutate.rs:3714-3757` | fixed arm has no offsets to fix |

`builder_arena_transfer.rs:727` deserves a specific decision: it was already dead
before plan-57 (no code path anywhere clears the `USED` bit — all 32 references to
`COLLECTION_ENTRY_FLAG_USED` are stores of `1` or comparisons against `1`, and
`removeAt` compacts rather than tombstoning). It is a guard against a tombstone
representation that does not exist. Either delete it, or keep it with a comment
saying what would make it live — do not leave it unexplained.

## 3. Design Overview

Straightforward removal work, with one genuine question (§4.2) and one
documentation obligation (§4.3).

**Where the risk concentrates:** deleting a path that is dead for fixed-width
lists but live for variable-width ones. Every item in the §2 table is a *branch*
that became unreachable in one arm, not a whole function. Deleting the function
would break `List OF String`. Read each site's other arm before touching it.

## 4. Detailed Design

### 4.1 Deletions

For each §2 item, remove the fixed-width arm and keep the variable-width one.
Where a whole helper becomes unused, delete it outright — `cargo check
--all-targets` reports it immediately, since the tree carries no blanket
`#![allow(dead_code)]`.

### 4.2 Function values and pointer payloads

Both are 8-byte fixed-width, so they *look* like they belong in the kind-2 set.
They were deliberately excluded (plan-57-C §4.1) because they carry ownership.

The audit that decides it:

- Does the scope-drop path walk a list's entries to free per-element owned
  values, or does it free the block wholesale? (`Scope-drop frees`,
  `ActiveCleanup::OwnedValue`.)
- Does `collection_needs_transfer_fix`
  (`builder_arena_transfer.rs:531-547`) return true for these element types? If
  so, thread transfer walks entries and reads `flags` — the one live-ish entry
  read in the tree.
- Is a function value in a list a bare code pointer or a closure-environment
  pointer that participates in drop?

If every answer is "the block is freed wholesale and transfer does not walk
entries," they can join the set for a further memory win. If any answer involves
per-entry ownership, they stay `kind = 0` **and the reason is recorded here**, so
the question is not reopened blind.

Recommendation absent the audit: leave them out. The memory win is 48→8 bytes per
element rather than 41→1, and the downside is a double free.

### 4.3 Documentation

- `src/docs/spec/memory/05_collections.md`: the Payload Order contract drafted in
  bug-365 must now distinguish the two representations — a `kind = 2` list
  guarantees index order by construction; a `kind = 0` list does not, and a reader
  must still go through `entry[i].valueOffset`. Without that distinction the
  contract reads as though the hazard is gone, when it is gone for exactly half
  the element types.
- `bugs/bug-365-*`: retitle/rescope to variable-width lists; update the
  per-operation table; keep it open.
- `bugs/bug-333-*`: mark the collection-builder items plan-57-A/B collapsed.

## Compatibility / Format Impact

Nothing changes. Deletion of unreachable code and documentation corrections only.
If any deletion changes generated output, the code was not dead — investigate
rather than accept the diff.

## Phases

### Phase 1 — deletions

- [ ] Remove the fixed-width arms listed in §2, keeping every variable-width arm.
- [ ] Resolve `builder_arena_transfer.rs:727-735` explicitly: delete, or keep with
      a comment stating what would make it live.
- [ ] `cargo check --all-targets` clean, with **no new `#[allow(dead_code)]`**.

Acceptance: `scripts/artifact-gate.sh` byte-identical — dead code emits nothing,
so removing it must change nothing. Any diff means a deleted path was live.
Commit: —

### Phase 2 — the widening audit

- [ ] Answer the three questions in §4.2 from the code, citing sites.
- [ ] Record the decision and its evidence in this document.
- [ ] If widening: extend `list_element_is_fixed_width`, add the payload-size
      arms, and add a runtime fixture covering construction, read, copy, drop and
      thread transfer for a `List OF <function type>`.

Acceptance: a written, cited decision. If widening, the drop and transfer fixtures
pass on every target — a double free is the failure mode and it is not always
immediate.
Commit: —

### Phase 3 — documentation reconciliation

- [ ] Amend the Payload Order contract in
      `src/docs/spec/memory/05_collections.md` to distinguish the two
      representations (§4.3).
- [ ] Rescope `bugs/bug-365-*` to variable-width lists; update its table.
- [ ] Update `bugs/bug-333-*`'s item list.
- [ ] `cargo test --bin mfb spec`; confirm no leaked `[[` markers.

Acceptance: the spec states the ordering guarantee per representation; bug-365
accurately describes what remains; bug-333 does not claim duplication that no
longer exists.
Commit: —

## Validation Plan

- Tests: none new unless Phase 2 widens the set, in which case the drop and
  thread-transfer fixtures are mandatory.
- Runtime proof: not applicable to Phases 1 and 3 (deletion and documentation).
  Required for Phase 2 if it widens.
- Byte-identity: `scripts/artifact-gate.sh` is the guard for Phase 1 and is a
  strong one — genuinely dead code cannot change output.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Function values and pointer payloads** — see §4.2. Recommend excluding until
  the ownership audit says otherwise; the win is small and the failure mode is a
  double free.
- **Should `lower_sort_string_list_helper` (`codegen_utils.rs:8-127`) be
  revisited?** It permutes entry records and leaves the data region untouched, for
  `fs::listDirectory` determinism — the single clearest example of why bug-365
  survives plan-57. It is correct as written *given* the contract. Recommend
  leaving it and letting bug-365's variable-width fix decide, since changing it
  here would be fixing bug-365 by the back door.

## Summary

Small and mostly mechanical, with one real question. The trap is treating "dead
for fixed-width lists" as "dead" — every item in §2 is one arm of a branch whose
other arm still serves `List OF String`. `artifact-gate` byte-identity catches a
wrong deletion immediately, which is why Phase 1 leads.

Untouched: everything. This sub-plan removes and documents; it does not change
behavior.
