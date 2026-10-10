# plan-13-C: the `app::` package surface

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: plan-13-A (resource-union parameters). Feature-wide precondition:
plan-13 master §Prerequisites.
Produces: `src/builtins/app.rs`, 11 types, 32 functions as overload sets, `WIDGET_VARIANTS`,
the close-op registrations, and **the ability for an emitted helper to mint a `RES` record
outside a `LINK` block**. Consumed by every later unit.

Registers `app::` so programs typecheck against the full surface — with no shadow tree, no
solver, and no native window.

The single behavioral outcome: a program using the entire `app::` surface — including
`Widget`-union overload resolution, per-widget close ops and RES ownership rules — compiles
and is rejected exactly where it should be, while creating nothing.

References (read first):

- `src/builtins/net.rs` — `resolve_call`'s context-free `exact()` matching, the shape
  `app::` must generalize. Find with `rg -n 'fn resolve_call' src/builtins/net.rs`.
- `src/builtins/mod.rs` — `call_param_name_overloads` / `select_param_name_overload`.
  Find by symbol; the 2026-07-09 line numbers rotted (master §2.3).
- `src/binary_repr/builder.rs` — `FIRST_TABLE_TYPE_ID`. New builtin record types must use
  the high reserved range or collide (the `term::` `TermColor`/`TermSize` precedent).
- `src/ir/link.rs` — `IrNativeResource` / `close_function`, and plan-53's 80-byte resource
  record. §3.2.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| **plan-62 has landed** — the `app::` package is already registered (`getMode`/`setMode`/`Mode` exist, `--app` gating in place) | `rg -n '"app"' src/builtins/mod.rs` → `is_builtin_import` arm present | **NOT MET (plan-62 pending, added 2026-07-24)** |
| plan-13-A has landed (union params accepted on all three paths) | `rg -n 'resource-union-param-valid' tests/` | **NOT MET** |
| The high reserved type-ID range is free | `rg -n 'FIRST_TABLE_TYPE_ID' src/binary_repr/` | **MET** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every row before continuing and again before deciding to stop. If you stop, report every
> row. Locate every symbol below with `rg`, not by line number — master §2.3 records this
> family's citations rotting by up to 932 lines in eleven days.

## 1. Goal

- `app::` is registered as the 27th builtin package: 5 widget resources (`Window`,
  `Container`, `Button`, `Label`, `Input`), the `Widget` union, `Size`/`Rect`/`Spacing`
  records, and `Direction`/`Justification`/`Align`/`ClickMode` enums.
- Every function is an explicit **arity × type overload set** with all optional parameters
  **trailing**; a skipped middle argument is rejected.
- `app::close` is `Window`'s **exported** close op; the four `app::destroy` overloads are
  the child widgets' **internal** close ops — registry-only, never in the user-callable
  table, so `app::destroy(...)` in user code is an unknown function.
- An emitted runtime helper can **mint a `RES` record with a close op** without a `LINK`
  declaration (§3.2 — new capability).
- Every function stubs against the not-yet-built shadow tree.

### Non-goals (explicit constraints)

- **No shadow tree, no solver, no host seam, no native window.** Those are 13-D/13-E/13-F.
- **No `app::destroy` in the user-callable table.** It is a close op only.
- **Do not add `app::Widget` to `is_c_abi_type`** or any FFI surface — it is an MFBASIC
  type, not a C one.
- **No behavior.** Every acceptance here is accept/reject, not observable runtime output.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| **Files to change to register one builtin package** | **16** | `rg -l 'builtins::term\b\|builtins::net\b' -g '*.rs' src/ \| wc -l` |
| Builtin packages today | 26 | `ls src/builtins/*.rs \| wc -l` (minus `mod.rs`) |
| `app::` types to declare | 11 | master §2.1 |
| `app::` functions to declare | 32 (before overload expansion) | master §2.1 |
| Close ops to register | **5** (1 exported + 4 internal) | §1 |
| Existing package sizes, for scale | `term.rs` 331, `net.rs` 753, `audio.rs` 757 | `wc -l src/builtins/{term,net,audio}.rs` |

**The 16 files are the finding.** The 2026-07-09 draft budgeted a single checkbox —
"Register the `app::` builtin package" — for a change that touches the resolver,
three syntaxcheck files, three binary_repr files, `ir/lower.rs`, `ir/verify/mod.rs`, two
plan tables, and four `target/shared/` files. That is this sub-plan's real bulk.

The full list: `resolver/mod.rs`, `syntaxcheck/{inference,mod,helpers,builtins}.rs`,
`binary_repr/{builder,tests,sections}.rs`, `ir/lower.rs`, `ir/verify/mod.rs`,
`target/{linux_common,macos_aarch64}/plan.rs`, `target/shared/runtime/mod.rs`,
`target/shared/code/{validation,mod,module_analysis}.rs`.

