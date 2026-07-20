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

### 4.2 DECISION (2026-07-20): do NOT widen. Not yet.

The three questions, answered from the code:

1. **Scope-drop frees the block wholesale.** `emit_owned_value_drop` computes one
   block size via `emit_flat_block_size` and issues one `arena_free`. There is no
   per-element walk. `OwnedListCleanup` exists but is the §15.6 *resource close*
   obligation, not a payload free.
2. **Transfer does not walk entries for these types.**
   `collection_payload_needs_transfer_fix` returns true only for a record with
   pointer fields, or a non-flat nested collection / union / `Result`. A function
   type matches none of them, so it takes `copy_flat_block`.
3. **A function value is a bare 8-byte pointer with reference semantics** —
   "stored, copied, and read as a bare pointer word with no deep copy and no
   per-value free" (`type_utils.rs:is_function_type`, bug-73).

By §4.2's own stated criterion that is three for three, and they qualify. The
experiment confirms it: widening the predicate and running the whole suite gives
**1038 passed, 0 failures**.

**Widening was still declined, because the evidence is weaker than it looks.**
`List OF FUNC` is exercised by one fixture, 52 lines, using exactly **one**
operation — `collections::get`. Nothing tests append, prepend, insert, removeAt,
set, sort, copy, or thread transfer over a list of function values. A green suite
over one operation says almost nothing about a representation change to the other
twenty, and this sub-plan's own §4.2 names the failure mode as a double free —
which is precisely the class that passes tests and corrupts later.

plan-57-D supplied the decisive precedent. `collections::zip` segfaulted through
a flipped flag, 1038 green acceptance tests and 1265 green goldens, and was
caught only by the benchmark — because its inlined path had *no test that
satisfied its guard*. Widening here would repeat that setup exactly: a
type-predicate-gated path with near-zero coverage.

**What would change the answer:** extend `collection-of-function-rt` to the full
mutation and copy surface, add a thread-transfer case, and re-run the experiment.
Then widen. The win (48→8 bytes per element, 6x) is real and worth having — it
is the coverage, not the analysis, that is missing.

**One thing found while auditing, not resolved here:** question 2's answer means
a `List OF FUNC` crossing a thread boundary is copied wholesale, closure pointers
included, into a different arena. Those pointers address the *source* arena. This
is pre-existing and orthogonal to kind 2 — it behaves identically under both
representations — but it is not obviously sound and nothing in the tree tests it.
Worth its own investigation; not widened in scope here.

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

