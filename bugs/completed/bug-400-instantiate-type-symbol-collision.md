# bug-400: `instantiate_type` inserts into `concrete_types` without the `unique_concrete_symbol` collision guard that bug-226 added to `instantiate_function`

Last updated: 2026-07-28
Effort: small (<1h)
Severity: LOW
Class: Correctness (latent — cache/symbol-key collision → wrong monomorphized type)

Status: Fixed (2026-08-01) — `instantiate_type` now claims `mangle_name(name, args)`
against the unambiguous `name<args>` key via `unique_concrete_symbol`, exactly as
`instantiate_function` has since bug-226. Byte-identical for every reachable
single-instantiation case (`unique_concrete_symbol` returns the same symbol when no
other key claims it): full artifact-gate = 1511 golden(s) checked, 0 diff(s) across
all four targets.
Regression Test: `src/monomorph/lower.rs` unit test
`instantiate_type_disambiguates_mangle_colliding_arguments` — drives `instantiate_type`
directly with two punctuation-only-different colliding arguments and asserts each keeps
a distinct concrete symbol + concrete-type declaration. RED before the fix (both
collapsed to `Box$FUNC$Integer$$AS$Nothing`), GREEN after. **Deviation from the
originally-proposed source fixture:** no *valid* source spelling reaches the collision —
the grammar fixes each punctuation slot relative to the alnum tokens and every mangled
generic fragment carries a leading identifier, so the mangled symbol is the only lossy
layer. This is precisely why bug-226/bug-400 are latent; the mechanism is therefore
exercised at the `instantiate_type` boundary (the same rationale as
`total_instantiation_budget_halts_wide_fanout`, which drives its counter directly).

`instantiate_type` (`src/monomorph/lower.rs:765`) builds `concrete_name =
mangle_name(name, args)` and inserts the lowered type directly into
`concrete_types[concrete_name]` with **no collision guard**. `mangle_name` /
`sanitize_type_name` are lossy (every non-alphanumeric → `$`), so two distinct
same-arity type-argument tuples that differ only in punctuation (e.g.
function-typed arguments) can collapse to one symbol; the second instantiation then
overwrites the first, and both use-sites rewrite to one shared — possibly wrong —
concrete type.

This is the exact hazard bug-226 fixed for functions via `unique_concrete_symbol` /
`concrete_symbol_keys` (applied at `instantiate_function`, lower.rs:646). bug-226's
own root cause explicitly names BOTH maps — "the `concrete_functions`/
`concrete_types` maps are keyed by that symbol, so the second instantiation
overwrites the first" — but the landed fix applied the disambiguator only to
`instantiate_function`; the `concrete_types` half was documented as affected yet
left unguarded.

References:

- `src/monomorph/lower.rs:765` (`instantiate_type` unguarded insert) vs `:646`
  (`instantiate_function` uses `unique_concrete_symbol`).
- bug-226 (`bugs/completed/`) — root cause names `concrete_types`; fix covered only
  `concrete_functions`. Found during goal-07.

## Failing Reproduction

Latent — bug-226 itself labeled the collision "Latent", and a valid colliding pair
of concrete *type* strings was not constructed within the review budget (the
lossy-mangle collision requires two distinct type-argument tuples whose
`sanitize_type_name` outputs coincide). The mechanism is established by code
reading: `instantiate_type` has no analogue of the `unique_concrete_symbol`
disambiguation that guards `instantiate_function`.

- Observed (by inspection): a second `instantiate_type` with a mangle-colliding
  arg tuple overwrites `concrete_types[concrete_name]`; the first use-site silently
  binds the wrong concrete type.
- Expected: colliding symbols are disambiguated (as for functions), so each
  instantiation keeps a distinct entry.

## Root Cause

`instantiate_type` reuses the lossy mangled name as the `concrete_types` map key
without the `unique_concrete_symbol` / `concrete_symbol_keys` disambiguation that
`instantiate_function` uses.

## Goal

- `instantiate_type` disambiguates colliding concrete symbols the same way
  `instantiate_function` does, so two distinct type-argument tuples never share one
  `concrete_types` entry.

### Non-goals (must NOT change)

- The mangling scheme itself; only the collision-disambiguation at the insert.

## Blast Radius

- `src/monomorph/lower.rs:765` (`instantiate_type`) — fixed by this bug.
- `instantiate_function` (`:646`) — already guarded (bug-226); the model to mirror.
