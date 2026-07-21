# bug-375: rebinding a `RES` parameter closes the caller's resource

Last updated: 2026-07-21
Effort: medium (2h–4h)
Severity: HIGH
Class: Correctness (premature release / spec violation)

Status: Fixed (2026-07-21)
Regression Test: `tests/rt-behavior/resources/res-rebind-alias-runtime` (runtime
proof) and `tests/res_rebind_alias.rs` (emitted close-site counts)

## Correction (2026-07-21) — the Fix Design below was wrong as written

Three things this document got wrong, found by probing before implementing:

1. **The proposed classifier would have reintroduced bug-374's leak.** Fix
   Design says to treat a `Bind` as non-owning when "its initializer is a plain
   reference to an already-live resource (`NirValue::Local`)". But `TRAP`
   desugaring lowers a *producing* bind through a temp: `RES f AS File =
   fs::openFile(…) TRAP …` becomes `bind f = local $trap_val1`. At the NIR level
   a producer is therefore indistinguishable *by shape* from an alias, and the
   literal rule would have stopped closing every `TRAP`-bound resource. What
   saves it is that the temp is itself a resource bind in the *same* scope and
   carries the obligation — verified by counting emitted close sites, not
   assumed. `tests/res_rebind_alias.rs::trap_bound_producer_still_closes` pins
   this.

2. **The defect is not limited to `NirValue::Local`.** A resource element read
   out of a collection (`RES g AS File = collections::get(xs, 0)`) reproduces it
   identically when the alias is in an inner scope, and its initializer is a
   `Call`. So the boundary is *not* "local aliases / calls produce": §15.6 makes
   `get`/`getOr` yield "a pointer to the one resource", never a transfer, while
   every other call (`fs::openFile`, a user `FUNC … AS RES File`) does transfer.
   Both shapes had to be classified together.

3. **The record-field Open Decision is moot.** §15's opening states "Resources
   are atomic — records never hold them" (`TYPE_RESOURCE_FIELD_FORBIDDEN`), so a
   resource rebound from a record field is not expressible.

One more correction, to the Validation Plan: the fixture must NOT live under
`tests/rt-behavior/native/`. Those fixtures are executed (the `<pkg>.run` marker
is what triggers it) but the assertion lands in the `build.log` golden, and the
native ones additionally need `libsqlite3`; the built-in-`File` reproduction
belongs under `tests/rt-behavior/resources/` where it runs unconditionally.

Binding a `RES` parameter to a new `RES` name inside a callee —

```basic
FUNC passThrough(RES f AS File) AS Nothing
  RES g AS File = f
END FUNC
```

— registers `g` as an **owner** and closes the caller's resource when
`passThrough` returns. The caller's binding is still live and still believed
open; the next use of it fails hard:

```
Error: 7-703-0004
Resource handle is already closed.
[exit 255]
```

This contradicts §15.6 directly: "A `RES` binding, a `RES` parameter, and a
collection slot all hold **a copy of the one handle pointer**. … **None of these
close the resource**; the owning scope closes it exactly once on exit."
(`src/docs/spec/language/15_resource-management.md:143`.) §15's opening adds
that the owning scope is "the **outermost** scope that touches it" — here, the
caller's.

The shape is not exotic: it is the "opaque -> opaque" pass-through that
`tests/syntax/resources/state-opaque-narrow-valid/src/main.mfb:17-19` declares
**legal and expected**, verbatim. That fixture is syntax-tier, so it checks
diagnostics only and never runs the code — which is why this has gone unseen.

Found while fixing bug-373, as the probe for "is the `unused runtime helper`
error a false positive?". It is not: that internal error is a loud compile-time
failure standing in front of exactly this miscompile. See bug-373's Correction
(2026-07-21).

References:

- `src/target/shared/code/builder_control.rs:240-291` — the ownership branch
- `src/escape.rs:85-103` — `ResOwner`, which defaults a plain bind to `Local`
- `src/docs/spec/language/15_resource-management.md:143` — §15.6, the rule broken
- `bugs/completed-bugs/bug-373-user-resource-shadowing-builtin-name-internal-error.md`
  — where this was found (see its Correction, 2026-07-21)

## Failing Reproduction

Both a **built-in** and a **user-declared** resource reproduce it; the defect is
in the shared ownership branch, not in either resource family.

### Built-in resource (`File`)

```basic
IMPORT fs
IMPORT io

FUNC passThrough(RES f AS File) AS Nothing
  RES g AS File = f
END FUNC

FUNC main() AS Integer
  RES h AS File = fs::openFile("/tmp/b373probe.txt", "w")
  fs::writeAll(h, "before")
  passThrough(h)
  fs::writeAll(h, "after")     ' <- fails: handle already closed
  io::print("reached end")
  RETURN 0
END FUNC
```

