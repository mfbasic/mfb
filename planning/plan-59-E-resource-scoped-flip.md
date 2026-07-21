# plan-59-E: The model flip — resource-scoped ownership

Last updated: 2026-07-20
Effort: medium
Depends on: plan-59-C + plan-59-D
Produces: the language change itself. Removes
`TYPE_RESOURCE_INVALIDATE_NOT_OWNER` and `TYPE_RESOURCE_ELEMENT_NOT_OWNER`,
making a `RES` a pointer to one resource owned by the outermost scope that
touches it. Final letter — nothing consumes it.

This is the letter that answers the question `res.md` was opened to ask: *"is a
resource owned by a binding, or by a scope?"* Everything before it is
scaffolding that makes the answer safe to give.

Behavioral outcome: this compiles and runs correctly —

```basic
EXPORT FUNC closeSound(RES sound AS SoundFile STATE FileInfo) AS Nothing
  sndLink::closeFile(sound)
END FUNC
```

— which `res.md` §8 lists as the change's headline benefit: *"'Take a handle,
give it back' becomes writable. Not expressible today in any form."*

References:

- `planning/res.md` §1, §3, §8, §9 — Track B in full; read §8's cons before
  starting
- `./mfb spec language resource-management` §15, §15.6
- `./mfb spec architecture escape-analysis` §23
- Prerequisites: see plan-59-A.

## 1. Goal

- A resource is owned by the **outermost scope that touches it**. A `RES` is a
  pointer to the one resource. Any holder of that pointer may close it; the
  owning scope closes it once if nobody already did.
- `TYPE_RESOURCE_INVALIDATE_NOT_OWNER` (2-203-0086) and
  `TYPE_RESOURCE_ELEMENT_NOT_OWNER` (2-203-0100) no longer fire.

### Non-goals (explicit constraints)

- **A resource is still closed exactly once.** The model changes *who* may close
  it, never *how many times* it is closed.
- **`TYPE_USE_AFTER_MOVE` stays** for the cases it can still prove. `res.md` §8
  says the change "loses static use-after-close"; that overstates it — the loss is
  confined to paths through a call that may return the same resource. Straight-line
  `close(a); use(a)` on one binding must still be a compile error. Verified today:
  that probe yields 2-203-0055.
- **No change to `STATE`.** plan-59-C settles it; nothing here revisits it.
- **No new user-visible syntax.** No `BORROW`/`MOVE` annotation, no lifetime
  construct (§15).
- **Retire the codes, do not recycle them.** 2-203-0086 and 2-203-0100 must not
  be reused for a different meaning — follow the `PROJECT_JSON_VALID` precedent
  at `src/rules/table.rs:58-68`, which keeps a reserved row rather than deleting.

## 2. Current State

Two rules implement binding-scoped ownership:

- `TYPE_RESOURCE_INVALIDATE_NOT_OWNER` — **1** emit site
  (`src/ir/verify/mod.rs:2497`), registered at `:146`, asserted by 2 tests
  (`ir/verify/tests.rs:2681`, `:5009`).
- `TYPE_RESOURCE_ELEMENT_NOT_OWNER` — **3** emit sites
  (`src/ir/verify/mod.rs:930`, `:1163`, `:1660`), registered at `:147`, asserted
  by 3 tests (`ir/verify/tests.rs:3661`, `:4582`, `:4595`).

`src/escape.rs` is the escape-analysis pass whose soundness argument rests on the
first rule — its module doc at `:21-22` states a resource "only ever" escapes one
way *because* a non-owning resource cannot escape a callee. That argument is what
this sub-plan replaces with plan-59-D's runtime identity check.

### Measured populations

| What | Count | Command |
|---|---|---|
| `TYPE_RESOURCE_INVALIDATE_NOT_OWNER` emit sites | 1 | `grep -rn TYPE_RESOURCE_INVALIDATE_NOT_OWNER src/ --include="*.rs"` → `mod.rs:2497` (plus `:146` registration, 2 tests, 2 comments) |
| `TYPE_RESOURCE_ELEMENT_NOT_OWNER` emit sites | 3 | same command → `mod.rs:930`, `:1163`, `:1660` (plus `:147`, 3 tests, 2 comments) |
| Syntax fixtures asserting these two rules | 5 | the fixtures re-baselined during the terminology purge: `resource-invalidate-not-owner-invalid`, `resource-non-owner-return-invalid`, `resource-collection-close-floated-invalid`, `resource-collection-not-owner-invalid`, `ownership-collection-resource-invalid` |
| Spec sections describing the removed model | UNMEASURED — §15, §15.6, §23 at minimum | Phase 1's first task |

### Verified properties

