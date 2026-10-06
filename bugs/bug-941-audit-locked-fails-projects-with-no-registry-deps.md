# bug-941: `mfb audit --locked` always fails a project with no registry dependencies, the state where `pkg update` deletes `mfb.lock`

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Footgun

Status: Open
Regression Test: `src/audit/collect/findings.rs` tests (`lockfile_findings`), plus an acceptance golden for `mfb audit --locked` on a fresh `mfb init` project

**Owner decision (2026-10-05):** with zero registry dependencies, a missing `mfb.lock` raises no finding under
`--locked`, and audit exits 0. A present lock is checked as today. A project that does declare
registry dependencies keeps `AUDIT-LOCK-MISSING` under `--locked`. The lockfile and audit spec
topics agree on this.

Two spec topics disagree, and the code follows the one that makes `--locked`
impossible to satisfy:

- The lockfile topic says a project with no registry dependencies "has nothing to
  lock". It says "An absent lock is a valid, correctly configured state, not a
  missing file", and that `mfb pkg update` "removes a stale `mfb.lock`". Measured:
  `pkg update` deletes the lock and prints `No registry dependencies to resolve;
  mfb.lock is not needed.`
- The `--locked` policy table (same topic) and the audit-format catalogue raise
  `AUDIT-LOCK-MISSING` (**error**) whenever `--locked` is set and `mfb.lock` is
  absent, with no exception for a project that needs no lock.

So `mfb audit --locked`, the documented CI gate, fails (exit 1) on every freshly
`mfb init`-ed project, on every project whose packages are all local `file://`
copies, and after `mfb pkg remove` takes the last registry dependency. The user
cannot fix it: the only command that would write the lock deletes it again. A CI
pipeline has to drop `--locked`, which also drops the staleness gate for projects
that do have registry dependencies.

**Proposed behavior (UNCLEAR, owner decision):** with zero registry dependencies,
an absent `mfb.lock` raises no finding under `--locked`. A present one is checked
as today, or reported as unneeded. The alternative is to keep the error and make
`pkg update` write an empty lock. That contradicts the lockfile topic's "absent lock
is a valid state" and bug-925's direction.

References:

- `src/docs/spec/tooling/03_lockfile.md` *Projects with no registry dependencies*
  and *`--locked` policy* (row "`mfb.lock` absent | no finding |
  `AUDIT-LOCK-MISSING`, severity **error**").
- `src/docs/spec/tooling/04_audit-format.md` finding table row `AUDIT-LOCK-MISSING`.
- `src/docs/spec/tooling/07_cli-reference.md` command table: `audit` exits `1` on
  error findings.
- Related, not a duplicate: bug-925 (`pkg install` with no registry deps but a
  leftover lock).

## Failing Reproduction

```
mfb init /tmp/nolock && cd /tmp/nolock
mfb pkg update;            echo "update=$?"; ls
mfb audit --locked >/dev/null; echo "audit=$?"
mfb audit --locked | grep AUDIT-LOCK
```

- Observed (binary /tmp/hardening/mfb-head, HEAD 28a15a153):
  ```
  No registry dependencies to resolve; mfb.lock is not needed.
  update=0
  project.json  src
  audit=1
    error AUDIT-LOCK-MISSING mfb.lock is required by --locked but was not found (mfb.lock)
  ```
  The JSON report has `"lockfile": {"present": false, "locked": true, …}`.
- Expected: `audit=0` with no `AUDIT-LOCK-*` finding, because the project declares
  no registry dependency and the toolchain itself says no lock is needed.

Contrast: a project with one `file://` package (`mfb pkg add
file:///…/k1.mfp`) behaves the same (`AUDIT-LOCK-MISSING`, exit 1). Without
`--locked`, both report `Lockfile: absent` and no finding.

## Root Cause

`src/audit/collect/findings.rs:lockfile_findings` emits `AUDIT-LOCK-MISSING`
whenever `lockfile.locked && !lockfile.present`. It does not consult the registry-dependency
count, which `src/cli/resolve.rs` uses (`registry_dependency_count`) to decide that
no lock is needed.

## Goal

- The audit and the lock-writing commands agree on when a lock is required.
- A fresh `mfb init` project passes `mfb audit --locked`.

### Non-goals (must NOT change)

- `AUDIT-LOCK-MISSING`, `-STALE` and `-MALFORMED` for projects that do declare
  registry dependencies.

## Blast Radius

- `lockfile_findings` is fixed by this bug.
- `03_lockfile.md` *`--locked` policy* table and `04_audit-format.md` row: the spec
  text changes with the fix.
- `src/cli/resolve.rs:install` has the inverse inconsistency (filed as bug-925).
