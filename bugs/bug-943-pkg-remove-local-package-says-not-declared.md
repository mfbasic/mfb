# bug-943: `mfb pkg remove <name>` on a declared local (`file://`) package says it "is not declared in project.json"

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Footgun

Status: Open
Regression Test: `src/cli/pkg.rs` tests, beside the existing `is not declared in project.json` assertions (`run_remove`)

**Owner decision (2026-10-05):** `mfb pkg remove <name>` removes a declared local (`file://` / unsigned) package by name,
with the same closure pass and file cleanup as a registry package. The cli-reference synopsis
lists both `<owner>#<pkg>` and a local package name.

`mfb pkg add file:///…/k1.mfp` declares `{"name":"k1","ident":"k1",…,"source":"file:///…"}`
in `project.json`. The CLI then has no way to undo it. `mfb pkg remove k1` fails
with `` error: `k1` is not declared in project.json `` (exit 1), and the entry it
names is plainly in the file. `remove` only considers entries whose `ident`
contains `#` (registry idents). Any other declared package is invisible to it, and
the message misstates the reason.

The lockfile topic counts `mfb pkg remove` among the ways a project ends up "one
whose only packages are local `file://` copies". The cli-reference synopsis is
`mfb pkg remove <owner>#<pkg>`. So either removing a local package is meant to be
unsupported, in which case the message must say so, or it is meant to work.
**Correct behavior (UNCLEAR, owner decision), either:**
(a) `pkg remove <name>` removes a declared local/unsigned package by name, with the
same closure pass and file cleanup; or (b) the refusal names the real reason:
`` `k1` is a local package (source file:///…); pkg remove takes a registry
<owner>#<package> — edit project.json to drop it ``, and exits 2 (usage) instead of
reporting it undeclared.

References:

- `src/docs/spec/tooling/07_cli-reference.md` *`pkg remove` and the Reverse-Dependency
  Cascade* (synopsis `mfb pkg remove <owner>#<pkg> [--yes]`).
- `src/docs/spec/tooling/03_lockfile.md` *Projects with no registry dependencies*
  (row "`mfb pkg add` / `mfb pkg remove` | writes `project.json` …").
- Found during hardening batch B14.

## Failing Reproduction

```
mfb init-pkg /tmp/k1 && (cd /tmp/k1 && mfb build -q)
mfb init /tmp/app1 && cd /tmp/app1
mfb pkg add file:///tmp/k1/k1.mfp      # Added package k1 0.1.0 to ./project.json (pinned)
grep -A3 '"name": "k1"' project.json
mfb pkg remove k1; echo "exit=$?"
```

- Observed (binary /tmp/hardening/mfb-head, HEAD 28a15a153):
  ```
  "name": "k1",
  "ident": "k1",
  "version": "0.1.0",
  "pin": true,
  error: `k1` is not declared in project.json
  exit=1
  ```
- Expected: (a) the package is removed, or (b) a message saying why a local package
  cannot be removed this way.

Contrast: in the same project `mfb pkg update k1` answers `error: registry ident must
use <owner>#<package>`, which is accurate. `pkg remove` should give at least that.
(In a project that does *not* declare `nothash`, `mfb pkg update nothash` answers
`` `nothash` is not declared in project.json; use `mfb pkg add nothash` ``, and
following that advice fails, because `pkg add nothash` is rejected as not an ident.
That is a smaller wording issue.)

## Root Cause

`src/cli/pkg.rs:run_remove` builds `declared` with `.filter(|dep|
dep.ident.contains('#'))` and then errors `` `{target}` is not declared in
project.json `` when the target is not among them. Unlike the `pkg update`
targeted form, it never checks whether the target is a non-ident name.

## Goal

- No message calls a package "not declared" when `project.json` declares it.

### Non-goals (must NOT change)

- The registry cascade, the not-installed gate, and `--yes` semantics for
  `<owner>#<pkg>` targets.

## Blast Radius

- `run_remove`: fixed by this bug.
- `pkg update <non-ident>`: correct for a declared local package. The "use `mfb pkg
  add <non-ident>`" advice for an undeclared non-ident is unhelpful but out of scope.
