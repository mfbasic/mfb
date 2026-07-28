# bug-36: Imported-overload resolution binds an untyped empty collection (`Unknown`) to the FIRST matching package overload instead of reporting ambiguity

Last updated: 2026-07-08
Effort: small (<1h)

`src/monomorph/lower.rs::resolve_imported_overload` (`:116-133`) selects an imported
package overload with `.find()` — the **first** candidate whose arity and types
match — and `types_compatible` (`:139-150`) treats the `Unknown` token (produced by
an untyped empty `[]` literal → `List OF Unknown`) as a **bidirectional wildcard**
(`p == a || *p == "Unknown" || *a == "Unknown"`). So when a package exports
`f(List OF Integer)` and `f(List OF String)` and the importer calls `f([])`, **both**
candidates match, and `.find()` silently picks whichever the package stored first —
no `TYPE_OVERLOAD_AMBIGUOUS` — rewriting the call to an arbitrary specialization.

The local overload path (`resolve_overload` → `params_match`,
`helpers.rs:~374`) uses **exact** matching, so it leaves `f([])` unresolved rather
than silently choosing one — the two paths disagree on the same input.

The single correct behavior a fix produces: an ambiguous imported-overload match
(multiple candidates matching only via `Unknown` wildcards) is reported as
ambiguous, consistent with the local path, instead of resolving to export order.

Severity LOW (footgun): selection is deterministic (export order) but arbitrary and
silent; requires a package with element-type-differentiated overloads and a bare
`[]` argument.

References:

- `src/monomorph/lower.rs:116-133` (`resolve_imported_overload`, `.find()` first
  match), `:135-150` (`types_compatible`, `Unknown` bidirectional wildcard).
- Contrast: local overload resolution `params_match` (`helpers.rs:~374`) is exact.
- Found during goal-01 review of `src/monomorph/**`.

## Failing Reproduction

Import a package exporting `f(List OF Integer)` and `f(List OF String)`; call
`f([])`.

- Observed: the call resolves to the first-stored overload (export order), no
  ambiguity diagnostic.
- Expected: `TYPE_OVERLOAD_AMBIGUOUS` (or the same "unresolved" behavior the local
  path gives), since `[]` does not disambiguate the element type.

Contrast: `f([1])` (`List OF Integer`) resolves correctly; the equivalent local
overload set leaves `f([])` unresolved rather than silently choosing.

## Root Cause

`resolve_imported_overload` takes the first wildcard-compatible match instead of
requiring a unique one, and `types_compatible`'s `Unknown` wildcard makes multiple
element-typed overloads all match an untyped empty collection.

## Goal

- Imported-overload resolution reports ambiguity when more than one candidate
  matches only via `Unknown`, matching the local path's stricter behavior.

### Non-goals (must NOT change)

- Resolution of concretely-typed arguments (correct today).
- The `Unknown`-as-wildcard convenience when it uniquely selects one overload.

## Blast Radius

- `resolve_imported_overload` / `types_compatible`. Align with the local
  `params_match` and the encoding/return-type ambiguity handling.

## Fix Design

Collect **all** matching candidates; if exactly one, use it; if more than one and
the disambiguation depended on an `Unknown` wildcard, emit `TYPE_OVERLOAD_AMBIGUOUS`
instead of `.find()`'s first hit.

## Phases

### Phase 1 — failing test + audit

- [ ] Test: importing element-type-differentiated overloads + `f([])` reports
      ambiguity. Confirm it silently resolves today.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Unique-match / ambiguity check in `resolve_imported_overload`.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh`; concretely-typed imported-overload goldens
      unchanged.

## Validation Plan

- Regression test(s): the ambiguous-imported-overload test.
- Full suite: `scripts/test-accept.sh`.

## Summary

Imported-overload resolution silently picks export order for an untyped empty
collection; the fix requires a unique match (or reports ambiguity), aligning it with
the local overload path.
