# plan-61-F: Migrate the tree and make `description` required

Last updated: 2026-07-21
Effort: medium (1h–2h)
Depends on: plan-61-E
Produces:
- `description` set in all 81 `kind: "package"` manifests
- `description` promoted from warning to hard error for `kind: "package"`
- The regenerated golden corpus

Lands **last and alone**, because it is the only sub-plan that churns generated
output and the only one whose change is tree-wide. Both are reasons the
write-plan rule puts it here: output-churning work must not be mixed into
letters whose diffs need reviewing, and largest blast radius goes last.

The single behavioral outcome: `mfb build` on a `kind: "package"` project with no
`description` fails with a clear diagnostic, and every package in the tree
builds.

References:
- `plan-61-repo-web.md` §Measured populations — the 81 manifests and 41 goldens
- `plan-61-D-description-field.md` §4 — the warning this converts to an error
- `AGENTS.md:7-82` — the STOP rule on tests and goldens
- `AGENTS.md:139-145` — never regenerate goldens while a bug is live

## Prerequisites

See `plan-61-repo-web.md` §Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-61-E complete | `curl -sf "$REPO/packages/alice%23pkg" \| grep -q '"description"'` returns a non-null value | NOT MET |
| The tree is green before churning goldens | `scripts/test-accept.sh target/debug/mfb target/accept-actual` → 0 failures | UNVERIFIED |

That second row is not ceremony. `AGENTS.md:139-145` is explicit that
regenerating goldens while a bug is live enshrines the bug (bug-309). This
sub-plan regenerates goldens in bulk, so the tree must be green *first*, and a
failure found here is a real failure to fix, never a baseline to accept.

## 1. Goal

- Every `kind: "package"` manifest in the tree carries a `description`.
- A `kind: "package"` project without one is a build error.

### Non-goals

- **No weakening of any acceptance criterion or golden to make this pass.** If a
  golden diff appears that this change does not explain, it is evidence of a bug
  in plan-61-D, not a baseline to refresh. See the STOP rule.
- **No change to `kind: "executable"`.** Executables neither require nor reject
  `description`.
- **No bulk sed across the tree.** See §3.
- No new format change — plan-61-D closed the format.

## 2. Measured surface

Re-measure before starting; these are from 2026-07-21 and manifests may have been
added since.

| What | Count | Command |
|---|---|---|
| `kind: "package"` manifests | 81 | `find . -name project.json -not -path './target/*' -not -path '*/packages/*' -exec grep -ohE '"kind"[[:space:]]*:[[:space:]]*"[a-zA-Z]+"' {} + \| sort \| uniq -c` |
| …that also carry a `golden/` dir | 49 | see `plan-61-repo-web.md` §Measured populations |
| `.mfp` goldens | 16 | `find tests -name '*.mfp' -path '*/golden/*' \| wc -l` |
| `.hex` goldens | 2 | `find tests -name '*.hex' -path '*/golden/*' \| wc -l` |
| `.info` / `.audit` goldens | 23 | `find tests \( -name '*.info' -o -name '*.audit' \) -path '*/golden/*' \| wc -l` |

Distribution of the 81: `tests/syntax` 49, `tools/thread-package-sources` 18,
`tools/security-package-sources` 9, `tools/link-package-sources` 2,
`bindings/sqlite3` 1, `bindings/libsnd` 1, `benchmark/mfb` 1.

**This diverges from the project's standing pattern for new manifest fields**,
which is optional-with-a-documented-default: the worked precedent is plan-58-C's
`maxBuffer`, added optional and set in exactly one fixture
(`grep -rl maxBuffer tests --include=project.json | wc -l` → 1). The divergence is
deliberate and was the user's explicit requirement — a registry whose purpose is
a browsable, informative package index cannot have half its packages
description-less. But it *is* a divergence, and if it does not survive review the
fallback is to stop after plan-61-E and leave the warning permanent. That costs
nothing already built.

## 3. Migration approach — no unchecked tree-wide script

There is **no bulk tool for manifest fields**; the 1069 fixture manifests are
hand-maintained. Do not reach for one.

Specifically, do not `sed` across 81 files: BSD `sed` on macOS silently ignores
`\b`, so a word-boundary mutation appears to succeed while changing nothing, and
a fixture loop will then "prove" a behavior that was never exercised. If any
scripted edit is used, it must **assert the mutation landed** in every file
before anything downstream runs.

The descriptions themselves must be meaningful, not placeholder text. A
fixture named `pkg-02-type-confusion` gets a description saying what it tests.
Filling 81 files with `"description": "TODO"` would satisfy the validator and
defeat the entire purpose of the field — and would then be rendered on a public
website.

Work in batches by directory (the seven groups in §2), verifying after each, so a
mistake is localized and reviewable.

## Phases

> Tick `- [x]` in the same commit as the work. **An unticked box means NOT DONE.**

### Phase 1 — Migrate the non-fixture packages

