# plan-126-A: Latest-active-version selection

Last updated: 2026-09-06
Overall Effort: x-large (1d–3d) — the whole plan-126 feature (A–F)
Effort: medium (1h–2h)
Depends on: nothing

The registry's "latest version" is computed with **no release-state filter**, so a
package whose newest release is `yanked` advertises that release as its headline
version — on the search page with no state badge at all. This sub-plan gives the
registry one `latest_active_version` selection that matches the vocabulary the
install client already enforces, and routes every human-facing "latest" through it.

Behavioral outcome: for a package whose newest release by `published_at` is
`yanked`, `blocked`, or `legal-tombstoned`, the search page and the package
Overview header both name the newest `available`/`deprecated` release instead —
and when no such release exists, they say so rather than naming an ineligible one.

References:

- `src/cli/pkg.rs:1369-1373` — `state_is_floating_eligible`, the project's
  definition of an installable release state.
- `src/cli/pkg.rs:1377-1414` — `select_index_version`, the client's newest-eligible
  selection this sub-plan mirrors server-side.
- `repository/src/server.rs:2118-2126` — the maintainer release-state vocabulary.
- `.ai/testing-gates.md` — which gates cover which code.

## Prerequisites

These are a precondition on the whole feature, not a dependency to negotiate.
Sub-plans B–F point here rather than restating them.

| Must be true | Command | Status |
|---|---|---|
| Workspace builds and tests clean at HEAD | `rustup run 1.96.0 cargo test --no-fail-fast` → 0 failures | MET (measured 2026-09-12: 161 suites, 5317 passed, 0 failed, exit 0) |
| `repository` is a workspace member (so its tests are in the denominator) | `rustup run 1.96.0 cargo metadata --no-deps --format-version 1` → `workspace_members` length 2 | MET (re-measured 2026-09-12: 2 members) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you.

## 1. Goal

- One server-side selection, `latest_active_version`, defined as *newest by
  `published_at` among releases whose state is `available` or `deprecated`*.
- `PackageDetailResponse.latest_version` and the search results' headline version
  both come from it.
- The search results page carries the selected version's state, so a `deprecated`
  headline is visibly deprecated.
- A package with **no** active release renders an explicit "no active release"
  state rather than silently naming a yanked one.

### Non-goals (explicit constraints)

- **Client-side resolution does not change.** `src/cli/pkg.rs:944`, `1369-1414`
  already filter correctly and are pinned by tests at `src/cli/pkg.rs:2352`
  (2.0.0 yanked → resolves 1.5.0) and `2774-2785`. Do not touch them.
- **`GET /index/:ident` does not change.** It deliberately returns *every* version
  with its `state` (`repository/src/server.rs:1603-1627`) and the client decides.
  Filtering server-side there would break exact-pin installs of a yanked release,
  which `select_index_version` explicitly supports (`src/cli/pkg.rs:1388-1401`).
- **Ordering stays `published_at`, not semver.** The client picks with
  `max_by_key(|entry| entry.published_at)` (`src/cli/pkg.rs:1408`); making the
  registry order by semver would put the two surfaces in disagreement. This was
  raised as a possible defect during research and is **not** one — it is the
  project's consistent convention. (See Corrections.)
- **The Overview versions table stays complete and unfiltered.** It lists every
  version including yanked ones by design — `repository/src/web/mod.rs:505-509`
  says so in the page copy. Only the *header* and the fold default change.
- No change to `package_versions` schema, the publish path, or any signed payload.

## 2. Current State

`latest_version` is produced once, at `repository/src/server.rs:1433`:

```rust
latest_version: versions.first().map(|version| version.version.clone()),
```

`versions` comes from `Store::package_detail`, whose query
(`repository/src/store.rs:1456-1462`) is:

```sql
SELECT pv.id, pv.version, pv.hash, pv.created_at, pv.state, ...
  FROM package_versions pv JOIN packages p ON p.id = pv.package_id
 WHERE p.ident = ?1
 ORDER BY pv.created_at DESC, pv.id DESC
```

