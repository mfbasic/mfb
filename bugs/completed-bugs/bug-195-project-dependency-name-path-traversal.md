# bug-195: project.json dependency name not path-validated → path traversal at packages/<name>.mfp

Last updated: 2026-07-14
Effort: small (<1h)
Severity: MEDIUM
Class: security (path traversal)

Status: Fixed (2026-07-15) — `project_package_dependency` now calls
`validate_package_name(&name)` and returns `None` (rejecting the dependency, like
the blank-name case) when the name is not a single path component, so no
`packages/<name>.mfp` path is ever built from a `../…`/absolute/`sub/dep`/leading-`.`
name. Only `name` is path-joined (not `ident`), so this closes the vector.
Regression Test: unit test `project_package_dependency_rejects_path_traversal_name`
(rejects `../../../../etc/passwd`, `/etc/passwd`, `sub/dep`, `.hidden`; still
accepts `legit_pkg`); verified `mfb audit` no longer probes the traversal path.

A package dependency `name` read from `project.json` is never validated as a
single path component, yet it is interpolated into `packages/<name>.mfp` and
read/merged by multiple consumers. `project_package_dependency`
(`src/manifest/package.rs:494-536`) only rejects a blank name; `validate_package_name`
(which guards the *header-stored* name against `..`, leading `.`, and separators
— bug-58) is never applied to this path. `PathBuf::join` with an absolute string
also replaces the base, enabling absolute targets.

Consumers that build and read the path: `src/audit/collect/dependencies.rs:18-20`
and `findings.rs:127-130` (`mfb audit` — existence oracle: `missing` vs `invalid`,
plus disclosure of another project's `.mfp` header/version/content-hash into the
report), `src/cli/pkg.rs:807-809`, and the resolver/build merge path.

## Failing Reproduction

```
project.json: { "packages": [ { "name": "../../../../etc/passwd" } ] }   (declared, never IMPORTed)
mfb audit    # / mfb build / mfb pkg verify
```
Observed: the collector `Path::join`s the name, escaping `packages/`, and
`is_file()`/`read_mfp_header`/`package_content_hash_file` the resulting path —
leaking existence/metadata of arbitrary host `*.mfp` paths (and merging exports
from any structurally-valid `.mfp` found). Expected: the dependency is rejected
as an invalid name.

## Root Cause

`src/manifest/package.rs:494-536` `project_package_dependency` accepts any
non-blank `name` and does not call `validate_package_name`, while every
`packages/<name>.mfp` join site trusts it. Asymmetric with the header-name path,
which validates precisely to stop this.

## Non-goals

- Do not change validation of the header-stored name (already correct).
- Do not alter resolution of legitimately-named dependencies.

## Blast Radius

- `project_package_dependency` (root fix). Consumers listed above inherit the
  fix once the name is validated at construction; alternatively validate at each
  join site.

## Fix Design

Call `crate::manifest::package::validate_package_name(&dependency.name)` inside
`project_package_dependency` (reject / emit an invalid-dependency finding on
failure) so no `packages/<name>.mfp` path is ever built from an unvalidated name.
