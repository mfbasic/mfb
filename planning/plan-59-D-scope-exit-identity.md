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
| `ActiveCleanup` variants needing the same treatment | UNMEASURED — `Resource` and `ResourceUnion` at minimum; `OwnedList` matters for the collection-float case | Phase 1's first task |
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

- [ ] Read `builder_control.rs`'s scope-exit path and all 10
      `ActiveCleanup::Resource` sites; determine whether the value being returned
      is in a known slot/register at the point cleanup is emitted, or must be
      threaded in.
- [ ] Enumerate which `ActiveCleanup` variants need the skip
      (`Resource`, `ResourceUnion`, `OwnedList` at least) and record the list and
      its command in Measured populations.
- [ ] Write the finding into Corrections. If the pointer is *not* available and
      threading it is large, stop and report — that is a premise failure, and it
      belongs in Prerequisites for a revised plan.

Acceptance: a written, cited answer to whether the escaping pointer is reachable
at each of the 10 sites, and the variant list is measured.
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

<!-- Filled in during execution. -->

## Summary

The engineering risk is not the compare — it is making sure the skip fires on
exactly the escaping path and no other. A skip that is too eager leaks on error
exits; one that is too shy double-closes, which plan-59-B's flag would mask into
a silent early close. Phase 4 exists to prove neither happens.

Untouched: the close-exactly-once guarantee, and the static rules — those come
out in plan-59-E.
