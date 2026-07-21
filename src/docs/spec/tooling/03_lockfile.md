# Lock File (mfb.lock)

`mfb.lock` records the resolved dependency state a project was last reconciled
against, so a later build or audit can detect that the declared dependency set
has drifted from what was locked. It lives at the project root beside
`project.json`. It is **written by `mfb pkg update`**
and **applied by `mfb pkg install`**, which fetches each locked blob by hash and
verifies it — never re-resolving. `mfb audit` also consumes it (the `lockfile`
section and `AUDIT-LOCK-*` findings).

## Writer: `mfb pkg update`

`mfb pkg update` runs the resolver over the project's registry dependencies and
writes `mfb.lock` with a byte-stable formatting, so re-resolving an unchanged
project reproduces the file exactly. For each dependency it records the
requested and selected versions, the content hash, the pinned owner `identKey`
(metadata form) and its fingerprint, and the current release `state`; the file
also carries the registry `repoFingerprint` and the pinned transparency-log
`checkpoint` (size + root). Signing keys are one-off per package, so
there is no key status/window to record. `mfb pkg install` reads this file,
cross-checks `repoFingerprint` against the pinned `server.pub`, and installs
each package by fetching `/blob/<hash>` and re-verifying the trust chain
against the locked `identKey` — no `/index` lookups.[[src/cli/resolve.rs:write_lock]][[src/cli/resolve.rs:install]]

The resolver picks, for every dependency, the highest install-eligible version
(`available`/`deprecated`; `yanked` only under an exact pin) whose exported
`ABI_INDEX` is a **superset** of every requirer's needs. A dependency that
imports another anchors that import at the ABI it compiled against; two
requirers that disagree on a symbol's hash are a **diamond conflict**, reported
by naming both requirers and the symbol. A `pin` dependency bypasses the search
and takes its exact version.[[src/cli/resolve.rs:select_node]]

## Location and presence

| Aspect | Value |
| --- | --- |
| Path | `<project>/mfb.lock` (sibling of `project.json`) |
| Required | No — absence is not an error unless `--locked` is set |
| Written by | `mfb pkg update` |
| Read by | `mfb audit` (the `lockfile` section + `AUDIT-LOCK-*` findings) |

The audit collector probes the path; a missing file yields a summary with
`present = false` and no version/hash comparison, while a present file is parsed
as JSON. [[src/audit/collect/lockfile.rs:collect_lockfile]]

## JSON shape

`mfb.lock` is a JSON object. Only two fields are read:

```json
{
  "lockfileVersion": 1,
  "projectHash": "5b1c…<64 lowercase hex chars>"
}
```

| Field | JSON type | Read as | Meaning |
| --- | --- | --- | --- |
| `lockfileVersion` | number | integer (`f64` truncated to `i64`) | Lock-file format version. Surfaced verbatim; not validated against a known set. |
| `projectHash` | string | string | Canonical hash of the declared dependency request set (see below). A missing/non-string value reads as the empty string, which can never match a real hash. |

Both fields are optional at the parse layer: a malformed object (or one missing a
key) leaves the corresponding summary value unset rather than erroring. Parsing
that fails entirely (unreadable file or invalid JSON) leaves `version` and the
hash-match result unset while still reporting `present = true`. [[src/audit/collect/lockfile.rs:collect_lockfile]]

Any other keys a writer chooses to record (resolved versions, content hashes,
sources) are **ignored** by the current reader; only `lockfileVersion` and
`projectHash` participate in policy.

## `projectHash` algorithm

`projectHash` is the lowercase-hex SHA-256 of a canonical, order-independent
serialization of the manifest's `packages[]` request tuples. It hashes what the
project *requests*, not what is installed — so it changes when a dependency is
added, removed, or its request fields edited, but not when an installed `.mfp`
changes on disk. [[src/audit/collect/mod.rs:project_hash]]

The exact construction:

1. Read `packages[]` from the parsed `project.json` manifest. Each entry is
   normalized to a request tuple `(name, ident, version, pin, source)`; entries
   whose `name` is blank are dropped, `ident` defaults to `name`, `version` and
   `source` default to the empty string, and `pin` defaults to `false`.
   [[src/manifest/package.rs:project_package_dependency]]
2. Render each tuple to a line by joining the five fields with a NUL (`U+0000`)
   separator and appending a trailing newline (`\n`):

   ```text
   name \0 ident \0 version \0 pin \0 source \n
   ```

   `pin` is rendered as the boolean's textual form (`true` / `false`).
3. **Sort** the rendered lines lexicographically (byte order). This makes the
   hash independent of the order packages appear in `project.json`.
4. Feed the sorted lines, in order, into a single SHA-256 stream (each line's
   UTF-8 bytes, including its `\0` separators and trailing `\n`).
5. The digest is rendered as lowercase hexadecimal (64 chars). [[src/cli/pkg.rs:hex_bytes]]

An empty or absent `packages[]` hashes the empty input — a fixed digest, the
SHA-256 of zero bytes. Comparison is exact string equality against the stored
`projectHash`; there is no normalization of the stored value. [[src/audit/collect/lockfile.rs:collect_lockfile]]

## `mfb pkg install` drift classification