### 2.2 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| Registering a package touches 16 files | **CONFIRMED** | the `rg -l` above, file list in §2.1 |
| `resolve_call` has no type-registry access | **CONFIRMED** | every package does context-free `exact()` string matching; it cannot see `app.Button` is an `app.Widget` variant |
| Builtin record types need the high reserved ID range | **CONFIRMED** | `FIRST_TABLE_TYPE_ID`; the `term::` `TermColor`/`TermSize` precedent |
| `TYPE_CALL_ARITY_MISMATCH` is emitted from `syntaxcheck/builtins.rs` | **CONFIRMED** | `:397` and `:446`, plus `syntaxcheck/inference.rs:1295` and `syntaxcheck/mod.rs:3113`/`:3131`. *(A review pass claimed it is "not raised from `syntaxcheck/builtins.rs` at all" and called that a design error. That claim is false — checked before acting.)* |
| A resource record with a close op can be minted outside `LINK` | **FALSE** | §3.2 — new capability |
| A resource union carries no `STATE` | **CONFIRMED** | `15_resource-management.md`; and `STATE` erases at parameter position, which makes `Widget`-as-parameter-only *more* sound (master §2.5) |

## 3. Design Overview

Three pieces: the registration sweep, the variant predicate, and the handle-minting
capability.

### 3.1 The variant predicate

`resolve_call` cannot reach the type registry, so `app.rs` owns a **static
`WIDGET_VARIANTS` table** and matches with a `widget_or(name)` predicate instead of
`exact`. This is a deliberate, package-local duplication of one fact — the union's variant
list — pinned by a `#[test]` against the registered union so it cannot rot.

**The overload-name `#[test]` is an A-owned contract with B and C as clients.** Two
`app::` overloads sharing an arity must share parameter names or differ in argument type,
or `select_param_name_overload` cannot separate them. 13-H adds `addTextArea` /
`addAttributedTextArea`; 13-I adds table `add*` forms. **Write the test to cover names
those units have not added yet**, and say so in its comment — plan-13-I hedged this as
"keeps *most of them* clear", and "most" is exactly the wrong word for a property the test
must enforce absolutely.

### 3.2 Minting a `RES` outside `LINK` — the new capability

plan-53's 80-byte resource record with a `STATE` payload and a registered close op is
exactly the right shape for a widget handle. But **every resource-with-close-op in the
tree today comes from a `LINK` declaration against a real C symbol** (`ir/link.rs`,
`IrNativeResource`), and `LINK` has no function-pointer ctype either. `app::`'s handles
come from an emitted runtime helper, not a `LINK` thunk.

So this sub-plan must add a path for a **codegen-emitted helper to produce a resource
record** with its close op registered. The 2026-07-09 design assumes widget handles are
resources — correctly — without noticing that the only existing route to one is closed.

**Where design uncertainty concentrates: §3.2, and only there.** Everything else is a
registration sweep with 26 precedents. Phase 1 is a spike on minting a single resource
from an emitted helper — with a close op that fires at scope drop — before the 32-function
surface is written against it.

**Where correctness risk concentrates:** the close-op registration. Registering
`app::destroy` in the call table instead of registry-only would make it user-callable, and
an explicit `destroy` plus a scope drop is a double free. The `-invalid` fixture asserting
`app::destroy(...)` is an **unknown function** is the guard.

**Rejected alternative:** *make `app::destroy` a normal exported function.* Rejected: the
close op fires at scope drop; a user-callable twin doubles it.

**Rejected alternative:** *derive `WIDGET_VARIANTS` from the registry at resolve time.*
Rejected: `resolve_call` deliberately has no registry access, and giving it some would
widen a seam every package shares for one package's benefit.

## Compatibility / Format Impact

- **New:** the `app::` package across 16 files; 11 types in the high reserved ID range;
  5 close-op registrations; the mint-a-`RES`-outside-`LINK` capability.
- **Unchanged:** every existing package; `is_c_abi_type`; the `LINK` surface.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — spike: mint a resource from an emitted helper (§3.2)

- [ ] Add the path for a codegen-emitted runtime helper to produce a resource record with
      a registered close op, no `LINK` declaration.
- [ ] Prove it with **one** throwaway resource type: bind it, let it drop, assert the
      close op fired exactly once.

Acceptance: a resource minted by an emitted helper is destroyed exactly once at scope
drop. If this cannot be done without a `LINK` declaration, stop — the whole handle model
rests on it and §3 needs redesigning.
Commit: —

### Phase 2 — the 16-file registration sweep

- [ ] Register `app::` across all 16 files (list in §2.1).
- [ ] Declare the 11 types; reserve their IDs in the high range.
- [ ] Declare the 32 functions as arity × type overload sets, optional params trailing.
- [ ] `WIDGET_VARIANTS` + the `widget_or` predicate, with the `#[test]` pinning it against
      the registered union.
