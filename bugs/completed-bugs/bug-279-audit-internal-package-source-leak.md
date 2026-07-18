# bug-279: `mfb audit` reports compiler-injected internal package source as user source flow

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed 2026-07-18
Regression Test: verified end-to-end — a project importing `collections` now
reports zero `builtins/` entries in `mfb audit` (was one per internal function)

## Resolution

The three *reporting* loops skip `file.internal`: `collect_source`
(`src/audit/collect/source.rs`), and `collect_native_resources` /
`collect_native_links` (`src/audit/collect/project.rs`).

The four *lookup* loops deliberately still scan every file —
`fallible_functions`, `link_aliases`, `link_fallible_calls`, and the
`close_may_fail` table in `collect_native_resources`. They build name tables, and
filtering them would strip the fallibility and alias labels off the *user's own*
calls into those packages, turning a reporting bug into a correctness bug. This
split is the whole subtlety of the fix.

Verified: a project with `IMPORT collections` previously emitted
`#collections_*` functions attributed to `builtins/collections.mfb`; it now emits
none, while permissions and control flow for the project's own source are
unchanged.

When a project imports a source-implemented package (e.g. `collections`),
`parse_project` appends the compiler-owned package `.mfb` file to `ast.files`,
parsed with `parse_source_internal` and tagged `internal: true`. The audit
collectors iterate `ast.files` without checking `file.internal`, so the injected
package's functions are reported as if they were the user's project source —
attributed to a sentinel path (`builtins/collections.mfb`) that does not exist in
the project. This pollutes the Control-flow section and the JSON `sourceFlow`,
misleading anyone auditing a dependency's actual attack surface.

The single correct behavior a fix produces: audit reports only the project's own
(and its declared dependencies') source; compiler-injected internal package files
and the prelude are excluded from all collectors.

References:

- Injection site `src/ast/manifest.rs:99` (`parse_project` tail).
- Found during goal-06 review of `src/audit/collect/source.rs`.

## Failing Reproduction

```
# Any project with: IMPORT collections
mfb audit .            # text
mfb audit --format json .
```

- Observed: text Control-flow prints e.g. `#collections_findIndex at
  builtins/collections.mfb:195 (fallible)`; JSON `sourceFlow` carries every
  `#collections_*` internal function attributed to `builtins/collections.mfb`.
- Expected: no internal package functions appear; only project source is reported.

## Root Cause

`src/audit/collect/source.rs:13` (`collect_source`, `for file in &ast.files`) and
the sibling collectors (`collect_resources`, `fallible_functions`,
`collect_native_links`, `collect_native_resources`) iterate `ast.files` without a
`file.internal` guard, so `parse_project`'s injected internal file
(`internal: true`) is treated as project source.

## Goal

- All audit collectors skip files with `internal == true` (and the prelude).

### Non-goals (must NOT change)

- The injection of internal package files into `ast.files` (needed for
  compilation).
- Reporting of the project's real dependencies where the audit is meant to surface
  them.

## Blast Radius

- `collect_source` + the four sibling collectors in `source.rs` / `project.rs`
  that loop `ast.files` — fix each loop.
- No other consumer of `ast.files` is in audit scope.

## Fix Design

Add `if file.internal { continue }` (and prelude check) at the top of each
collector's file loop. Rejected alternative: filtering after collection by path
prefix — fragile; the `internal` flag is the authoritative signal.

## Phases

### Phase 1 — failing fixture
- [ ] Audit fixture importing `collections`; assert no `#collections_*` /
      `builtins/*.mfb` rows. Confirm it fails today.
### Phase 2 — the fix
- [ ] Guard each collector loop on `!file.internal`.
### Phase 3 — validation
- [ ] Regenerate audit goldens; full suite green.

## Validation Plan

- Regression: the new fixture.
- Runtime proof: repro project's audit is clean of internal source.
- Doc sync: none.

## Summary

One guard per collector loop; the risk is only finding every loop that iterates
`ast.files` in the audit module.
