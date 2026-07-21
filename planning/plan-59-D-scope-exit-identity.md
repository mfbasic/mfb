# plan-59-D: Scope-exit pointer identity

Last updated: 2026-07-20
Effort: medium
Depends on: plan-59-B
Produces: a scope-exit cleanup that skips any resource whose pointer matches an
escaping value, so a resource returned (or floated into an escaping collection)
is not closed by the scope it escaped from. Consumed by plan-59-E, which cannot
remove the static escape rule until this exists.

`res.md` §3.3 names this as the machinery Track B needs, and calls runtime
pointer identity "**now the leading candidate**" since fact #10 removed the free
problem. Today the static rule does this job: a resource cannot escape a callee,
so scope exit can close unconditionally. Once `RETURN f` of a parameter is legal
(plan-59-E), scope exit must be able to tell "this resource escaped" from "this
resource is mine to close".

Behavioral outcome: a function that returns a resource it was given closes it
exactly zero times on the way out, and the caller's scope closes it exactly once.

References:

- `planning/res.md` §3.3 — the three options and why pointer identity wins
- `src/target/shared/code/mod.rs:392-398` — `ActiveCleanup` variants
- `src/target/shared/code/builder_codegen_primitives.rs:1606` —
  `emit_resource_block_reclaim`, the existing skip-on-a-flag precedent
- Prerequisites: see plan-59-A.

## 1. Goal

- At scope exit, a resource whose record pointer equals the value escaping the
  scope (a `RETURN`ed resource, or one reachable from a `RETURN`ed collection) is
  **not** closed and **not** reclaimed; its obligation moves to the caller.

### Non-goals (explicit constraints)

- **No whole-program aliasing analysis.** `res.md` §3.3 rejects it: it kills
  separate compilation.
- **No user-visible lifetime or identity annotation.** §15: "There is no
  user-visible lifetime construct."
- **No change to the close-exactly-once guarantee.** After this, a resource is
  still closed exactly once — by whichever scope ends up owning it.
- **No dependence on the closed flag to paper over a double close.** The flag
  makes a second close a defined no-op (plan-59-B), but relying on it to hide a
  premature close would leave the resource closed too early. Identity must be
  correct on its own.

## 2. Current State

Cleanup is driven by `ActiveCleanup` (`src/target/shared/code/mod.rs:392`), whose
variants are `Thread`, `Resource`, `ResourceUnion`, `OwnedList`, `OwnedValue`.
`ActiveCleanup::Resource` is referenced at 10 sites across two files
(`builder_control.rs`, `builder_codegen_primitives.rs`).

There is already a precedent for skipping cleanup on a runtime test:
`emit_resource_block_reclaim` (`builder_codegen_primitives.rs:1606-1630`) loads
the flag word and branches to `resource_reclaim_skip` when the **moved** bit is
set, with a comment stating the guard exists to make the property "a property of
the code rather than of the caller". This sub-plan generalizes that shape from a
flag test to a pointer comparison.

`res.md` §3.3 also records why the signature cannot carry the answer:

> the compiler cannot tell whether `b` aliases `a` or is a fresh resource — the
> signature `AS RES File` does not encode identity. Both bodies are legal under
> one signature.

### Measured populations

| What | Count | Command |
|---|---|---|
| `ActiveCleanup::Resource` reference sites | 10 across 2 files | `grep -rn "ActiveCleanup::Resource" src/ --include="*.rs" \| wc -l` → 10; files: `builder_control.rs`, `builder_codegen_primitives.rs` |
| `ActiveCleanup` variants needing the skip | **3 of 5** — `Resource`, `ResourceUnion`, `OwnedList`. Not `OwnedValue` (copy-insertion makes it unaliased) or `Thread`. | `grep -n "ActiveCleanup::" src/target/shared/code/mod.rs` → 5 variants at `:392-398`; verdicts in C4 |
| Cleanup **dispatchers** every exit path funnels through | **2** — `emit_cleanups` (`:2033`) and `emit_cleanup_branch_to_depth` (`:2055`) | the "10 sites" are pushes/lookups, not emission points; see C3 |
| Existing skip-on-test precedent | 1 (`emit_resource_block_reclaim`) | read at `builder_codegen_primitives.rs:1606-1630` |

### Verified properties

- **A skip-on-runtime-test at scope exit already exists and works.** Verified by
  reading `emit_resource_block_reclaim` in full: it loads the flag, masks the
  moved bit, and branches past the reclaim. The control-flow shape this sub-plan
  needs is proven; only the predicate changes.
