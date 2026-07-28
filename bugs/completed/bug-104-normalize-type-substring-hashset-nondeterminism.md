# bug-104 — `normalize_type` qualifier stripping is substring-blind & HashSet-ordered → nondeterministic compilation; aliased imports of overloaded package fns unusable

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G2). **Reproduced: 8
identical builds flap between two distinct diagnostic sets.**
**Severity:** MED — nondeterministic diagnostics for the same source; a whole
class of valid programs (aliased import of an overloaded package function)
never resolves.
**Class:** correctness / nondeterminism.

## Finding

`src/monomorph/lower.rs:181-187` (`normalize_type` — `String::replace` of every
qualifier anywhere in the string), `src/monomorph/helpers.rs:294`
(`qualifiers.into_iter().collect()` from a `HashSet` — unordered), lower.rs:122-161
(`resolve_imported_overload` rewrites to `package.name$Types`).

Qualifier prefixes ("binding.", "package.") are stripped by an **unanchored
substring replace in nondeterministic HashSet order**. When one qualifier is a
substring of another (e.g. builtin `io.` inside alias `radio.`), the
normalization result — and therefore imported-overload resolution — depends on
hash order, so the same source produces different diagnostics run to run.

Separately, the rewrite target `package.base$Types` uses the *package* name,
which the post-monomorph resolver cannot resolve when the file imported the
package under an alias (imports map binding→package, not package→package). So
aliased imports of overloaded package functions can never resolve.

## Trigger (reproduced with the binary)

Project importing the `package-simple` fixture as `IMPORT package_simple AS
radio` and calling `radio::score(v)` flaps run-to-run between
`SYMBOL_UNKNOWN_IMPORT` (1 error) and `TYPE_UNKNOWN_VALUE` +
2×`TYPE_CALL_ARGUMENT_MISMATCH` (3 errors) — observed alternating across 8
consecutive identical builds.

## Fix sketch

Anchor qualifier stripping to a leading prefix match (strip only at position 0,
longest-match-first), and iterate qualifiers in a deterministic order (sort, or
use an ordered structure). Fix the overload rewrite to carry the binding/alias
so aliased imports resolve.

## Prior art

bug-87 is linker/exe byte nondeterminism — unrelated stage. None for this.
