# bug-398: compiler-side untrusted JSON decode via tinyjson has no recursion depth guard → stack-overflow abort on `project.json` / `mfb.lock` / package manifests

Last updated: 2026-07-28
Effort: small (<1h)
Severity: MEDIUM
Class: Security (DoS on untrusted input)

Status: Open
Regression Test: tests/ — a CLI fixture: `mfb pkg verify` (or `build`) in a project
whose `project.json` is `[`×N…`]`×N returns a clean error, not a stack-overflow
abort.

Every compiler-side decode of untrusted JSON (`project.json`, `mfb.lock`, a
dependency's package manifest) uses the vendored `tinyjson` 2.5 parser via
`str::parse::<JsonValue>()`. tinyjson is a recursive-descent parser with **no
nesting-depth limit**: `parse_array`/`parse_object` recurse once per nesting level,
so a JSON document nested a few hundred thousand levels deep overflows the native
thread stack and aborts the process (SIGABRT) *before* any schema/field validation
runs. Because `mfb` reads these files as the normal package-manager workflow on a
possibly-attacker-supplied project (clone + `mfb pkg install` / `build` / `verify`
/ `audit`), this is a reachable denial of service at the package trust boundary.

This is distinct from the already-fixed recursion bugs:
- bug-302 fixed the **MFBASIC-source** `json_package.mfb` runtime `json::parse`,
  not the Rust-side `tinyjson` parse of build files.
- bug-182/183/191/193/289 and goal-05 FE-02/FE-03 fixed **`.mfb` front-end**
  parse/resolve/monomorph recursion, not JSON decode.

None of the ~20 `parse::<JsonValue>()` sites in `src/` carries a depth guard.
(Note: `src/manifest/json_edit.rs` DOES implement an *iterative* brace-depth
counter for its structural edits — proof that the guarded pattern exists — but the
`parse::<JsonValue>()` calls it also makes, and every other site, do not.)

goal-05 rated the equivalent "SIGABRT on untrusted input" class HIGH for `.mfb`
source; the maintainer may wish to re-rate this MED accordingly. Kept MED here
because it is a build-time CLI abort (no memory corruption, no code execution).

References:

- Reproduced via `src/cli/resolve.rs` `read_lock` (`resolve.rs:908`) /
  `parse_project_json`, and `src/audit/collect/lockfile.rs:29`.
- tinyjson 2.5 (`Cargo.toml:21`), `parse_array`/`parse_object` unbounded recursion.
- Distinct from bug-302 (MFBASIC json builtin), bug-182/183/191/193/289 (front-end).
  Found during goal-07.

## Failing Reproduction

```
d=$(mktemp -d)
python3 -c "open('$d/project.json','w').write('['*120000+']'*120000)"   # 240 KB
cd "$d" && mfb pkg verify
```

- Observed: `thread 'main' has overflowed its stack` / `fatal runtime error: stack
  overflow, aborting` (SIGABRT). Verified 2026-07-28 against `target/debug/mfb`.
- Expected: a clean, bounded error (e.g. "project.json: JSON nested too deeply" or
  a normal parse error), non-crashing.

`project.json` is the vehicle because it is parsed first on nearly every
subcommand; `mfb.lock` (resolve.rs:908) is the same unguarded parse and crashes
identically once reached.

## Root Cause

`str::parse::<JsonValue>()` (tinyjson 2.5) recurses without a depth cap in
`parse_array`/`parse_object`; deep nesting exhausts the stack. Every compiler-side
untrusted-JSON entry point calls it directly.

## Goal

- Untrusted JSON of any nesting depth (`project.json`, `mfb.lock`, dependency
  manifests) yields a bounded parse error, never a stack-overflow abort.

### Non-goals (must NOT change)

- No change to the accepted JSON schema for well-formed inputs; a reasonable depth
  cap (e.g. 128–256, matching the front-end `MAX_EXPR_DEPTH`/`MAX_TYPE_DEPTH`) must
  not reject any legitimate manifest/lockfile.
- Not a fix for the `.mfb` front-end or `json::parse` (already handled).

## Blast Radius

Production untrusted-decode sites sharing this hazard (all fixed by one guarded
helper, e.g. a `parse_json_bounded(&str)` wrapper, or a bump-the-stack /
pre-scan-depth approach):

- `src/cli/resolve.rs:908` (`read_lock`) and `parse_project_json` — reproduced.
- `src/cli/pkg.rs` — `run_remove`, `add_*`, `verify_packages` (all via
  `parse_project_json`).
- `src/manifest/json_edit.rs:127,206,280` (`parse::<JsonValue>()` for edits).
- `src/audit/collect/lockfile.rs:29`, `src/audit/collect/dependencies.rs:152`,
  `src/audit/collect/findings.rs:476`.
- `src/resolver/packages.rs:208` (dependency `.mfp`/manifest read).
- `src/manifest/package.rs` — the many `parse::<JsonValue>()` at :1002/:1196/… are
  in `#[cfg(test)]` code (`.expect("json")`); not a production surface.

Recommended: route all production `parse::<JsonValue>()` through one bounded
wrapper so no future call site reintroduces the gap.
