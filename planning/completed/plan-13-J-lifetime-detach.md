# plan-13-J: lifetime, detach, and orphan correctness

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: plan-13-E and plan-13-F (both backends must exist — the model must hold on
each). Feature-wide precondition: plan-13 master §Prerequisites.
Produces: the detach-not-destroy model proven end to end, leak-free.

The `remove`/`close` detach semantics, orphan handling, and per-widget close ops — proven
across both backends.

The single behavioral outcome: every handle is destroyed exactly once at its own binding
drop or explicit close op; detached and orphaned widgets stay valid and re-attach
correctly; there is no double free across `close` + scope drop; and nothing leaks.

This is scheduled late for **blast radius**, not because it is polish. The failure mode
here is a double free or a leak, not a wrong pixel — and it can only be proven once both
backends exist, because "destroyed exactly once" must hold on each.

References (read first):

- `planning/old-plans/superseded-plan-13-A-app-builtin.md` §2 — the detach-not-destroy model this preserves.
- `src/docs/spec/language/15_resource-management.md` — scope-drop ordering, and the rule
  that a resource union's drop is **tag-dispatched** to the active variant's close op.
  Find with `rg -n 'resource union' src/docs/spec/language/15_resource-management.md`.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-13-E has landed | `rg -n 'host_present' src/target/macos_aarch64/` | **NOT MET** |
| plan-13-F has landed | `rg -n 'host_present' src/target/linux_gtk/` | **NOT MET** |
| A leak checker is available for both platforms | `rg -n 'leak' scripts/` | **UNVERIFIED — run it** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> before continuing and again before deciding to stop; report every row if you stop.

## 1. Goal

- `remove`/`close` **detach** rather than destroy; on `close`, every live descendant is
  retained and reparented to an offscreen holder before the native window dies.
- Each registered close op fires **exactly once**: `app::close` (exported) and a window's
  scope drop never double-fire; each internal `app::destroy` overload fires once at its
  widget's scope drop.
- A detached widget re-attaches to a new window and works.
- `app::Widget`'s tag-dispatched union drop is **never reached** — the type is
  parameter-only, so no binding of that type exists to drop.
- Churn in a scoped collection frees every widget at the owning collection's scope exit
  with no accumulation.

### Non-goals (explicit constraints)

- **No new surface.** This unit proves the model 13-C registered; it adds no function.
- **No `app::destroy` in user code.** It is registry-only; an explicit call is an unknown
  function, which is what prevents the double free.
- **Do not "fix" a leak by weakening a close op.** A close op that stops firing is not a
  fix.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| Registered close ops family-wide | **7** (1 exported `app::close` + 6 internal `app::destroy` overloads incl. TextArea and Table) | 13-C §1 + 13-H + 13-I |
| Backends the model must hold on | **2** (macOS, GTK4) | plus headless for the model tests |
| Widget kinds | **7 concrete + 1 union** | `Window`, `Container`, `Button`, `Label`, `Input`, `TextArea`, `Table` |

### 2.2 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| A resource union's drop is tag-dispatched to the active variant's close op | **CONFIRMED** | `15_resource-management.md` |
| A close op must name a concrete type | **CONFIRMED** | which is why there is deliberately no `app::destroy(w AS RES app::Widget)` |
| A resource union carries no `STATE` | **CONFIRMED** | `15_resource-management.md` |
| `app::Widget`'s union drop is unreachable | **CONFIRMED by construction** | the type is parameter-only; no binding of it exists. **Assert it anyway** — the assertion is what keeps it true as 13-H/13-I add variants |
| Every handle is destroyed exactly once | **UNVERIFIED — this is the acceptance criterion** | proven per backend, under a leak checker |

## 3. Design Overview

Detach-not-destroy, with orphan reparenting on window close.

**Where design uncertainty concentrates:** orphan reparenting. A window closing while
descendants are still live-bound is the case where native and MFBASIC lifetimes disagree
most sharply — AppKit and GTK both want to tear down a window's view tree, while MFBASIC
bindings say those widgets are still alive. Retain-and-reparent-to-an-offscreen-holder is
the design; **Phase 1 proves it on one backend before the full matrix is written**.

