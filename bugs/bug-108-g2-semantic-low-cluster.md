# bug-108 — G2 semantic LOW cluster: dead relocated syntaxcheck rules; non-nesting-aware `" TO "` Map splitters

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G2). Two independent
LOW/latent findings, batched per goal-02.

## 1. Relocation leftovers in syntaxcheck compute full diagnostics and discard them (dead-code)

- `src/syntaxcheck/inference.rs:434-494` (`report_match_not_exhaustive` — builds
  the complete missing-cases message into `_detail`, never reports).
- `src/syntaxcheck/mod.rs:1153-1170` (`report_expanded_union_member_conflicts` —
  expands every included union into maps, all consuming code is empty `if … {}`).
- `src/syntaxcheck/types.rs:303-313` (`require_comparable_type` — empty shell
  called from 5+ sites).

plan-20-Z relocated these rules to `ir::verify` (confirmed
`TYPE_MATCH_NOT_EXHAUSTIVE`, `TYPE_UNION_MEMBER_REQUIRES_TYPE` exist there), but
these helpers still run their full computation (string formatting, sorting,
recursive union expansion) and throw the result away on every MATCH / UNION /
Map-key check. No user impact; wasted work and names that claim to
"report"/"require" but don't. Same pattern as fixed bug-43 (resources.rs
stubs), different functions. Fix: delete the dead computation, keep only the
downstream-enforced shells or remove entirely.

## 2. Leftmost `" TO "` splitters mis-parse Map types whose key carries a top-level `TO` (latent)

- `src/monomorph/helpers.rs:219-223` (`split_top_level_to` — plain
  `split_once(" TO ")` despite the name; used by `unify_type`,
  `substitute_type_params`, `concrete_type_name`, `template_view_type`).
- `src/resolver/resolution.rs:1240,1248` (Map/MapEntry arms).
- `src/syntaxcheck/inference.rs:791-798` (MapEntry member access).

All three split `K TO V` at the first `" TO "`, so a key that itself contains a
top-level `TO` (nested Map/Thread/`FUNC() AS` key) shreds the type — the
depth/ownership-aware splitter from bug-41 (`split_map_body`, types.rs:436)
exists only in syntaxcheck's `parse_type`. Because Map keys must be comparable,
such keys are only expressible in already-invalid programs, so today this
degrades diagnostics rather than accepting/rejecting wrongly.

Trigger: `Map OF Map OF String TO Integer TO Boolean` in any type position →
resolver reports `SYMBOL_UNKNOWN_TYPE: Type 'Map OF String'…` instead of the
comparable-key rule (misleading message on an invalid program). No valid
program reaches the wrong split. Fix: use the depth-aware `split_map_body` at
these sites. bug-41 fixed the syntaxcheck instance only.