- [ ] The overload-name `#[test]` per §3.1 — **written to cover names 13-H and 13-I will
      add**, with a comment saying so.

Acceptance: a program naming every `app::` function typechecks; `cargo check --all-targets`
is clean; the two `#[test]`s pass.
Commit: —

### Phase 3 — close ops and the rejection matrix

- [ ] `app::close` as `Window`'s exported close op; four `app::destroy` overloads as
      internal, registry-only close ops.
- [ ] Stub every function against the not-yet-built shadow tree.
- [ ] Tests under `tests/syntax/app/`: arity; overload resolution (`Window` vs `Widget`,
      `Window` vs `Container`); variant→union widening accepted; RES non-owner rejection;
      use-after-`close` rejected on a `Window`; **`app::destroy(...)` in user code rejected
      as an unknown function**; skipped-middle-argument rejected with
      `TYPE_CALL_ARITY_MISMATCH`.

Acceptance: every row of the rejection matrix produces its own diagnostic, and
`app::destroy` is unreachable from user code. The unknown-function test is the one that
prevents a double free.
Commit: —

## Validation Plan

- Tests: the rejection matrix above. Negative cases are the substance of this sub-plan —
  it creates nothing, so acceptance is entirely accept/reject.
- Coverage check: `tests/syntax/app/` is golden-backed and in the gate's denominator.
  `tests/acceptance/` has **no** `golden/` dir by design — do not put the proof there.
- Runtime proof: none applicable; nothing runs yet. Runtime proof begins at 13-E.
- Doc sync: a new `src/docs/spec/stdlib/` topic and `src/docs/man/builtins/app/` pages —
  **not `src/docs/spec/package/`**, which is the binary container format (master §2.5).
- Acceptance: the project's full suite.

## Open Decisions

1. **Whether `Size`/`Rect`/`Spacing` are records or opaque builtins.** Recommended
   records in the high reserved ID range, matching `term::`'s `TermSize`.
2. **Whether the mint-a-`RES` capability (§3.2) should be general or `app::`-specific.**
   Recommended general — it is a compiler capability, and a second consumer (a future
   package with emitted handles) would otherwise duplicate it. But keep its first use
   here, and do not generalize speculatively beyond what `app::` needs.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-24 — **The 16/17-file registration sweep is now plan-62's, not this plan's.** plan-62-A
  registers the `app::` package (name gate, resolver, syntaxcheck, IR, target arms), declares
  the `app::Mode` enum via a source companion, and adds `getMode`/`setMode` with `--app` gating.
  This sub-plan is now a **prerequisite-gated extension**: with the package already registered,
  13-C adds the widget *types* (`Window`/`Container`/`Button`/`Label`/`Input`/`Widget` union,
  `Size`/`Rect`/`Spacing`, the enums) and their functions to the existing package, plus the §3.2
  mint-a-`RES`-outside-`LINK` capability and the close-op registrations. The Phase-2 "16-file
  registration sweep" below collapses accordingly — re-scope it in place to "extend the
  plan-62 package" before executing (do not re-register from scratch). The §3.2 handle-minting
  spike and the close-op/`app::destroy` rejection matrix are **unchanged** and remain 13-C's
  core risk.
  2026-07-09 draft as one checkbox. Full list in §2.1; it is this sub-plan's real bulk.
- 2026-07-20 — **Resource records with close ops are `LINK`-only today** (§3.2). The draft
  assumes widget handles are resources — right — without noticing the only route to one is
  closed to emitted helpers. New capability, now Phase 1.
- 2026-07-20 — **The overload-name `#[test]` is a contract with 13-H and 13-I**, which the
  draft did not say. plan-13-I hedged it as keeping "*most* of them" clear of A's forms;
  the test must enforce all.
- 2026-07-20 — **A review pass claimed `TYPE_CALL_ARITY_MISMATCH` is not emitted from
  `syntaxcheck/builtins.rs`. False** — `:397`, `:446`. The draft was right; checked before
  acting on it.
- 2026-07-20 — Documentation destination corrected to `src/docs/spec/stdlib/` +
  `src/docs/man/builtins/`; the draft's `mfb spec package` is the container format.

## Summary

The engineering risk is §3.2: widget handles must be resources with close ops, and the
only existing way to make one is a `LINK` declaration this package does not have. Phase 1
spikes it before 32 functions are written against the assumption.

The bulk is not design at all — it is a 16-file registration sweep the original budgeted
as a single checkbox.

The correctness risk is one line of registration: `app::destroy` in the call table instead
of the close-op registry makes an explicit destroy possible, and an explicit destroy plus a
scope drop is a double free.

What is left untouched: every other builtin package, the `LINK` surface, and anything that
runs — this sub-plan creates no widget and opens no window.