There is no `state` predicate. The search path repeats the pattern independently at
`repository/src/store.rs:1706-1713` (`ORDER BY pv.created_at DESC, pv.id DESC LIMIT 1`),
also with no `state` predicate, and its row type `SearchResultRow`
(`repository/src/store.rs:2965` region) carries no state at all.

Consumers:

| Consumer | Site | Severity |
|---|---|---|
| Package Overview header `latest v<N>` | `repository/src/web/mod.rs:455-457` | Cosmetic — the table below badges the same version yanked, so the correct data is on screen |
| Overview target-row fold default | `repository/src/web/mod.rs:524` | Cosmetic |
| Search result version chip | `repository/src/web/mod.rs:340-342` | **Wrong** — `SearchRow` (`web/mod.rs:281-287`) has no `state` field, so nothing marks it |
| `PackageDetailResponse.latest_version` JSON | `repository/src/server.rs:1433` | Wrong — API consumers read it as "the version to use" |

### Measured populations

| What | Count | Command |
|---|---|---|
| Release states in the vocabulary | 5 (`available`, `deprecated`, `yanked`, `blocked`, `legal-tombstoned`) | `sed -n '2310,2316p' src/cli/pkg.rs` |
| States the maintainer route accepts | 3 (`available`\|`deprecated`\|`yanked`) | `sed -n '2118,2126p' repository/src/server.rs` |
| States counted **active** by the client | 2 (`available`, `deprecated`) | `sed -n '1371,1373p' src/cli/pkg.rs` |
| `latest_version` occurrences in the repository crate | 9 | `grep -c latest_version repository/src/web/mod.rs repository/src/server.rs repository/src/store.rs` → 6+3+1, see below |
| `latest_version` sites in `web/mod.rs` | 5 (lines 284, 340, 436, 455, 524, 974) | `grep -n latest_version repository/src/web/mod.rs` |
| Tests in `repository/src/web/mod.rs` | 8 | `grep -c '#\[test\]' repository/src/web/mod.rs` |
| Repository crate lib tests total | 351 | `grep -rc '#\[test\]\|#\[tokio::test\]' repository/src/*.rs repository/src/web/*.rs \| awk -F: '{s+=$2} END {print s}'` |

### Verified properties

- **The client is unaffected and correct.** Read `select_index_version`
  (`src/cli/pkg.rs:1377-1414`): it filters by `state_is_floating_eligible` before
  `max_by_key(published_at)`, and handles an explicit `requested` version
  separately so a yanked release stays installable by exact pin. Tests at
  `src/cli/pkg.rs:2352-2362` and `2774-2785` pin both behaviors.
- **`blocked` and `legal-tombstoned` really are reachable states.** They are set
  by an operator path, not the maintainer route (`repository/src/server.rs:433`,
  `:2118`), and appear in a server test fixture (`repository/src/server.rs:4764`).
  A filter written as `!= 'yanked'` would therefore be **wrong**; it must be an
  allowlist of `available`/`deprecated`.
- **`SearchRow` genuinely has no state field.** Read
  `repository/src/web/mod.rs:281-287` — five fields, none of them state — and the
  render at `:340-342` emits the bare version chip. Adding the field is a real
  schema change to the view type, not a display tweak.

## 3. Design Overview

One function, two call sites, one new view field.

1. **`Store::latest_active_version(ident) -> Option<(String, String)>`** returning
   `(version, state)`, implemented as a state-allowlist predicate over the existing
   ordering. Placed next to `package_detail` in `repository/src/store.rs`.
2. **`package_detail`** stops using `versions.first()` and calls it.
3. **`search_packages`** stops using its private latest-version statement's
   unfiltered form and applies the same predicate, returning the state alongside.
4. **`SearchRow` gains `latest_state: Option<String>`**, and the search page renders
   the existing `state state--<mod>` badge (already defined for the Overview at
   `repository/src/web/mod.rs:541`) next to the version chip.