- **UNVERIFIED — whether the escaping value is available at cleanup emission
  time.** The comparison needs the returned pointer in hand where cleanup is
  emitted. Whether `builder_control.rs` has it at that point, or whether it must
  be threaded through, is the central unknown. Phase 1.
- **UNVERIFIED — the collection-float case.** A resource reachable from a
  `RETURN`ed `List OF RES T` must also be skipped. Whether that requires walking
  the list at scope exit (O(n) per exit) or can reuse the existing float
  machinery is not established. Phase 3.

## 3. Design Overview

At scope exit, for each `ActiveCleanup::Resource` whose scope is ending:

```
if resource_ptr == escaping_ptr:  skip close and reclaim
else:                             close and reclaim as today
```

`n` is tiny — the number of live resource cleanups in one scope — so the cost is
a handful of compares on a path that already walks a cleanup list.

**Where design uncertainty concentrates:** availability of the escaping pointer at
cleanup-emission time (UNVERIFIED above). Phase 1 is the smallest experiment that
could falsify the whole approach, and it is scheduled first for exactly that
reason.

**Where correctness risk concentrates:** the collection case and the error exits.
A resource must be closed on *every* path — normal exit, `RETURN`, `EXIT`/
`CONTINUE`, `FAIL`, `PROPAGATE`, auto-propagated failure, and `TRAP` routing
(§15). The skip must apply only to the path that actually escapes: on an error
exit *before* the return, the resource has **not** escaped and must still be
closed. §15.6 already states this for collections ("On an error exit *before* the
return, the resources are still closed by the function's scope"). Getting this
wrong leaks on the error path — which is why Phase 4 is last and behind tests.

**Rejected alternatives** (from `res.md` §3.3):

- *Whole-program aliasing analysis* — kills separate compilation.
- *An identity annotation in the signature* ("the returned resource **is**
  parameter f") — ruled out by §15's no-lifetime-construct rule.

## Phases

> **NOTE — keep the checkboxes current as you go.** **An unticked box means NOT
> DONE.**

### Phase 1 — Spike: is the escaping pointer available at cleanup emission?

Falsifies the approach cheaply before any behavior changes.

- [x] Read `builder_control.rs`'s scope-exit path and all 10
      `ActiveCleanup::Resource` sites; determine whether the value being returned
      is in a known slot/register at the point cleanup is emitted, or must be
      threaded in. — **AVAILABLE, in a known stack slot, with no threading
      needed.** See C3.
- [x] Enumerate which `ActiveCleanup` variants need the skip
      (`Resource`, `ResourceUnion`, `OwnedList` at least) and record the list and
      its command in Measured populations. — done; table updated, see C4.
- [x] Write the finding into Corrections. If the pointer is *not* available and
      threading it is large, stop and report — that is a premise failure, and it
      belongs in Prerequisites for a revised plan. — **not a premise failure.**
      The approach is viable as designed; C3, C4, C5.

Acceptance: a written, cited answer to whether the escaping pointer is reachable
at each of the 10 sites, and the variant list is measured.
**MET** — C3 answers reachability with citations and explains why the answer is
the same at all sites (they share two entry points, not ten); C4 measures the
variant list.
Commit: —

### Phase 2 — Single-resource identity skip

- [ ] Emit the pointer compare and skip for `ActiveCleanup::Resource` at scope
      exit, mirroring `emit_resource_block_reclaim`'s skip-label shape.
- [ ] Apply to the `RETURN`-a-resource path only; collections are Phase 3.
- [ ] Tests: `tests/rt-behavior/resources/resource-return-identity-rt` — a
      function that returns a resource it opened (already legal today) must close
      it exactly once, in the caller. Assert via an observable side effect, not
      just exit code.

Acceptance: the new fixture shows exactly one close for a returned resource;
existing `resource-return-ownership-valid` and `resource-state-return-rt` still
pass unchanged.
Commit: —

### Phase 3 — Collections and unions

- [ ] Extend the skip to a resource reachable from an escaping collection
      (`OwnedList`) and to `ResourceUnion`.
- [ ] **Reuse the existing float machinery** (§15.6's ownership-migration rule)
      rather than walking the escaping list at scope exit — DECIDED, see Open
      Decisions. The float machinery already knows which resources a collection
      owns; walking would be O(n) per scope exit and would duplicate that
      knowledge. If Phase 1 finds it does **not** record collection ownership in
      a form reachable here, stop and report rather than falling back to walking:
      that is a premise failure, and walking is the alternative this decision
      rejected.
- [ ] Tests: extend `resource-return-collection-order-rt` and
      `resource-collection-transfer-runtime`.

Acceptance: a returned `List OF RES File` closes each element exactly once in the
caller and zero times in the callee; the three existing collection fixtures pass.
Commit: —

### Phase 4 — Every exit path (largest blast radius, last)

The skip must not apply on a path where the resource did not actually escape.

- [ ] Verify the skip is inert on `FAIL`, `PROPAGATE`, auto-propagated failure,
      `TRAP` routing, `EXIT`/`CONTINUE`, and `EXIT PROGRAM` — on each, the
      resource has not escaped and must still be closed.
- [ ] Pay specific attention to the §15.6 rule: "On an error exit *before* the
      return, the resources are still closed by the function's scope."
- [ ] Tests: an error-exit fixture per path asserting the resource *is* closed
      when the return never happened. Model on `trap-cleanup` fixtures.

Acceptance: a fixture per exit path shows the resource closed exactly once when
the escape does not occur; no leak under a 1000-iteration error-exit loop
(arena-growth assertion, as in plan-52-B).
Commit: —

## Validation Plan

- Tests: new `resource-return-identity-rt`, extensions to three existing
  collection fixtures, and one fixture per error-exit path.
- Coverage check: all `rt-behavior`, so they execute. The leak assertions must
  measure arena growth, not merely exit 0 — a leak is invisible to exit code.
- Runtime proof: a 1000-iteration loop returning a resource through two frames,
  showing bounded arena use (mirroring plan-52-B's 961 MB → 31 MB method).
- Doc sync: §15.6's ownership-float description, and §15's exit-path list.
- Acceptance: `cargo test`; `scripts/test-accept.sh target/debug/mfb <tmp>
  'resource*' 'trap*'` with a hermetic `MFB_HOME`.

## Open Decisions

- ~~**Walk the escaping collection at scope exit, or reuse the float machinery?**~~
  **DECIDED (owner, 2026-07-20): reuse the float machinery.** Walking is O(n) per
  scope exit and duplicates knowledge the compiler already has. This makes
  Phase 1's variant enumeration load-bearing: it must establish that the float
  machinery's record of collection ownership is reachable at cleanup-emission
  time. If it is not, that is a premise failure to report — not a licence to
  walk.

## Corrections

### C1 — native resources have NO scope-exit cleanup to skip (2026-07-20, from plan-59-A Phase 3)

This sub-plan's whole subject is making scope-exit cleanup *skip* a resource that
escaped. For a user-declared `RESOURCE T CLOSE BY nativeOp` — the form every
native binding uses — there is currently **no scope-exit cleanup at all**: no
close call, no reclaim, no diagnostic. Found while executing plan-59-A Phase 3
and filed as **bug-374** (HIGH, Correctness); see plan-59-A Correction C9 for the
evidence, including a 2.92 GB vs 10.4 MB peak-RSS contrast over 20 000
iterations.

Root cause: `resource_cleanup_symbol` resolves through
`builtin_resource_close_function`, an 8-entry map of built-ins only, so
`builder_control.rs:260`'s `else if let Some(symbol) = …` silently falls through
for a user resource and no `ActiveCleanup::Resource` is ever pushed.

**What this changes for this sub-plan:**

- Phase 1's enumeration of `ActiveCleanup` variants is still valid, but it must
  record that the variants are populated **only for built-in resources today**.
  The "10 `ActiveCleanup::Resource` reference sites" figure is a count of
  *emission* sites, not evidence that native resources reach them.
- Phase 2's identity skip will be correct but **inert for native resources**
  until bug-374 lands — there is no cleanup to skip. Any fixture written to prove
  the skip must therefore use a **built-in** resource, or it will pass vacuously
  by skipping nothing.
- This is *not* a blocker for D. The skip is needed for built-in resources
  regardless, and it is the right shape for native ones once they register
  cleanups.

**Sequencing.** bug-374 recommends landing after plan-59-B (whose `closed` flag
makes the resulting second close a defined no-op). If it lands before this
sub-plan, D's fixtures should cover both resource kinds; if after, D's acceptance
must not claim native coverage it does not have.

### C3 — the escaping pointer IS available, already spilled to a known slot (2026-07-20)

§2's central UNVERIFIED property is discharged **positively**. The value being
returned is spilled to a fixed stack slot immediately before cleanups run, and
reloaded immediately after (`builder_codegen_primitives.rs:2100-2115`):

```rust
ExitDestination::Return => self.active_cleanups.clone(),
…
if !cleanups.is_empty() {
    self.store_pending_current_result();   // RESULT_VALUE_REGISTER -> slots.value
    self.emit_cleanups(&cleanups)?;
    self.load_pending_result_registers();
}
```

`store_pending_current_result` (`:1221`) writes `RESULT_VALUE_REGISTER` to
`ensure_pending_result_slots().value`. So at the exact point `emit_cleanups`
runs, the escaping pointer is live in a known `sp`-relative slot. The comparison
this sub-plan needs is a load from that slot and a compare — no threading, no new
plumbing, no signature changes.

**The spill already exists for a reason that guarantees it will keep existing:**
cleanups call helpers that destroy every caller-saved register, so the result
*must* be parked across them or it would be lost. The slot is not incidental to
this sub-plan; it is load-bearing for the existing code.

**"All 10 sites" is the wrong unit, and the plan's framing here is corrected.**
The 10 `ActiveCleanup::Resource` references are not 10 independent emission
points to check. They are pushes, lookups, and two dispatchers — `emit_cleanups`
(`:2033`) and `emit_cleanup_branch_to_depth` (`:2055`) — through which *every*
exit path funnels. The skip therefore has **one** natural home per dispatcher,
not ten, which makes Phase 2 substantially smaller than the plan implies.

### C4 — variant enumeration for the skip (2026-07-20)

`ActiveCleanup` has five variants (`mod.rs:392-398`). Verdicts:

| Variant | Needs the skip? | Why |
|---|---|---|
| `Resource` | **Yes** | The direct case: a `RETURN`ed resource must not be closed by the scope it escaped. |
| `ResourceUnion` | **Yes** | Same obligation, dispatched on a tag; a returned union is equally escaping. |
| `OwnedList` | **Yes** | The collection-float case — a returned `List OF RES T`. Phase 3. |
| `OwnedValue` | **No** | An arena value, not a resource; copy-insertion already guarantees its block is unaliased (`:1604`'s comment), so a returned value is a copy, not the same block. |
| `Thread` | **No** | A thread handle is cancelled/joined, not closed-or-escaped; §15's model does not let one escape as a resource does. |

So three of five, matching the plan's "at minimum" guess exactly.

### C5 — the skip is INERT on error exits by construction, which is Phase 4's whole concern (2026-07-20)

Phase 4 exists to prove the skip does not fire on a path where the resource did
not actually escape (§15.6: "On an error exit *before* the return, the resources
are still closed by the function's scope"). Phase 1 already establishes most of
that argument, and it is worth recording now because it shapes Phase 2's design.

The error exits use a **different** entry point, `emit_error_value_exit`
(`:2122-2145`), which calls `store_pending_error_from_value(error)` — it writes
the **error** into the pending slots, never a resource pointer. Likewise the
`ExitDestination::Trap` arm routes through `route_current_result_to_trap`.

So on an error exit the slot the skip compares against holds an error value, not
the resource's record pointer, and a pointer-equality test cannot match. The skip
is inert on those paths **structurally**, not because it is suppressed there.

This is a strong position for Phase 4, but it is **not a substitute for Phase 4's
fixtures**: "cannot match" rests on an error value never colliding with a live
record pointer, which is true but is a property worth testing rather than
assuming, and `EXIT`/`CONTINUE` route through
`emit_cleanup_branch_to_depth` — the *other* dispatcher, which does **not** spill
a pending result at all. That second dispatcher is where Phase 2 must be careful:
with no escaping value in play, its skip must be unconditionally off rather than
comparing against a stale slot.

### C2 — the `Commit:` lines and populations here are unverified as of 2026-07-20

Recorded so the next reader does not mistake this sub-plan's Measured populations
for re-checked figures. `ActiveCleanup::Resource` → 10 sites was independently
re-run and **confirmed** at execution start
(`grep -rn "ActiveCleanup::Resource" src/ --include="*.rs" | wc -l` → 10). The two
`UNMEASURED` rows remain unmeasured; they are Phase 1's tasks.

## Summary

The engineering risk is not the compare — it is making sure the skip fires on
exactly the escaping path and no other. A skip that is too eager leaks on error
exits; one that is too shy double-closes, which plan-59-B's flag would mask into
a silent early close. Phase 4 exists to prove neither happens.

Untouched: the close-exactly-once guarantee, and the static rules — those come
out in plan-59-E.