- [x] Remove the fixed-width arms listed in §2, keeping every variable-width arm.
  **NOTHING TO REMOVE — §2's table is wrong**, in the same way plan-57-D's §4.4
      was wrong, and for the same reason. §2 lists five paths as "dead on
      fixed-width paths". Every one of them is a branch plan-57-D already
      *skips* for kind 2 and that remains live for kind 0:

      | §2 item | actual state |
      |---|---|
      | `mid`'s order probe | branched past via `if let Some(payload) = mid_payload` |
      | `slice` offset rewrite | guarded by `slice_payload` |
      | `emit_alloc_list` entry-fill | the fixed arm never emits it |
      | `emit_offset_compaction_fixup` | guarded by `if kind2_payload.is_none()` |
      | `flags` USED guard | genuinely dead — see below |

      §3 warned about exactly this ("deleting a path that is dead for
      fixed-width lists but live for variable-width ones") and then §2 tabulated
      the paths as if they were dead anyway. **A representation change gated on
      a type predicate adds a branch; it does not retire one.** Both arms stay
      live while the predicate can go either way — which, for `List OF String`
      and every `Map`, is forever.
- [x] Resolve `builder_arena_transfer.rs:727-735` explicitly: delete, or keep with
      a comment stating what would make it live.
  **DELETED**, with the audit in a comment at the site. §2's claim was correct
      here and now verified rather than asserted: that load was the **only** read
      of `COLLECTION_ENTRY_OFFSET_FLAGS` anywhere in the tree. Every other
      reference is a store, and every store writes `COLLECTION_ENTRY_FLAG_USED`.
      Nothing clears the bit — `removeAt` compacts rather than tombstoning — so
      the compare could never fail. It guarded a tombstone representation that
      does not exist, and it predates plan-57.
- [x] `cargo check --all-targets` clean, with **no new `#[allow(dead_code)]`**.
  **Clean**, zero unused warnings, no new allows. `COLLECTION_ENTRY_FLAG_USED`
      is still used by ten other sites, so no constant became unused.

Acceptance: `scripts/artifact-gate.sh` byte-identical — dead code emits nothing,
so removing it must change nothing. Any diff means a deleted path was live.
**MET: 1265 goldens, 0 diffs.** Note this is weaker evidence than it looks — the
gate has no golden covering a pointer-payload thread transfer, so 0 diffs here
partly means "not observed" rather than "unchanged". Verified behaviourally
instead: all 64 thread fixtures pass, and a direct `List OF String` transfer
probe returns the right elements.
Commit: —

### Phase 2 — the widening audit

- [x] Answer the three questions in §4.2 from the code, citing sites.
- [x] Record the decision and its evidence in this document.
- [x] If widening: extend `list_element_is_fixed_width`, add the payload-size
      arms, and add a runtime fixture covering construction, read, copy, drop and
      thread transfer for a `List OF <function type>`.
  **NOT WIDENING** — see §4.2 DECISION. All three questions answer favourably
      and the experiment passes 1038 tests, but `List OF FUNC` is covered by one
      fixture using one operation (`collections::get`), so that green run is not
      evidence about the other twenty. Declined pending coverage, with the
      criterion for reopening written down.

Acceptance: a written, cited decision. If widening, the drop and transfer fixtures
pass on every target — a double free is the failure mode and it is not always
immediate. **MET** (decision recorded, not widening).
Commit: —

### Phase 3 — documentation reconciliation

- [x] Amend the Payload Order contract in
      `src/docs/spec/memory/05_collections.md` to distinguish the two
      representations (§4.3).
  **DONE** — the distinction is now structural rather than a property to be
      maintained: a kind-2 list guarantees index order *by construction* and a
      linear reader cannot be wrong; a kind-0 list carries the full hazard. The
      section states plainly that half the element types are safe and the other
      half are exactly as dangerous as before. (plan-57-D also documented kind 2
      itself — the kind table, a *Fixed-Width Lists* section, and three worked
      examples that had become wrong.)
- [x] Rescope `bugs/bug-365-*` to variable-width lists; update its table.
  **PREMISE STALE — bug-365 was closed before this sub-plan was reached**, and
      correctly: its own table shows no variable-width linear reader exists
      (`fs::pathJoin` reads `entry[i].valueOffset`; `fs::listDirectory` is a
      producer). There is no variable-width half to rescope to. Added instead
      the one genuinely new fact: the fixed-width half is now **unfalsifiable**,
      because those types have no entry table left to disagree with the data
      region. Its ordering invariant is what made plan-57-D safe.
- [x] Update `bugs/bug-333-*`'s item list.
  **DONE** — but the finding inverts this task's assumption. bug-333 does not
      "claim duplication that no longer exists"; plan-57 made every C item's
      duplication *worse*, because each entry-table site now carries a second
      arm. Six sites times two arms. Recorded that C5 is now the highest-value
      item after C1, that any extracted `emit_entry_scan` must take its stride
      from the block KIND, and that plan-57's own estimates of this surface were
      wrong by large factors in both directions — so re-measure before
      scheduling.
- [x] `cargo test --bin mfb spec`; confirm no leaked `[[` markers.
  **48 passed.**

Acceptance: the spec states the ordering guarantee per representation; bug-365
accurately describes what remains; bug-333 does not claim duplication that no
longer exists. **MET**, with the bug-333 clause inverted as described above.
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
