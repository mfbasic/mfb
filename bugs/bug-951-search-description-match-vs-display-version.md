# bug-951: `/search` matches description text from the newest release of any state but shows the newest *active* release's description

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Coordinator note: measured by the review agent against a local `mfb-repo` on 127.0.0.1 only; the coordinator did not re-run it.
Regression Test: (proposed) repository/src/store.rs `search_description_match_uses_the_displayed_release`

`Store::search_packages` matches description text against the newest `package_versions`
row by `created_at`, **regardless of state**. The result row it then attaches
(`latestVersion`, `latestState`, `description`) is the newest **active**
(`available`/`deprecated`) release. Once the newest release is yanked, blocked or
legal-tombstoned, the two disagree:

- a word shown in the result's own description no longer finds the package, and
- a word that appears only in a yanked/blocked/legal-tombstoned release's description
  still makes the package findable, even though that text is not displayed.

The correct behavior: the description match and the displayed description come from the same release,
the newest active one (or none).

References:

- src/docs/spec/package-manager/01_repository-protocol.md "The anonymous read surface": "`/search` matches description text as well … scoped to the newest version so an old release's wording cannot keep a package findable by a word its current release no longer contains."
- `SearchResult.latest_version` doc: "The newest **active** release (plan-126-A)".

## Failing Reproduction

Local registry; `alice#longdesc@0.8.0` with a description containing `zebraword`, then
`@0.9.0` with description `plain`:

```
mfb repo publish alice tb5                # 0.8.0, description "éé…zebraword…"
(version 0.9.0, description "plain");  mfb repo publish alice tb5
cd tb5 && mfb repo release-state yanked 0.9.0
curl '127.0.0.1:47716/search?q=zebraword'
curl '127.0.0.1:47716/search?q=longdesc'
```

- Observed: `q=zebraword` → `{"total":0,"results":[]}`. `q=longdesc` → one result with
  `"latestVersion":"0.8.0"` and a `description` that contains `zebraword`.
- Expected: `q=zebraword` finds `alice#longdesc`, because its displayed (current active) description contains the word.

## Root Cause

`repository/src/store.rs:search_packages`: the `EXISTS (… ORDER BY created_at DESC, id DESC LIMIT 1 …)`
sub-select has no `state IN ('available','deprecated')` filter, but `latest_statement` below it does.

## Goal

- Add the same state allowlist to the description sub-select. Pin it with a test that yanks the newest release.

### Non-goals (must NOT change)

- Ident/owner match ranking; the 200-character clamp.

## Blast Radius

- `store.rs:search_packages` only.