**Where correctness risk concentrates:** the `close` + scope-drop interaction. `app::close`
is user-callable *and* the window's registered close op, so a program that calls
`app::close(w)` and then lets `w` go out of scope must not fire it twice. That is the one
double-free path the design deliberately leaves reachable, and it is the first thing the
matrix tests.

**Rejected alternative:** *destroy descendants when a window closes.* Rejected — it makes
every widget binding a dangling handle the moment its window closes, which the detach model
exists to prevent.

**Rejected alternative:** *reference-count widgets in MFBASIC.* Rejected: resource
semantics already give exactly-once drop, and a second counting scheme would be a second
source of truth.

## Compatibility / Format Impact

- **Changed:** nothing externally. This unit makes the registered model actually hold.
- **Unchanged:** the surface, the solver, the seam.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — spike: orphan reparenting on one backend

- [ ] `remove`/`close` detach rather than destroy.
- [ ] On `close`: retain + reparent every live descendant to an offscreen holder **before**
      destroying the native window.
- [ ] Prove on macOS: close a window whose `Button` is still bound, then use the button.

Acceptance: a widget outlives its window and remains valid. If the toolkit cannot be made
to release a window without tearing down its subtree, §3 needs redesigning before the full
matrix is written.
Commit: —

### Phase 2 — the exactly-once matrix

- [ ] `app::close` then scope drop → fires **once** (the deliberate double-free path).
- [ ] Each internal `app::destroy` overload fires once at its widget's scope drop.
- [ ] Re-attach a detached widget to a new window.
- [ ] Scope-drop teardown ordering (reverse declaration order).
- [ ] Churn in a scoped collection: every widget freed at the collection's scope exit, no
      accumulation.
- [ ] **Assert `app::Widget`'s union drop is never reached.**

Acceptance: every case destroys exactly once, on **both** backends, under the leak checker.
Commit: —

### Phase 3 — the second backend

- [ ] Re-run the entire Phase 2 matrix on GTK4.

Acceptance: identical results on GTK4. A model that holds on AppKit and not GTK is not a
model — and this is why this unit waits for both backends rather than shipping with one.
Commit: —

## Validation Plan

- Tests: the exactly-once matrix, per backend, under a leak checker.
- Coverage check: model-level cases run headless and are golden-backed; the native
  destroy-exactly-once cases are on-device and are stated as such.
- Runtime proof: macOS and the Debian aarch64 GTK4 box, both under the leak checker.
- Doc sync: none — the model is already specified in 13-C's surface docs.
- Acceptance: the project's full suite, no leaks.

## Open Decisions

1. **Where the offscreen holder lives.** Recommended one per process, created lazily on
   the first orphan — a per-window holder dies with the window it was meant to outlive.
2. **Whether re-attach is supported across windows or only within one.** Recommended
   across, as the draft specifies; it falls out of detach-not-destroy and forbidding it
   would need an extra check.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **Promoted from "plan-13-C Phase 6" to its own unit, and its dependency
  corrected.** It cannot land with one backend: "destroyed exactly once" must hold on
  each, so it depends on both 13-E and 13-F.
- 2026-07-20 — **The union-drop-unreachable property must be asserted, not assumed.** It
  holds by construction today because `app::Widget` is parameter-only, and the assertion is
  what keeps it true as 13-H and 13-I add variants.

## Summary

The engineering risk is that native and MFBASIC lifetimes disagree at exactly one moment —
a window closing with descendants still bound — and both toolkits' instinct is to tear the
subtree down. Retain-and-reparent is the answer and Phase 1 proves it on one backend before
the matrix is written against it.

The correctness risk is narrower and deliberate: `app::close` is both user-callable and the
window's registered close op, so it is the one reachable double-free path in the design.
The matrix tests it first.

This unit is late in the order for blast radius, not because it is polish — and it needs
both backends, because a lifetime model that holds on AppKit and not GTK is not a model.
