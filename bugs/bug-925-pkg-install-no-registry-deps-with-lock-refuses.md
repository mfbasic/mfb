# bug-925: `mfb pkg install` with no registry dependencies but a leftover `mfb.lock` fails instead of "nothing to install"

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Footgun

Status: Open
Coordinator note: repro measured by the review agent only (HEAD 28a15a153 build); the coordinator did not re-run it.
Regression Test: unit test beside `install_without_declared_dependencies_is_a_no_op` in src/cli/resolve.rs (new)

The lockfile spec says a project that declares no registry dependencies "has
nothing to lock", and that the commands "skip resolution and installation
entirely". `mfb pkg install` only takes that path when `mfb.lock` is **absent**.
When a lock is present (committed earlier, or left by a teammate before the last
registry dependency was replaced with a local `file://` copy), install processes
it:

- With a stale `projectHash`, it fails with `mfb.lock does not match project.json, but the difference is not in a field the lock records (most likely `pin` or `source` changed); run `mfb pkg update``. That message is misleading, since no registry dependency exists.
- With a matching `projectHash`, it fails with `pinned registry key does not match the repoFingerprint in mfb.lock; refusing to install`, which is a trust-chain error for a project that uses no registry.

`mfb pkg update` on the same project prints "No registry dependencies to resolve;
mfb.lock is not needed." and deletes the lock. The two commands disagree.

**Correct behavior:** with zero registry dependencies, `pkg install` prints
`nothing to install: project.json declares no registry dependencies` and exits
0 whether or not a lock exists. Optionally it also notes (or removes, as
`pkg update` does) the unneeded lock.

References:

- `src/docs/spec/tooling/03_lockfile.md` *Projects with no registry dependencies* (table row `mfb pkg install` → prints `nothing to install`, exits `0`).

## Failing Reproduction

```
mfb init-pkg /tmp/mylib && (cd /tmp/mylib && mfb build)
mfb init /tmp/app && cd /tmp/app
mfb pkg add file:///tmp/mylib/mylib.mfp     # Added package mylib 0.1.0 … (pinned)
echo '{"lockfileVersion":1,"projectHash":"00","packages":[]}' > mfb.lock
mfb pkg install; echo "exit=$?"
mfb pkg update;  echo "exit=$?"; ls mfb.lock
```

- Observed: `error: mfb.lock does not match project.json, but the difference is not in a field the lock records (most likely `pin` or `source` changed); run `mfb pkg update`` and `exit=1`. Then `No registry dependencies to resolve; mfb.lock is not needed.` and `exit=0`, and the lock is gone.
- With the lock's `projectHash` set to the correct value (`9447f2ac…`, computed per the spec's algorithm and confirmed by `mfb audit` `projectHashMatches: true`): `error: pinned registry key does not match the repoFingerprint in mfb.lock; refusing to install`, `exit=1`.
- Expected: `nothing to install: project.json declares no registry dependencies`, `exit=0`.

Contrast: with no `mfb.lock` file, install prints the expected no-op message and exits 0.

## Root Cause

`src/cli/resolve.rs:install` checks `registry_dependency_count(&manifest) == 0`
only inside the `let Some(lock) = read_lock(project_dir)? else { … }` arm, i.e.
only when the lock is absent.

## Goal

- Zero registry dependencies always makes `pkg install` a successful no-op.

## Blast Radius

- `src/cli/resolve.rs:install`: fixed.
- `classify_drift`'s "unattributable" message: also fires when only a local `file://` package was added or removed (local packages are hashed but not diffed). See the B13 spec-drift report.