- **Straight-line use-after-close is caught by `TYPE_USE_AFTER_MOVE`, a
  *different* rule.** Verified by compiling `fs::close(a); fs::writeAll(a, "hi")`
  → `error[2-203-0055] binding is used after move`. So removing 2-203-0086 does
  not remove straight-line protection; that is why the Non-goal above is
  achievable rather than aspirational.
- **UNVERIFIED — how much `TYPE_USE_AFTER_MOVE` survives once aliasing exists.**
  Its `moved` set is keyed by binding name (`ir/verify/mod.rs:2490-2510`); once
  two names can denote one resource, closing through one must mark the other
  "possibly closed" or the rule reports a false negative. Phase 2's first task.

## 3. Design Overview

Three changes, in order of increasing blast radius:

1. **Delete the two rules' emit sites** and retire their codes.
2. **Re-found `escape.rs`'s soundness argument** on plan-59-D's runtime identity
   check instead of on the escape rule.
3. **Rewrite §15/§15.6/§23** to describe scope ownership without the
   owner-vs-pointer split — `res.md` §8's "one rule instead of two" landing.

**Where correctness risk concentrates:** `TYPE_USE_AFTER_MOVE`'s aliasing
behavior (the UNVERIFIED property). A rule that silently reports *fewer* errors
looks like success. This is the single most dangerous part of plan-59, because
its failure mode is invisible — hence Phase 2 is a dedicated phase with fixtures
that assert the rule still fires where it should, before any rule is deleted.

**Where design uncertainty concentrates:** whether a "possibly closed" state is
worth adding. `res.md` §3.2 accepts the degradation to a runtime
`ErrResourceClosed`; the open question is whether to warn at the point of
uncertainty. See Open Decisions.

**Rejected alternative — keep a weakened escape rule for bare parameters only.**
Reintroduces the owner-vs-pointer split this change exists to delete, and under
the new model the compiler cannot statically tell whether a parameter escapes.

## Phases

> **NOTE — keep the checkboxes current as you go.** **An unticked box means NOT
> DONE.**

### Phase 1 — Inventory the spec surface

Measures the doc blast radius before any code moves.

- [ ] Enumerate every spec section that describes binding-scoped ownership: §15,
      §15.6, §23 (escape analysis), §14.9, `package/12_verifier-rules.md`,
      `threading/08_queue-semantics.md`. Record the count and command above.
- [ ] For each, note whether it states the rule as *behavior* (must change) or as
      *rationale* (must be re-founded on identity).

Acceptance: the spec surface is measured and written into this document with its
command.
Commit: —

### Phase 2 — `TYPE_USE_AFTER_MOVE` under aliasing (before any deletion)

The dangerous part, done first and provably.

- [ ] Read `check_resource_moves` (`ir/verify/mod.rs:2490-2560`) and determine
      what its `moved` set does when two bindings denote one resource.
- [ ] Implement the aliasing behavior: mark a binding "possibly closed" once it
      has passed through a call returning `RES`. **Emit no diagnostic for that
      state** — DECIDED, see Open Decisions. It exists to stop the rule reporting
      a false negative elsewhere, not to be reported itself.
- [ ] Tests: fixtures asserting the rule **still fires** for straight-line
      `close; use`, for `close; use` across a branch join, and inside a loop —
      i.e. every case that works today must keep working.

Acceptance: the three "still fires" fixtures pass *before* either rule is
removed, proving no protection is lost silently. `cargo test` green.
Commit: —

### Phase 3 — Remove the two rules

- [ ] Delete the emit site at `ir/verify/mod.rs:2497` and the three at `:930`,
      `:1163`, `:1660`; remove both names from the registration list at
      `:146-147`.
- [ ] Retire codes 2-203-0086 and 2-203-0100 in `src/rules/table.rs` as reserved
      rows, following `PROJECT_JSON_VALID` (`table.rs:58-68`) — never recycle.
- [ ] Convert the 5 negative syntax fixtures: each asserted a rejection that is
      now legal. Do **not** delete them — turn each into a positive fixture
      asserting the new behavior compiles and runs correctly, preserving what the
      original was protecting.
- [ ] Delete or convert the 5 `ir/verify/tests.rs` assertions
      (`:2681`, `:5009`, `:3661`, `:4582`, `:4595`).
- [ ] Re-found `escape.rs`'s module doc (`:21-22`) on plan-59-D's identity check.

Acceptance: `closeSound` (the `bindings/libsnd` case at `src/lib.mfb:317`)
compiles, and a program calling it closes the `SoundFile` exactly once — verified
by an arena-growth assertion, not exit code.
Commit: —

