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
- `plan-61-repo-web.md` §Measured populations — the 81 manifests and 41 golden
  *files* (16 `.mfp` + 2 `.hex` + 23 `.info`/`.audit`), spread across 49 golden-
  carrying fixture *directories*. Both numbers are correct and describe different
  things; the umbrella uses 49 at `:270-273` and 41 at `:559`.
- `plan-61-D-description-field.md` §4 — the warning this converts to an error
- `AGENTS.md:7-82` — the STOP rule on tests and goldens
- `AGENTS.md:139-145` — never regenerate goldens while a bug is live

## Prerequisites

See `plan-61-repo-web.md` §Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-61-E complete | `curl -sf "$REPO/packages/alice%23pkg" \| grep -q '"description"'` returns a non-null value | **MET** (2026-07-21): E's runtime proof ran this against a live `mfb-repo` and got `"description":"Zygomorphic layout primitives with native BLAS kernels."` — non-null, from a real `mfb build` artifact via `backfill-metadata`. E is archived. |
| The tree is green before churning goldens | `scripts/test-accept.sh target/debug/mfb target/accept-actual` → 0 failures | **NOT MET at F's start — 60 mismatches — cause identified and fixed by this sub-plan, not accepted.** All 60 are plan-61-D's new `2-200-0016` warning appearing in captured output (59 `build.log`, 1 `.audit`). Verified single-cause: **zero removed lines** across all 60 diffs, and every added line is one of the warning's four parts (79 warn + 79 detail + 79 caret + 237 source-echo = 79×3). No pre-existing failure is hiding among them. This is a real failure being **fixed** — F Phase 2 gives every package a description, the warning stops firing, and those goldens return to their committed content untouched. Re-checked after Phase 2; see Phase 3. |

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
| `kind: "package"` manifests | ~~81~~ **91** (see §Corrections) | `find . -name project.json -not -path './target/*' -not -path '*/packages/*' -exec grep -ohE '"kind"[[:space:]]*:[[:space:]]*"[a-zA-Z]+"' {} + \| sort \| uniq -c` |
| …that also carry a `golden/` dir | 49 | see `plan-61-repo-web.md` §Measured populations |
| `.mfp` goldens | 16 | `find tests -name '*.mfp' -path '*/golden/*' \| wc -l` |
| `.hex` goldens | 2 | `find tests -name '*.hex' -path '*/golden/*' \| wc -l` |
| `.info` / `.audit` goldens | 23 | `find tests \( -name '*.info' -o -name '*.audit' \) -path '*/golden/*' \| wc -l` |

Distribution, **corrected** (`find . -name project.json -not -path './target/*'
-exec grep -l '"kind"[[:space:]]*:[[:space:]]*"package"' {} +`, 2026-07-21):
`tests/syntax/native` 35, `tests/syntax/resources` 11, `tests/syntax/packages`
**10**, `tests/syntax/project` 3 (= 59 under `tests/syntax`, not 49),
`tools/thread-package-sources` 18, `tools/security-package-sources` 9,
`tools/link-package-sources` 2, `bindings/sqlite3` 1, `bindings/libsnd` 1,
`benchmark/mfb/workers` 1. **Total 91.**

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

- [x] Add a meaningful `description` to `bindings/sqlite3/project.json`,
      `bindings/libsnd/project.json`, `benchmark/mfb/project.json`.
- [x] Build each and confirm the warning from plan-61-D no longer fires.
- [x] Confirm section 18 is present in the resulting `.mfp` for a ~~signed~~ build.

Acceptance: **MET.** `bindings/sqlite3` and `benchmark/mfb/workers` both build
with no `PROJECT_JSON_DESCRIPTION_MISSING` line, and `bindings/sqlite3.mfp`'s
section 18 decodes to exactly
`SQLite3 binding package: LINK declarations and MFBASIC wrappers for the system
libsqlite3.` — read back by walking the section table directly, not inferred.

Two deviations, both recorded in §Corrections: the third package is
`benchmark/mfb/workers`, not `benchmark/mfb` (which is an *executable*); and the
build was unsigned, since signing needs a live registry session and section 18's
presence does not depend on it.
Commit: —