5. **Empty case**: when there is no active release, `latest_version` is `None`. The
   Overview header already handles `None` (`@if let Some(latest)` at `:455`) by
   omitting the chip; add explicit copy so the omission reads as a statement rather
   than missing data. Same for the search row.

**Where correctness risk concentrates:** the state allowlist. Writing it as a
denylist (`!= 'yanked'`) silently admits `blocked` and `legal-tombstoned`, which is
the same class of bug being fixed. The allowlist must be a single shared predicate,
not two SQL literals that can drift — mirror `state_is_floating_eligible`'s shape.

**Byte-identity is not this sub-plan's gate.** Nothing here touches codegen; the
`.ncode`/`.ncodesum` goldens are irrelevant and `scripts/artifact-gate.sh` will
report 0 diffs whether or not the work is correct. The gate is repository crate
tests plus the `cli_repo_*` acceptance suites.

**Rejected alternative — filter in `package_detail`'s SQL and drop yanked rows
from the response.** That would hide versions from the transparency view, which the
page's own copy forbids (`repository/src/web/mod.rs:505-509`: "Nothing is hidden or
collapsed — a version that has disappeared from this list would itself be evidence
of tampering"). The filter belongs to the *selection*, never to the *listing*.

**Rejected alternative — order by semver.** See Non-goals; it would diverge the
registry from `select_index_version`.

## Compatibility / Format Impact

- `PackageDetailResponse.latest_version` (JSON, `GET /packages/:ident`) changes
  **value** for packages whose newest release is not active. The field's type and
  name are unchanged; `null` was already a possible value (a package with no
  versions), so no consumer gains a new shape.
- `SearchResponse` **and `PackageDetailResponse`** each gain a `latestState`
  field (corrected — see Corrections). Additive; existing consumers ignore it.
- `GET /index/:ident`, the publish path, the DB schema, and every signed payload are
  untouched.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes. Use `- [~]` for partially done plus one line on
> what remains. Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill the
> `Commit:` line the moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — The selection, with tests, no consumers

Adds the predicate and query in isolation so the semantics are pinned before
anything reads them.

- [x] Add `pub fn state_is_active(state: &str) -> bool` to
      `repository/src/validation.rs`, `matches!(state, "available" | "deprecated")`,
      with a doc comment citing `state_is_floating_eligible` as the definition it
      mirrors and naming all five states so the allowlist rationale is on the page.
- [x] Add `Store::latest_active_version(&self, ident: &str) -> Result<Option<(String, String)>, String>`
      to `repository/src/store.rs` beside `package_detail`: same `ORDER BY
      pv.created_at DESC, pv.id DESC`, `WHERE p.ident = ?1 AND pv.state IN
      ('available','deprecated')`, `LIMIT 1`, returning `(version, state)`.
- [x] Tests in `repository/src/store.rs`'s test module: newest is yanked → returns
      the older active one; newest is `blocked` → same; newest is
      `legal-tombstoned` → same; all versions inactive → `None`; no versions →
      `None`; newest is `deprecated` → returns it *with* state `deprecated`.
      Six tests plus a `store_with_versions` helper; the yanked and all-inactive
      tests additionally assert `list_package_versions` still returns both rows,
      so the selection filter cannot be mistaken for a listing filter.
- [x] Test in `repository/src/validation.rs` asserting `state_is_active` agrees
      with `state_is_floating_eligible` across all five states
      (`active_states_are_an_allowlist_matching_the_install_client`), citing the
      sibling table by symbol + grep rather than by line number.

Acceptance: MET. `rustup run 1.96.0 cargo test -p mfb_repository --lib
--no-fail-fast` → `373 passed; 0 failed` (was 366; +6 store, +1 validation).
The denylist proof was run, not assumed: rewriting the predicate as
`pv.state != 'yanked'` turns **three** tests red —
`latest_active_version_skips_a_blocked_newest_release`,
`..._skips_a_legal_tombstoned_newest_release` and
`..._is_none_when_every_version_is_inactive` (`test result: FAILED. 3 passed;
3 failed`) — and the allowlist was restored.
Commit: 1285d1f96