The three real packages first: small, hand-written, and they prove the end-to-end
story before touching 78 fixtures.

- [ ] Add a meaningful `description` to `bindings/sqlite3/project.json`,
      `bindings/libsnd/project.json`, `benchmark/mfb/project.json`.
- [ ] Build each and confirm the warning from plan-61-D no longer fires.
- [ ] Confirm section 18 is present in the resulting `.mfp` for a signed build.

Acceptance: all three build with no missing-description warning, and a signed
build of `bindings/sqlite3` contains section 18 with the expected text.
Commit: —

### Phase 2 — Migrate the fixture packages

- [ ] Add descriptions to the 49 `tests/syntax` package manifests, in batches,
      each describing what the fixture tests.
- [ ] Add descriptions to the 18 `tools/thread-package-sources`, 9
      `tools/security-package-sources`, and 2 `tools/link-package-sources`
      manifests.
- [ ] Re-run the count and confirm zero `kind: "package"` manifests lack a
      description:
      `find . -name project.json -not -path './target/*' -not -path '*/packages/*' -exec grep -l '"kind"[[:space:]]*:[[:space:]]*"package"' {} + | while read f; do grep -q '"description"' "$f" || echo "MISSING $f"; done`
      → no output.
- [ ] Verify no file was left syntactically invalid: every migrated manifest
      still parses.

Acceptance: the "MISSING" command above produces no output, and every migrated
project still builds.
Commit: —

### Phase 3 — Regenerate goldens (the churn, isolated)

- [ ] Run `scripts/artifact-gate.sh target/debug/mfb` and record which goldens
      diff. **Explain every diff before regenerating any of them.** Expected:
      `.mfp` goldens gain section 18; `.info`/`.audit` goldens may gain a
      description line; `build.log` goldens lose the missing-description warning.
      Anything else is unexplained and must be investigated, not accepted.
- [ ] Seed with a filter, never bare: `scripts/sync-goldens.sh target/debug/mfb
      <name-glob>` per affected group. `sync-goldens.sh` **never creates** golden
      files — if a new golden kind is needed, pre-create it empty first.
- [ ] Re-run `scripts/artifact-gate.sh target/debug/mfb` and confirm zero diffs.

Acceptance: `artifact-gate.sh` reports 0 diffs, and every regenerated golden's
change is explained by one of the three expected causes above.
Commit: —

### Phase 4 — Flip to required (the behavioral change)

Last, so no intermediate commit leaves the tree red.

- [ ] In `src/manifest/mod.rs`, promote the missing-`description` warning for
      `kind: "package"` to a hard error, using the diagnostic code resolved in
      plan-61-D Phase 1.
- [ ] Update the schema table row in
      `src/docs/spec/tooling/01_project-manifest.md`: `required` becomes
      `yes¹` with a footnote reading "required when `kind` is `package`;
      optional and ignored for `executable`" — mirroring the existing `kind`
      footnote idiom at `:57`.
- [ ] Add a `tests/syntax/` fixture proving the new error: a `kind: "package"`
      project with no description fails to build with the expected diagnostic.
      Pre-create its `golden/build.log` empty, then seed it with a filtered
      `sync-goldens.sh`.
- [ ] Add a fixture proving `kind: "executable"` without a description still
      builds cleanly.
- [ ] Verify: `cargo build && cargo test --bin mfb spec`.

Acceptance: the new negative fixture fails the build with the expected
diagnostic and its `build.log` golden matches; an executable without a
description still builds; and the full acceptance suite is green.
Commit: —

## Validation Plan

- Tests: the two new `tests/syntax` fixtures (required-for-package fails,
  optional-for-executable succeeds), plus the existing `src/manifest/mod.rs`
  inline tests updated from warning to error.
- Coverage check: `sh scripts/coverage.sh && sh scripts/coverage-check.sh`.
- Runtime proof: `mfb build` a `kind: "package"` project with the `description`
  line deleted and observe the error; restore it and observe success.
- Doc sync: `src/docs/spec/tooling/01_project-manifest.md` (the `required`
  column and its footnote).
- Acceptance: `scripts/artifact-gate.sh target/debug/mfb` → 0 diffs, then the
  full `scripts/test-accept.sh target/debug/mfb target/accept-actual` → 0
  failures. This is the sub-plan where the full ~15-minute suite is
  non-negotiable; the fast gate is nearly blind to codegen and this change
  touches the payload of every package.

## Open Decisions

- **Should `mfb init` emit a `description` stub for `kind: "package"`?**
  *Recommended:* yes, with an empty string and a comment-free placeholder that
  fails validation until edited — so a new package author hits the requirement at
  `init` time rather than at first build. Check whether `mfb init` templates are
  in scope before committing to this; if it turns out to touch a template system
  this plan has not surveyed, drop it and file it separately rather than
  expanding scope here.

## Corrections

- *(none yet)*