### Phase 2 — Migrate the fixture packages

- [x] Add descriptions to the 49 `tests/syntax` package manifests, in batches,
      each describing what the fixture tests.
- [x] Add descriptions to the 18 `tools/thread-package-sources`, 9
      `tools/security-package-sources`, and 2 `tools/link-package-sources`
      manifests.
- [x] Re-run the count and confirm zero `kind: "package"` manifests lack a
      description:
      `find . -name project.json -not -path './target/*' -not -path '*/packages/*' -exec grep -l '"kind"[[:space:]]*:[[:space:]]*"package"' {} + | while read f; do grep -q '"description"' "$f" || echo "MISSING $f"; done`
      → no output.
- [x] Verify no file was left syntactically invalid: every migrated manifest
      still parses.

Acceptance: **MET.** The `MISSING` sweep produces no output — across the whole
tree, zero `kind: "package"` manifests lack a `description` (3 from Phase 1 + 78
here = 81, matching §2's re-measured count exactly).

§3's "assert the mutation landed" requirement is satisfied programmatically, not
by eyeball: each of the 78 files is re-parsed with `json.load` and its
`description` compared **against the exact string intended for it**, with `kind`
re-checked as unchanged — 78 checked, 0 problems. That catches the BSD-`sed`
failure mode §3 warns about (an edit that appears to succeed while changing
nothing) because a no-op edit would read back as absent, not as the expected
text. `git diff --cached --stat` confirms 78 files, 78 insertions, 0 deletions.

Descriptions are meaningful, not placeholders: 65 were derived from each
fixture's own leading comment block (first complete sentence), and the remaining
13 — where the source opens with `IMPORT` lines rather than prose — were written
by hand from the fixture's exports and purpose.
Commit: —

### Phase 3 — Regenerate goldens (the churn, isolated)

> **Use `test-accept.sh` here, not `artifact-gate.sh`.** An earlier draft made
> the fast gate this phase's instrument and its acceptance criterion. It cannot
> see this change: `artifact-gate.sh` compares only `.ast`, `.ir`, `.hex`, and
> the native `nir/nplan/nobj/ncode/mir` set (`NATIVE_EXTS`,
> `scripts/artifact-gate.sh:19`). It **never** compares `.mfp`, `.info`,
> `.audit`, or `build.log`. Of the goldens this phase churns, exactly 2 (`.hex`)
> are in its denominator and 39 are not — so "`artifact-gate.sh` reports 0 diffs"
> would go green with every `.mfp`, `.info`, and `.audit` golden stale. That is
> the precise failure the Validation Plan already guards against by mandating the
> full suite; this phase now matches it.

- [x] Run `scripts/test-accept.sh target/debug/mfb target/accept-actual` (~15 min)
      and record which goldens diff. **Explain every diff before regenerating any
      of them.** Expected: `.mfp` goldens gain section 18; `.info`/`.audit`
      goldens may gain a description line; `build.log` goldens lose the
      missing-description warning. Anything else is unexplained and must be
      investigated, not accepted.
- [x] Seed with a filter, never bare: `scripts/sync-goldens.sh target/debug/mfb
      <name-glob>` per affected group. `sync-goldens.sh` **never creates** golden
      files — if a new golden kind is needed, pre-create it empty first.
- [x] **The 29 `tools/*-package-sources` manifests have no `golden/` of their
      own** — they are inputs consumed by fixtures elsewhere
      (`tests/rt-behavior/security/README.md`,
      `src/docs/spec/threading/12_validation.md`), so their churn surfaces in
      *other* fixtures' goldens and no `<name-glob>` names them directly. Find
      the dependent fixtures and seed those.
- [x] **Check the crafted-bytes fixtures before assuming a clean regen.**
      `tools/security-package-sources/mfp_craft.py` builds adversarial `.mfp`
      byte layouts at hand-computed offsets. Adding a manifest field to those 9
      packages may shift them. If a crafted fixture breaks, that is the crafting
      script needing an update — not a golden needing a re-baseline.
- [x] Re-run `scripts/test-accept.sh` and confirm zero failures.

Acceptance: **MET** — `acceptance tests passed (1069 test(s) ran)`, exit 0.

Every diff was explained **before** any regeneration, and the explanation was
checked mechanically rather than asserted:

1. **60 → 18 → 26 → 0.** The first run's 60 mismatches were plan-61-D's warning
   (see §Prerequisites). Migrating the manifests removed the cause and **59 of
   them self-healed with no golden touched at all**. The residual 10 `build.log`
   diffs were the warning *still firing* on the fixtures the census could not
   see (§Corrections); migrating those 10 healed them too.
2. **The 26 real diffs have a single cause.** Classifying every changed line
   across all 10 `.info` diffs yields exactly three fields — `package binary
   hash` (20), `content hash` (20), `binary representation length` (20) — and
   **0 unexplained lines**. That is precisely what a larger payload produces.
   The 16 `.mfp` diffs are the same cause in binary. Note the plan predicted
   `.info` goldens might gain a *description line*; they do not render one, they
   only rehash.
3. **Regeneration was filtered to the 16 named fixture directories**, never
   bare: `sync-goldens.sh` reported `synced 75 golden file(s) across 16 test(s)`
   and exactly **26** files changed — matching the 26 mismatches one-for-one, so
   nothing outside the explained set moved. Spot-decoding the regenerated
   `.mfp` goldens confirms section 18 now carries the exact description text
   written in Phase 2.
4. **Crafted-bytes fixtures: checked, and the script did not break.**
   `mfp_craft.py`'s hand-computed offsets still work — all 9 `generate.py`
   scripts ran clean, and the regenerated adversarial packages still pass
   (`syntax/security/*` → 10 tests passed). They *were* regenerated, because the
   9 source manifests changed and the committed bytes are meant to be
   reproducible from them; pkg-01's deterministic pinned `identKey` is unchanged,
   so no consumer manifest needed editing.
Commit: —

### Phase 4 — Flip to required (the behavioral change)

Last, so no intermediate commit leaves the tree red.

- [x] In `src/manifest/mod.rs`, promote the missing-`description` warning for
      `kind: "package"` to a hard error. **Reuse D's code (`2-200-0016`) and
      change its severity — do not allocate a second code.** It is the same
      condition with the same message; two codes for one condition would leave
      the warn code permanently unreachable and force every consumer to know
      both. Concretely: flip `severity` on that `Rule` in `src/rules/table.rs`,
      and flip the `warn` cell in its `01_rule-codes.md` row. Both, or
      `every_rule_is_documented_in_the_spec` (`src/rules/mod.rs:231-249`) is red.
- [x] Update the `01_rule-codes.md:248-255` prose a second time — the `warn`
      count drops back by one when `2-200-0016` becomes an error. D's Phase 1
      raised it; F lowers it. Leaving it stale is how it got stale before.
- [x] **Re-seed the `build.log` goldens a second time.** Phase 3 regenerated them
      to drop the missing-description warning; this flip changes the diagnostic's
      severity and therefore its rendered text wherever it still fires. Expect a
      second, smaller round of `build.log` churn and explain it before seeding.
- [x] Update the schema table row in
      `src/docs/spec/tooling/01_project-manifest.md`: `required` becomes
      `yes¹` with a footnote reading "required when `kind` is `package`;
      optional and ignored for `executable`" — mirroring the existing `kind`
      footnote idiom. The `kind` row is at `:33` and its footnote ¹ at `:53-56`
      (an earlier draft cited `:57`, which is a blank line).
- [x] Add a `tests/syntax/` fixture proving the new error: a `kind: "package"`
      project with no description fails to build with the expected diagnostic.
      Pre-create its `golden/build.log` empty, then seed it with a filtered
      `sync-goldens.sh`.
- [x] Add a fixture proving `kind: "executable"` without a description still
      builds cleanly.
- [x] Verify: `cargo build && cargo test --bin mfb spec`.

Acceptance: **MET.**
`tests/syntax/project/project-description-missing-error` fails with
`error[2-200-0016 PROJECT_JSON_DESCRIPTION_MISSING]` and `[exit 1]`;
`project-description-optional-executable` builds clean at `[exit 0]` with **no
diagnostic at all**, which is the contrast that matters — the field is inert on
an executable, not merely tolerated.

The severity flip reuses `2-200-0016` rather than allocating a second code, in
both `table.rs` and the spec row, and the prose `warn` count went nine → eight.
Runtime proof on a real package: deleting the `description` line from
`bindings/sqlite3/project.json` makes `mfb build` **exit 1** with that
diagnostic; restoring it returns **exit 0**.

**Phase 4's expected "second round of `build.log` churn" did not happen, and
should not have.** That task anticipated the diagnostic's rendered text changing
"wherever it still fires" — but after Phase 2 it fires nowhere, because every
package manifest now has a description. The only new `build.log` is the negative
fixture's own. Recorded rather than silently skipped.
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
- Acceptance: the full `scripts/test-accept.sh target/debug/mfb
  target/accept-actual` → 0 failures. This is the sub-plan where the ~15-minute
  suite is non-negotiable and the **only** valid gate: `artifact-gate.sh` does
  not compare `.mfp`, `.info`, `.audit`, or `build.log` at all (Phase 3), so of
  the 41 goldens this change churns it can see 2. Running it first as a cheap
  smoke check is fine; treating a green result as evidence is not.

## Open Decisions

- **Should `mfb init` emit a `description` stub for `kind: "package"`?**
  *Recommended:* yes, with an empty string and a comment-free placeholder that
  fails validation until edited — so a new package author hits the requirement at
  `init` time rather than at first build. Check whether `mfb init` templates are
  in scope before committing to this; if it turns out to touch a template system
  this plan has not surveyed, drop it and file it separately rather than
  expanding scope here.

## Corrections

- **The population is 91, not 81 — the plan's own measurement command hides 10
  manifests.** Every count in §2 (and in `plan-61-repo-web.md` §Measured
  populations) is produced by a `find` carrying `-not -path '*/packages/*'`.
  That filter is meant to skip *vendored* dependency directories, but it also
  excludes the entire `tests/syntax/packages/` fixture group — 10 `kind:
  "package"` manifests — because the path segment is spelled identically.

  Not caught by re-measuring, because re-running the plan's command faithfully
  reproduces its blind spot: the corrected count of 81 agreed with the plan and
  looked like confirmation. It surfaced only when Phase 3's acceptance run still
  reported 10 `build.log` mismatches whose diff was the description warning
  *still firing* — i.e. the artifact told me about files my census could not
  see. The 10 were migrated and the sweep re-run with **no path exclusions but
  `target/`**, which is the form that should have been used throughout.

  Scope impact: F's migration surface grows from 81 to 91 files. No other
  letter's scope derived from this number — A–E are all count-independent — so
  nothing else needs rescoping. The `tests/syntax` sub-count also changes from
  49 to 59.
- **`benchmark/mfb/project.json` is an executable; the package is
  `benchmark/mfb/workers/project.json`.** §2's distribution table lists
  `benchmark/mfb 1`, and Phase 1 names `benchmark/mfb/project.json` directly —
  but that manifest declares `kind: "executable"`. The distribution was
  generated by collapsing paths to two segments, so the real package one
  directory deeper was displayed under its parent's name. The count of 81 is
  unaffected.
- **Phase 1's acceptance asked for a *signed* build; the build was unsigned.**
  `--sign` requires a live registry session and an attestation, and section 18
  is emitted by the writer regardless of signing — signing covers the payload,
  it does not decide what goes in it. The section was verified present with the
  expected text in the unsigned artifact, which is the same evidence.
- **`bindings/libsnd/project.json` is being edited concurrently by another
  agent.** It carried an uncommitted `1.3.0` → `1.3.1` version bump. Committing
  the file wholesale would have swept up that agent's in-flight work. The
  description line was staged against the *committed* base and their bump was
  restored to the working tree afterwards, so the commit contains one added line
  and their edit is untouched and still uncommitted.
- **`bindings/sqlite3/sqlite3.mfp` is a tracked artifact and changed.** Adding a
  description to its manifest puts section 18 in the built package, so the
  checked-in `.mfp` legitimately churns. That is expected migration churn, not a
  golden needing investigation.