### Phase 2 — Route `package_detail` and the Overview through it

- [x] Replace `versions.first()` in `package_detail`
      (`grep -n "fn package_detail" repository/src/server.rs`) with a call to
      `Store::latest_active_version`, keeping the full unfiltered `versions`
      vector in the response exactly as it is today.
- [x] Add `latest_state: Option<String>` to `crate::web::PackageView` and
      populate it in `package_page_html`. **Also** added `latestState` to
      `PackageDetailResponse` — see Corrections; without it the HTML page would
      have had to run its own second selection, breaking the handler's stated
      "the two surfaces cannot disagree" property.
- [x] Render the state badge beside the header's `latest v<N>` chip using the
      existing `state state--<modifier>` classes and `state_modifier`.
- [x] When `latest_version` is `None` **and** the package has versions, render
      explicit copy in the header — "no active release" plus "every published
      version is yanked or blocked" — rather than omitting the chip silently.
      A package with **no** versions takes neither branch: there is nothing to
      state the absence of. New `.pkg-latest--none` CSS rule reuses the existing
      danger tokens.
- [x] Tests in `repository/src/web/mod.rs`: four —
      `the_header_names_the_newest_active_release_while_the_table_keeps_the_yanked_one`,
      `a_deprecated_headline_release_carries_its_state_badge`,
      `no_active_release_is_stated_not_omitted`, and
      `a_package_with_no_versions_renders_no_release_copy_at_all`.
- [x] Added task: a **handler-seam** test the web tests cannot reach —
      `package_detail_names_the_newest_active_release_not_a_yanked_newest`
      (`repository/src/server.rs`). See Corrections: the pre-existing
      `package_detail_lists_every_version_including_yanked_ones` yanks the
      *older* version, so both selections agree there and it stayed green
      throughout the bug's lifetime. Its misleading "Newest first, so
      `latestVersion` is the un-yanked 2.0.0" comment was corrected in place.

Acceptance: MET. `the_header_names_the_newest_active_release_while_the_table_keeps_the_yanked_one`
renders `package_page` for a view whose newest version is yanked and asserts the
header names `latest v1.5.0`, does **not** contain `latest v2.0.0`, and the table
still carries the yanked row (`state--yanked` present). `cargo test -p
mfb_repository --lib --no-fail-fast` → `378 passed; 0 failed`.
Commit: 90150eff1

### Phase 3 — Search results carry state (the actual bug)

Largest user-visible change, landed last.

- [x] Apply the `state_is_active` allowlist to the per-package latest-version
      statement in `Store::search_packages`
      (`grep -n "failed to prepare latest-version query" repository/src/store.rs`)
      and select `pv.state` alongside `pv.version`.
- [x] Add `latest_state: Option<String>` to `SearchResultRow`
      (`grep -n "pub struct SearchResultRow" repository/src/store.rs`) and to
      `SearchRow` (`grep -n "pub struct SearchRow" repository/src/web/mod.rs`).
- [x] Add the field to the search JSON response type `SearchResult` in
      `repository/src/server.rs` as `latestState` and populate it.
- [x] Render the badge next to `span."result__ver"` in `search_page`, reusing the
      existing `state state--<modifier>` classes rather than adding a parallel
      `.result__state` rule set — the badge is the *same* badge the Overview
      version table uses, and a second rule set for one would drift. The one new
      rule is `.result__ver--none` for the no-active-release statement.
- [x] Tests: three new —
      `search_names_the_newest_active_release_and_carries_its_state` (JSON: yanked
      newest → older active; deprecated newest → returned *with* its state; all
      withdrawn → row kept, version `null`),
      `the_search_page_never_shows_a_yanked_version_in_the_result_chip` (HTML),
      and `the_search_page_badges_a_deprecated_headline_release` (HTML).
      `search_ranks_exact_then_prefix_then_substring` gained a `latest_state`
      assertion.