- **Observed:** `Error: 7-703-0004  Resource handle is already closed.`, exit
  255. The file on disk contains `before` only — `after` is silently lost and
  `reached end` never prints.
- **Expected:** exit 0, file contains `beforeafter`, `reached end` printed. `g`
  is an alias; the owning scope is `main`.

### User-declared native resource (`RESOURCE Db CLOSE BY sql::close`)

Same shape, `passThrough(h)` then `sql::errcode(h)` → identical
`7-703-0004`, exit 255. Note this fails **only if the caller uses the resource
after the call**; with no subsequent use the program exits 0 and looks correct
while having closed the handle early. So the silent-corruption window is real:
the damage is a premature release, and the error is only raised if something
later happens to touch the handle.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS 24.6.0 | aarch64, debug | fails ✗ (both cases) |

**Not a regression from bug-374.** bug-374 touched `code/mod.rs`,
`builder_codegen_primitives.rs`, and `validation.rs`; it did not touch the
ownership branch in `builder_control.rs`, and the `File` case above runs entirely
through the built-in cleanup path bug-374 left alone. `git log -L 250,270` on
that branch shows its last change was `a6f4bf282`, a pure borrow→alias
vocabulary rename. The defect is long-standing and latent.

## Root Cause

`builder_control.rs:240-291` decides a `Bind`'s close obligation from the
**declared type**, consulting the initializer only for two special cases:

```rust
} else if aliases_union_variant || by_ref_capture_slot {
    // Non-owning — no cleanup (the parent binding frees it).
} else if let crate::escape::ResOwner::Float(collection) = &resource_owner {
    ...
} else if let Some(symbol) = self.resource_cleanup_symbol(type_) {
    self.active_cleanups.push(ActiveCleanup::Resource(...));   // <- g lands here
}
```

The three non-owning escape hatches are `UnionExtract`, `Capture { by_ref }`,
and a collection-floated owner. An initializer that is a **plain reference to an
existing resource** (`NirValue::Local("f")`) matches none of them, and
`src/escape.rs:91,103` defaults it to `ResOwner::Local` — so the bind is
classified as an owner and gets an `ActiveCleanup::Resource`.

`RES` **parameters** correctly register no cleanup
(`function_lowering.rs:651-690` pushes only `ActiveCleanup::Thread`). That is
the asymmetry: `f` is correctly an alias, but `RES g = f` manufactures an owner
out of an alias.

The runtime closed-flag is what keeps this from being a double-free: the
caller's own drop finds the flag set and no-ops. That makes the failure mode
*premature release*, not memory corruption — and it is why the bug survives as
an exit-0 program whenever nobody looks at the handle again.

## Goal

- A `RES` binding whose initializer merely names an already-owned resource is an
  **alias**: it registers no cleanup, and the resource is closed once by its
  owning scope, per §15.6.
- Both reproductions above exit 0 with the resource still usable after the call.

### Non-goals (must NOT change)

- **Do not reintroduce the bug-374 leak.** A bind that genuinely *produces* a
  resource (initializer is a call returning `AS RES T`) must keep its cleanup.
  The fix must distinguish producing from aliasing, not blanket-disable
  cleanups for resource-typed binds.
- **Do not weaken the `validate.rs:107` unused-helper check**, and do not
  "fix" the bug-373 route-2 symptom by teaching `used_helpers` to count plain
  resource binds. That error is currently the only thing stopping these programs
  from reaching codegen; silencing it converts a compile error into this runtime
  fault. It should stop firing as a *consequence* of this fix, not by being
  suppressed.
- **Do not change `RETURN`-of-a-resource ownership transfer** (the plan-59-D
  `escaping_value_slot` identity skip) — a different, working path.

## Blast Radius

- `src/target/shared/code/builder_control.rs:240-291` — the branch to change,
  plus its mirror predicate `owns_resource_slot` at `:127-132`, whose comment
  states it "mirror[s] the cleanup-registration branches below exactly". **Both
  must change together** or the prologue zero-init and the cleanup set disagree.
- `src/escape.rs` — if the alias classification is better expressed as a new
  `ResOwner` variant than as an initializer-shape test in the builder.
- `src/target/shared/runtime/usage.rs:142-147` — should stop declaring the
  helper for an aliasing bind once no close is emitted for it, which is what
  closes bug-373's route 2 without touching `validate.rs`.
- `tests/native_resource_scope_drop.rs` — bug-374's 5 exit-path assertions; they
  pin that *producing* binds still close, and must stay green. They are the
  guard against over-correcting into the bug-374 leak.
- `tests/syntax/resources/state-opaque-narrow-valid` — the fixture that already
  declares this shape legal; gains a runtime counterpart.