### Phase 4 — Spec rewrite (largest blast radius, last)

- [ ] Rewrite §15's four-event model, §15.6's collection carve-out, and §23's
      soundness argument for scope ownership. §15.6's `TYPE_RESOURCE_ELEMENT_NOT_OWNER`
      machinery collapses — `res.md` §8's "one rule instead of two".
- [ ] Update `diagnostics/01_rule-codes.md` for the two retired codes.
- [ ] Update `planning/res.md` §9 to record Track B as done and archive per the
      project's convention.
- [ ] Tests: `cargo test --bin mfb spec` — `every_rule_is_documented_in_the_spec`,
      `spec_links_resolve`, `spec_citations_resolve`.

Acceptance: spec tests green; no spec section still describes a resource as owned
by a binding; `./mfb spec language resource-management` reads coherently end to
end.
Commit: —

## Compatibility / Format Impact

- **Changes:** two diagnostics stop firing. Source that was rejected now
  compiles. This is purely permissive — **no previously-valid program changes
  meaning**, which is what makes the flip safe to land in one step.
- **Unchanged:** the `.mfp` ABI, the record layout, `STATE` semantics, and the
  close-exactly-once guarantee.
- Codes 2-203-0086 and 2-203-0100 are retired, never recycled.

## Validation Plan

- Tests: 5 converted syntax fixtures, 3 new `TYPE_USE_AFTER_MOVE` "still fires"
  fixtures, and a `closeSound` end-to-end fixture.
- Coverage check: **critical here.** A rule that stops firing makes negative
  fixtures pass vacuously. Each converted fixture must assert the new *positive*
  behavior (it compiles AND does the right thing at runtime), never merely that
  the error is gone.
- Runtime proof: build `bindings/libsnd` with `closeSound` and run
  `examples/audio` against it — the case that opened this whole line of work.
- Doc sync: §15, §15.6, §23, §14.9, `diagnostics/01_rule-codes.md`,
  `package/12_verifier-rules.md`, `threading/08_queue-semantics.md`.
- Acceptance: `cargo test`; full `scripts/test-accept.sh` (~15 min — poll the
  output file, never rebuild during it).

## Open Decisions

- ~~**Warn at the point of aliasing uncertainty?**~~ **DECIDED (owner,
  2026-07-20): no warning.** Track the binding as "possibly closed" internally so
  the rule does not report a false negative, and keep hard-erroring everywhere
  liveness is still provable — but emit nothing at the pass-through itself. A
  warning on every call returning `RES` would fire on correct code and train
  people to ignore it. Revisit only if real code shows the silence hiding bugs.
- ~~**Do the 5 negative fixtures become positive fixtures or move to
  `rt-behavior`?**~~ **DECIDED (owner, 2026-07-20): convert in place.** Each stays
  in `tests/syntax/resources/` and becomes a positive fixture asserting the new
  behavior, plus one `rt-behavior` fixture for the runtime proof. None is
  deleted — the original intent stays traceable to the same directory, and a
  converted fixture still records what the rule was protecting.

## Corrections

### C1 — Phase 3's `closeSound` citation points into an uncommitted working tree (2026-07-20)

Phase 3's acceptance cites "the `bindings/libsnd` case at `src/lib.mfb:317`". At
HEAD there is no `closeSound` in that file:

```
$ git stash list   # nothing; the change is unstaged in the working tree
$ git show HEAD:bindings/libsnd/src/lib.mfb | grep -c closeSound
0
```

It exists only in an uncommitted working-tree change that was already present
when plan-59 execution began — the same change also adds `openSound`,
`loadFrames`, and `seekFrames`, and alters `sndError`'s signature. The plan's
headline example was therefore written against a dirty tree.

Consequence for this sub-plan: Phase 3 must not assume `closeSound` exists. When
Phase 3 is reached, either (a) that binding work has landed on its own and the
citation is re-pinned to its real committed line, or (b) Phase 3 creates its own
fixture expressing the same `RES` parameter → close shape and cites that. The
acceptance criterion itself is unchanged and unweakened — a `closeSound`-shaped
function must compile and close its `SoundFile` exactly once, proven by arena
growth. Only the citation is in question, not the requirement.

## Summary

The real engineering risk is not deleting the rules — that is 4 emit sites. It is
`TYPE_USE_AFTER_MOVE` quietly reporting less than it used to once two names can
denote one resource. That failure mode is invisible in a green test run, which is
why Phase 2 proves the rule still fires *before* Phase 3 removes anything.

Untouched: `STATE` (plan-59-C), the record layout (plan-59-A), the guard
(plan-59-B), and the close-exactly-once guarantee. What changes is only who is
permitted to close.
