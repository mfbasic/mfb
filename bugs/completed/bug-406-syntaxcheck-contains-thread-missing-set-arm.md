# bug-406: `contains_thread` / `contains_resource_or_thread` miss a resource/thread nested in a `Set` (no `Type::Set` arm)

Last updated: 2026-07-28
Effort: small (<1h)
Severity: LOW
Class: Correctness (latent soundness) — missing match arm

Status: FIXED (c835a90dd)
Regression Test: `src/syntaxcheck/resources.rs::resources_tests::contains_thread_walks_set_element`
— a direct-checker unit test: `contains_thread(Set OF Thread)` and
`contains_resource_or_thread(Set OF Thread)` must return `true`. RED before the
fix (fell through to `_ => false`), GREEN after.

## STATUS: FIXED (c835a90dd)

Added a `Type::Set(element)` recursion arm to both `contains_thread_with_seen`
(`src/syntaxcheck/resources.rs:50`) and `contains_resource_or_thread_with_seen`
(`resources.rs:93`), immediately after each `Type::List` arm — matching the
sibling walks `is_copyable_type_with_seen` / `is_thread_sendable_type_with_seen`.
A `Set` element is now walked exactly like a `List` element.

Blast-radius audit (per §Blast Radius): every other `Type::List` walk in
`src/syntaxcheck/` already carries a matching `Set` arm
(`checking.rs:560`, `mod.rs:574`/`:1439`/`:1571`, and the two other predicates in
`resources.rs`). The two `builtins.rs` `Type::List` matches (`:847`, `:861`) are
`strings::contains`/`replace`/`find` argument extractions, not recursive
type-walks, so no `Set` arm applies. No deviation from the plan.

Verified: RED→GREEN on the new unit test; full `cargo test` green
(3725 bin + all integration targets, 0 failed). No golden shift (the predicate
is latent — no reachable source path changes codegen output).

`contains_thread_with_seen` (`src/syntaxcheck/resources.rs:42-81`) and
`contains_resource_or_thread_with_seen` (`resources.rs:83-127`) recurse through
`List`, `Map`, and `User` record fields but have **no `Type::Set(_)` arm** — a
`Set` falls through to `_ => false`. So both predicates answer `false` for
`Set OF Thread` / `Set OF <resource>`, and (via the `Type::User` field recursion)
report a record with a `Set OF Thread` field as thread-free.

Every sibling type-walk over the same `Type` has a `Set` arm —
`is_copyable_type_with_seen` (`resources.rs:209`),
`is_thread_sendable_type_with_seen` (`resources.rs:269`),
`is_comparable_with_seen` (`types.rs:335`) — so these two (which predate
`Set`/plan-63) were simply not updated when `Set` was added.

**Latent today.** In-tree, every path that could feed a `Set`-of-thread/resource
to these predicates is independently rejected first: `check_type_reference`'s `Set`
arm (`mod.rs:1439-1447`) and `infer_set_literal` (`inference.rs:579-581`) reject a
resource/thread `Set` element at the `Set` itself, and a record with such a field
is rejected at its own `check_type_decl`. The only smuggling route is a
corrupt/hand-crafted `.mfp` installed via `install_package_type_info` (which
bypasses `check_type_decl`, and `validate_package_metadata_type` at
`mod.rs:573-585` recurses through `Set` without rejecting a thread element) — a
package-trust-boundary case. So there is no reachable unsound path from normal
source today; it is a real hole waiting on any future caller that invokes these
predicates on a `Set`-bearing type not otherwise gated. Ranked LOW.

References:

- `src/syntaxcheck/resources.rs:42`, `:83` (the two walks lacking a `Set` arm) vs
  `:209`, `:269` and `src/syntaxcheck/types.rs:335` (siblings that have it). Found
  during goal-07.

## Failing Reproduction

No reachable source-level trigger (the `Set` element is rejected upstream). The gap
is demonstrable at the unit level: call `contains_thread(&Type::Set(Box::new(
Type::Thread…)))` → returns `false`.

- Observed: `contains_thread(Set OF Thread) == false`.
- Expected: `true` (a `Set` element is walked like `List`/`Map`).

## Root Cause

The two predicates were written before `Set` existed and never got a `Type::Set`
recursion arm, unlike their siblings.

## Goal

- `contains_thread_with_seen` and `contains_resource_or_thread_with_seen` recurse
  into `Type::Set(element)` exactly as they do for `List`/`Map`.

### Non-goals (must NOT change)

- The upstream `Set`-element rejection rules (which currently make this latent).

## Blast Radius

- `src/syntaxcheck/resources.rs:42`, `:83` — fixed by this bug (add the `Set` arm).
- Sibling walks already handle `Set`; grep for any other `Type::` match in
  syntaxcheck missing a `Set` arm.