## Fix Design

Classify a `Bind` as non-owning when its initializer is a plain reference to an
already-live resource (`NirValue::Local` / parameter reference), joining the
existing `UnionExtract` and `Capture { by_ref }` hatches — and mirror it in
`owns_resource_slot`.

The correctness risk is the boundary between *aliasing* and *producing*: the
initializer kind is the signal, and a call returning `AS RES T` must stay
owning. A wrong boundary silently reintroduces bug-374's leak, which no test
catches by exit code — bug-374 measured it as 5.84 GB → 18.9 MB, so the runtime
fixture's memory ceiling is the real assertion.

**Rejected alternative — count plain resource binds as helper uses in
`validate.rs`.** This was the first thing tried while fixing bug-373. It makes
the program compile and is exactly wrong: it removes the compile-time barrier
and ships the premature close. Recorded here because it is the tempting fix and
it *looks* principled (it mirrors the existing resource-union block).

**Rejected alternative — reject `RES g AS T = f` as a diagnostic.** The spec
declares this shape legal (§15.5's opaque→opaque row) and there is an in-tree
fixture asserting it compiles. Rejecting it is a language change.

## Phases

### Phase 1 — failing test (no behavior change)

- [x] Add the runtime fixture driving every reproduction; confirmed it fails
      with `7-703-0004` / exit 255 against an unfixed compiler, and passes with
      the fix. (Landed as `tests/rt-behavior/resources/res-rebind-alias-runtime`
      — see Correction, item 4.)
- [x] Confirmed `tests/native_resource_scope_drop.rs` green before the change
      (6 passed), establishing the leak baseline.
- [x] Probed the Open Decisions: collection-element rebind reproduces the defect
      in an inner scope; record-field rebind is not expressible.

Acceptance: met — the fixture fails with the documented runtime error before the
change and bug-374's assertions pass.

### Phase 2 — the fix

- [x] Added `value_aliases_live_resource` (`builder_values.rs`) and wired it into
      both the cleanup branch and `owns_resource_slot` in `builder_control.rs`.
      Gated on the resource-typed cleanups alone, so a plain aliasing bind of a
      flat value still takes `owns_freeable_value` and is freed.
- [x] Stopped declaring the close helper for an aliasing bind in `usage.rs`.
      This was **required, not optional**: with no close emitted, the helper
      would be declared-but-unused and trip `validate.rs:107` — so leaving it
      would have turned working programs into compile errors.
- [x] Reused the existing `is_resource_element_pointer` (`ir/verify`) for the
      collection half rather than duplicating a name list; it was dead code, so
      this also removes a `dead_code` warning.

Acceptance: met — all reproductions exit 0 and use the resource after the call;
bug-374's assertions still pass.

### Phase 3 — leak proof + full validation

- [x] Leak proof by **emitted close-site count**, not RSS: the count is the
      direct observable, whereas an RSS/FD ceiling proved unreliable here (a
      hand-rolled FD-exhaustion harness reported success even for a deliberately
      leaking control, because the `ulimit` was not reaching the child). For the
      `TRAP` producer the count goes 4 → 3: exactly the one duplicate obligation
      removed, with one close per exit path intact.
- [x] Goldens seeded and synced; `cargo test` green (3251 passed); full
      `scripts/test-accept.sh` green (1068 tests).

## Validation Plan

- Regression test: `tests/rt-behavior/native/res-rebind-alias-rt`, failing with
  `7-703-0004` before and exiting 0 after.
- Runtime proof required — this is a codegen defect; a compile-time assertion
  cannot show the resource is still open. The post-call *use* is the assertion.
- Leak proof: peak-RSS ceiling over a hot loop, per Non-goals.
- Full suite: `cargo test`; `scripts/test-accept.sh`.

## Open Decisions

- **Is the initializer shape the right signal, or should `escape.rs` carry an
  explicit `ResOwner::Alias`?** Recommend the latter if the builder's
  initializer test cannot see through intermediate temporaries — escape analysis
  already computes ownership and is the honest home for the question. Decide in
  Phase 2 once the failing test pins the exact NIR shape.
- **Does the same defect apply to a resource rebound from a collection element
  or a record field?** Not yet probed. Phase 1 should extend the fixture to
  cover both before the fix is designed, so the classification is built against
  the full set.

## Summary

The engineering risk is the aliasing/producing boundary: too narrow and the
premature close survives, too wide and bug-374's leak returns. bug-374's
existing exit-path assertions and an RSS ceiling are the two guards, and they
pull in opposite directions by design.

Left untouched: `RETURN` ownership transfer, the `validate.rs:107` invariant,
and the legality of `RES g AS T = f`, which the spec already grants.
