# bug-396: file-PRIVATE rename pass skips `StateAssign.resource`, yielding a wrong/misleading diagnostic

Last updated: 2026-07-28
Effort: small (<1h)
Severity: LOW
Class: Correctness (latent) / Footgun (misleading diagnostic)

Status: FIXED
Regression Test: tests/ — a resolve/diagnostic fixture: a `FUNC` doing
`g.state = v` where `g` is a file-PRIVATE top-level binding should report the
"not a local binding" diagnostic, not "unknown identifier".

STATUS: FIXED (commit e48863ab5; regression fixture 49b30b30f)

The `StateAssign` arm in `scope_privates.rs` now sweeps `resource` through the
`rename` map exactly like the sibling `Assign` arm (an in-scope local target is
still correctly left bare, so the local-only rule is unchanged). The PRIVATE
repro now reports `2-203-0043 TYPE_UNKNOWN_VALUE: State assignment target `g`
is not a local binding.` — matching the PUBLIC contrast.

Deviation from the doc's plan (a one-line fix): sweeping the reference exposed a
second, undocumented half of the same footgun — the checker's diagnostic
(`syntaxcheck/checking.rs:345-359`) interpolated the now-mangled `resource` raw,
so it would have leaked the untypeable internal spelling `#<hash>$g` into the
user-facing message. The message is now routed through
`internal_name::display_name`, which demangles `#<hash>$g` back to the plain
source `g`, so the accurate diagnostic reads exactly as the "Expected" line
below. Regression coverage:
`src/ast/scope_privates.rs` — two unit tests (`private_binding_used_as_state_assign_target_is_rewritten`,
`a_local_shadowing_a_private_keeps_its_state_assign_target_bare`) and a
full-pipeline diagnostic fixture
`tests/syntax/resources/resource-state-assign-private-invalid/`.

The file-PRIVATE name-mangling pass in `src/ast/scope_privates.rs` rewrites every
reference position of a PRIVATE top-level binding to its mangled `#<hash>$name`
spelling — except one. The `StateAssign` arm (`scope_privates.rs:340`) is:

```rust
Statement::StateAssign { value, .. } => rewrite_expr(value, rename, types, scope),
```

Only `value` is swept; the `resource` field (the assignment target's name) is
never run through the `rename` map. Its sibling `Statement::Assign` (lines
332-338) *does* rewrite its target `name`. So for a `foo.state = v` statement whose
`foo` names a file-PRIVATE top-level binding, the declaration is mangled to
`#<hash>$foo` while the `StateAssign.resource` reference stays bare `foo`. The
resolver then fails with `SYMBOL_UNKNOWN_IDENTIFIER` ("Identifier `foo` is not
declared in this scope") instead of the accurate `TYPE_UNKNOWN_VALUE` ("State
assignment target `foo` is not a local binding").

Impact is bounded to a **misleading error message on already-invalid code**:
`StateAssign` is only valid for a *local* binding — `syntaxcheck/checking.rs:345-359`
has no `lookup_visible_binding` global fallback (unlike `Assign` at 322-338) — and
a local is always in `scope`, so it is correctly never rewritten. Thus no *valid*
program is miscompiled today. But the un-swept reference field is a genuine latent
asymmetry: it becomes a real correctness bug if module-level resources ever become
assignable. bug-288 ("private-resource-half-mangled") fixed the sibling gaps
(close_fn, param/return STATE types, LINK signatures) but never touched
`StateAssign.resource`.

References:

- `src/ast/scope_privates.rs:340` (the arm that skips `resource`), vs the `Assign`
  arm at `:332-338` that rewrites its target.
- `src/syntaxcheck/checking.rs:345-359` (`StateAssign` local-only, no global
  fallback) vs `:322-338` (`Assign` allows a global target).
- Prior sibling fix: bug-288 (`bugs/completed/`). Found during goal-07.

## Failing Reproduction

Project `/tmp/scpriv`, `src/main.mfb`:

```
PRIVATE MUT g AS Integer = 0
FUNC f() AS Integer
  g.state = 5
  RETURN 0
END FUNC
```

- Observed (PRIVATE g): `2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER: Identifier g is not
  declared in this scope`.
- Expected: `2-203-0043 TYPE_UNKNOWN_VALUE: State assignment target g is not a
  local binding` (the accurate diagnostic).

Contrast (proves the cause): change `PRIVATE`→`PUBLIC` (no mangling) and the same
program reports `TYPE_UNKNOWN_VALUE: State assignment target g is not a local
binding` — so `g` resolves fine when not mangled, and the divergence is caused
solely by the un-rewritten `StateAssign.resource` reference.

## Root Cause

`scope_privates.rs:340` sweeps only `StateAssign.value`, not `StateAssign.resource`,
leaving the target-name reference at its bare (unmangled) spelling while its
declaration is mangled.

## Goal

- The PRIVATE rename pass rewrites `StateAssign.resource` through the same `rename`
  map as `Assign.name`, so the resolver sees the mangled spelling and emits the
  accurate `TYPE_UNKNOWN_VALUE` diagnostic (or resolves it, if a future language
  change makes module-level resources assignable).

### Non-goals (must NOT change)

- Do NOT make `StateAssign` accept a global/module-level target — the local-only
  rule (`checking.rs:345-359`) is correct and out of scope.
- No change to mangling scheme or to the `Assign` arm.

## Blast Radius

- `src/ast/scope_privates.rs:340` — fixed by this bug.
- Other `StateAssign`-touching passes: none share this rename map; the resolver
  (`resolution.rs:892`) is the consumer that surfaces the wrong diagnostic.