- [x] Added task: extend the pre-existing
      `search_results_escape_html_metacharacters_in_publisher_values` to cover
      the new field. `latest_state` reaches a `class=` attribute via
      `state_modifier`, so a hostile value is a new XSS surface — see
      Corrections for the false-positive assertion this first produced.

Acceptance: MET. `rustup run 1.96.0 cargo test -p mfb_repository --lib
--no-fail-fast` → `381 passed; 0 failed`.
`the_search_page_never_shows_a_yanked_version_in_the_result_chip` drives the real
`/search.html?q=` route and asserts the yanked `9.9.9` appears **nowhere** in the
response body while `v1.0.0` does; with the fallback withdrawn too it asserts
`no active release` is present and `v1.0.0` is gone.
Also green: `cargo test --test cli_repo_publish --test cli_repo_install --test
cli_repo_auth --test cli_repo_governance` → 4 / 7 / 9 / 6 passed, 0 failed.
Commit: —

## Validation Plan

- **Tests:** repository crate lib tests (`repository/src/store.rs`,
  `repository/src/validation.rs`, `repository/src/web/mod.rs`), including the
  negative cases above — `blocked`, `legal-tombstoned`, all-inactive, and no-versions.
- **Coverage check:** the repository crate is a workspace member
  (`cargo metadata --no-deps` → 2 members), so its 351 lib tests are in the
  `cargo test` denominator. `scripts/artifact-gate.sh` covers **none** of this code
  — a 0-diff there is meaningless for this sub-plan and must not be cited as a gate.
- **Runtime proof:** start a local `mfb-repo`, publish two versions, `mfb pkg
  release-state <ident> <newest> yanked`, then load `/p/<ident>` and
  `/search.html?q=<ident>` and confirm both name the older version while the
  Overview table still lists the yanked one.
- **Client non-regression:** `rustup run 1.96.0 cargo test --no-fail-fast` must keep
  `src/cli/pkg.rs`'s resolution tests green — in particular
  `pkg.rs:2352` and `2774-2785`. This sub-plan must not move them.
- **Acceptance:** `rustup run 1.96.0 cargo test --no-fail-fast` (which includes
  `tests/golden.rs` → `scripts/artifact-gate.sh all`), plus
  `tests/cli/cli_repo_publish.rs` and `tests/cli/cli_repo_install.rs`.
- **Doc sync:** none — no `mfb man` page or `src/docs/spec/**` text describes the
  registry web UI's latest-version selection. Confirm with
  `grep -rn "latest version" src/docs/` before ticking.
- **Format:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Does `deprecated` count as active?** Recommended **yes** — it matches
  `state_is_floating_eligible` (`src/cli/pkg.rs:1372`), which installs deprecated
  releases on a floating add. Alternative: treat only `available` as active, which
  would make the registry stricter than the installer and hide a package whose
  only maintained line is deprecated. (§1)
- **No-active-release copy wording.** Recommended "no active release — every
  published version is yanked or blocked". Needs to read as a statement, not an
  error. (§Phase 2)

## Corrections

- **Every line citation in this sub-plan had drifted.** The plan was written
  2026-09-06 against line numbers that no longer resolve. Measured 2026-09-12:
  `latest_version: versions.first()…` is at `repository/src/server.rs:1477`, not
  `:1433`; `Store::package_detail` at `store.rs:1592`, not `:1456`; the search
  latest-version statement at `store.rs:1858-1867`, not `:1706-1713`;
  `SearchResultRow` at `store.rs:3286`, not `:2965`; `state_is_floating_eligible`
  at `src/cli/pkg.rs:1374`, not `:1369`. Everything below cites symbol + grep
  instead, per the project's citation rule.