A `projectHash` mismatch means the manifest's request set changed since the lock
was written, but it does not say *how*. `mfb pkg install` therefore diffs the
manifest's registry dependencies against `packages[]` in the lock, matched on
`ident`, and decides per difference.[[src/cli/resolve.rs:classify_drift]]

| Condition | Class | Outcome |
|---|---|---|
| declared in `project.json`, absent from the lock | `Added` | error |
| present in the lock, no longer declared | `Removed` | error |
| `version` differs and `pin` is `false` | `FloorMoved` | **warning, install continues** |
| `version` differs and `pin` is `true` | `PinMoved` | error |
| same `ident`, different `name` | `Renamed` | error |
| no difference in any field the lock records | *unattributable* | error |

Only `FloorMoved` is recoverable. Under `pin: false` the manifest's `version` is
an **ABI floor**, not a demand for that exact version (see
`./mfb spec tooling cli-reference`, `pkg add` Pin Inference), so moving it does
not invalidate the locked selection. `install` warns on stderr, naming the new
floor, the version the lock was resolved against, and the version it **selects**
— the last of these because under `pin: false` the locked `requested` and
`selected` differ, and the selected one is what lands on disk — then installs the
locked selection. The exit code stays `0`.

All differences are classified before any outcome is decided, so a project with
several drifted dependencies reports every one in a single run rather than
surfacing them one re-run at a time.

### Why a mismatch can be unattributable

`projectHash` covers the full request tuple `(name, ident, version, pin,
source)`, but the lock records only `name`, `ident`, `requested` and `selected`.
**`pin` and `source` are hashed and not stored.** Flipping either therefore
produces a mismatched hash with every diffable field equal, and the diff finds
nothing.

This asymmetry is intentional, and `install` reports it honestly rather than
inventing a cause:

```text
error: mfb.lock does not match project.json, but the difference is not in a
field the lock records (most likely `pin` or `source` changed); run `mfb pkg update`
```

Erring toward refusal is the conservative direction. Proceeding on the grounds
that nothing recognizable changed would install the old locked blob after a
`source` edit had pointed the dependency somewhere else entirely.

## `--locked` policy

The `mfb audit --locked` flag elevates lock-file staleness/absence from advisory
to fatal. It is plumbed through `AuditInputs.locked` into the lock-file summary
and the finding pass; without it, the same conditions are non-fatal. [[src/audit/collect/findings.rs:lockfile_findings]]

| Condition | Without `--locked` | With `--locked` |
| --- | --- | --- |
| `mfb.lock` absent | no finding | `AUDIT-LOCK-MISSING`, severity **error** |
| `projectHash` mismatch | `AUDIT-LOCK-STALE`, severity **warning** | `AUDIT-LOCK-STALE`, severity **error** |
| `mfb.lock` unreadable / not a JSON object | `AUDIT-LOCK-MALFORMED`, severity **warning** | `AUDIT-LOCK-MALFORMED`, severity **error** |
| `projectHash` matches | no finding | no finding |

A missing lock file under `--locked` short-circuits: the missing-finding is
emitted and the stale check is skipped (there is nothing to compare). The stale
check fires only when the file is present *and* the hash comparison resolved to a
definite mismatch (`Some(false)`); an unparseable lock file leaves the result
unset and emits no stale finding. [[src/audit/collect/findings.rs:lockfile_findings]]

`AUDIT-LOCK-MISSING`, `AUDIT-LOCK-STALE` and `AUDIT-LOCK-MALFORMED` are category
`lockfile` findings. The
finding catalogue (codes, categories, severities, and the `mfb.audit.v1` JSON
envelope they appear in) is owned by `./mfb spec tooling audit-format`; this
topic only states which lock-file conditions raise them.

Any error-severity finding (from any category, including an elevated
`AUDIT-LOCK-*`) makes `mfb audit` exit non-zero. The exit-code contract for the
command itself lives in `./mfb spec tooling cli-reference`.

## Audit report representation

When `mfb audit` renders its report, the lock-file state appears as a `lockfile`
section. The JSON form (`--format json`) uses these keys, distinct from the
on-disk `mfb.lock` keys above:

| Audit JSON key | Source | Notes |
| --- | --- | --- |
| `path` | always `"mfb.lock"` | display path, project-relative |
| `present` | file existence | |
| `locked` | the `--locked` flag | echoes the request |
| `lockfileVersion` | on-disk `lockfileVersion` | `null` when absent/unparsed |
| `projectHashMatches` | on-disk `projectHash` vs computed | `null` when absent/unparsed |

The on-disk `projectHash` string is **not** echoed in the report — only the
boolean match result is. [[src/audit/collect/lockfile.rs:collect_lockfile]]

## See Also

* ./mfb spec tooling audit-format — the `AUDIT-LOCK-*` finding catalogue, severities, and the `mfb.audit.v1` JSON envelope
* ./mfb spec tooling project-manifest — the `packages[]` request fields hashed into `projectHash`
* ./mfb spec tooling cli-reference — `mfb audit` flags and the command's exit codes
* ./mfb spec architecture packages — version constraints, pin/install policy, and where a writer of `mfb.lock` would fit
* ./mfb spec diagnostics rule-codes — the `6-603` `LOCKFILE_MISMATCH` diagnostic family for resolved-state mismatches