- **`PackageDetailResponse` needed a `latestState` field, which
  § Compatibility did not list.** The plan called for `latest_state` on
  `web::PackageView` populated in `package_page_html`, but that handler builds
  its view *entirely* from `package_detail`'s response and its doc comment makes
  the "renders the same output the JSON route serves, so the two surfaces cannot
  disagree" property normative. Threading the state any other way (a second
  `latest_active_version` call in the HTML handler, or re-deriving it by scanning
  `versions` for the matching row) would add a second path to the same fact.
  Added `latestState` to the JSON response instead — additive, `null` exactly
  when `latestVersion` is, and symmetric with the `latestState` the plan already
  specified for `SearchResponse`. § Compatibility should have said "both
  responses gain `latestState`".

- **The pre-existing yanked test never covered the bug.**
  `package_detail_lists_every_version_including_yanked_ones`
  (`repository/src/server.rs`) yanks **1.0.0** and leaves **2.0.0** available —
  the *older* release. Under that fixture `versions.first()` and
  newest-active agree, so the test was green for the entire life of the defect,
  and its comment ("Newest first, so `latestVersion` is the un-yanked 2.0.0")
  positively asserted the broken selection was correct. Comment corrected in
  place; a new handler test covers the newest-yanked shape, including the
  further case where the fallback is withdrawn too and the field goes `null`.

- **`latest_state` is a new XSS surface, and the first assertion written for it
  was a false positive.** The field reaches a `class=` attribute through
  `state_modifier`, so it joined the hostile-value set in
  `search_results_escape_html_metacharacters_in_publisher_values`. The first
  assertion — `!rendered.contains("onmouseover=")` — went **red**, and the cause
  was the test's own documented trap: maud escapes the value to
  `&quot; onmouseover=&quot;alert(1)`, which legitimately still contains the text
  `onmouseover=`. Corrected to assert the absence of an attribute *break*
  (`" onmouseover="`) plus the presence of the escaped form, and that
  `state_modifier`'s total map sends the out-of-vocabulary value to
  `state--other` so it never reaches the attribute at all.

- **A fully-withdrawn package keeps its search result row.** The plan did not say
  which way this should go. Dropping the row was rejected: the query found the
  package, it exists, and silently hiding it is the same truncation the
  Overview's complete version table exists to prevent. The row renders with a
  `no active release` statement in place of the version chip.

- **Populations re-measured 2026-09-12** (plan figures in parentheses):
  repository crate lib tests **366** at HEAD (351);
  `CREATE TABLE IF NOT EXISTS` statements in `store.rs` **22** (21);
  `repository/src/abi.rs` **1,488 lines / 29 tests** (1,063 / 21 — it grew with
  bug-578); committed `.mfp` fixtures **160** (159). None of these re-scope any
  phase; they are recorded so the next reader does not trust a stale number.

- **Semver ordering is not a defect.** During research this was raised as a third
  consequence ("a 1.9.1 published after 2.0.0 reads as latest"). Reading
  `select_index_version` (`src/cli/pkg.rs:1408`) shows the install client selects
  with `max_by_key(|entry| entry.published_at)` — publish-time ordering is the
  project's consistent convention across client and registry, not a registry bug.
  Changing it here would create the divergence. Recorded as a Non-goal.
- **The state vocabulary is five, not three.** Initial research read
  `src/cli/pkg.rs:341` (the `release-state` CLI's accepted values:
  available/deprecated/yanked) as the whole vocabulary.
  `state_is_floating_eligible` (`src/cli/pkg.rs:1371`) and its test
  (`src/cli/pkg.rs:2310-2316`) show two operator-only states as well —
  `blocked` and `legal-tombstoned`. A `state != 'yanked'` filter would therefore
  reproduce the bug for those two. The allowlist form in Phase 1 exists because of
  this correction.

## Summary

The real engineering content is one SQL predicate written as an allowlist rather
than a denylist, and one new field on a view type that never had it. The risk is
not implementation difficulty but scope discipline: the transparency listing must
stay unfiltered, and the install client's resolution must stay untouched. Sub-plans
B–F do not depend on this one, but the Docs tab (F) consumes
`Store::latest_active_version` directly, so landing it first removes a duplicate
selection later.
