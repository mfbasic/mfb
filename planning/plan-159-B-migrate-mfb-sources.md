# plan-159-B: Migrate every `.mfb` source from `SUB` to `FUNC … AS Nothing`

Last updated: 2026-09-26
Effort: large (3h–1d) (guess: 257 files is mechanical; the golden regeneration and the `.mfp` rebuild order are what cost time)
Depends on: plan-159-A (the rewriter, and `FUNC main AS Nothing` as a valid entry)

Every `.mfb` file in the repository stops declaring `SUB`, with one exception: the
fixtures that pin SUB-only rules, which D retires. The compiler still accepts `SUB`
after B. It just has no users outside those fixtures. Codegen is unchanged, and the
proof of that is the `.ncode`/`.ncodesum` goldens staying byte-identical while the
`.ast`/`.ir`/`.nir`/`build.log` goldens move exactly as predicted.

References: plan-159-A (prerequisites, §2 measurements, the rewriter),
`tools/sub-to-func/README.md`, `.ai/testing-gates.md` (golden maintenance: `sync-goldens.sh`
skips `syntax/**` and target-infixed sums), `.ai/resources-packages.md` (package build/import).

## Prerequisites

The plan-wide prerequisites are in plan-159-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-159-A complete | `ls planning/plan-159-A-*` → no match (it has moved to `planning/completed/`) | MET (2026-09-27: A archived to `planning/completed/`) |

## 1. Goal

- `rg -l '^\s*(PUBLIC\s+|PRIVATE\s+|EXPORT\s+)?(ISOLATED\s+)?SUB\b|\b(END|EXIT)\s+SUB\b' -g '*.mfb' .`
  lists **only** the SUB-rule fixtures in the exclusion list (§2).
- The `.ncode` and `.ncodesum` goldens are byte-identical to the pre-B tree.

### Non-goals

- No compiler change. If B needs one, it is a correction to A.
- The SUB-rule fixtures stay as `SUB` (D converts or retires them).
- Prose in comments that mentions "SUB" is reworded only where it describes the code
  in that file (the rewriter reports these lines). Historical bug and plan docs under
  `bugs/completed/` and `planning/completed/` are **not** rewritten, because they
  record past state.

## 2. Current State

Population, measured with the full decl regex from A (`R='^\s*(PUBLIC\s+|PRIVATE\s+|EXPORT\s+)?(ISOLATED\s+)?SUB\b'`):

| Where | Files | Command |
|---|---|---|
| `tests/` | 197 | `rg -l "$R" -g '*.mfb' tests \| wc -l` (the agent census: 197 files / 293 decls) |
| `examples/` | 19 | `rg -l "$R" -g '*.mfb' examples \| wc -l` |
| `packages/` | 18 | same, `packages` |
| `tools/` | 18 | same, `tools` |
| `bugs/` | 5 | same, `bugs` |
| `spikes/` | 7 | same, `spikes` |
| `benchmark/` | 2 | same, `benchmark` |

**Goldens that move.** These are expected diffs, i.e. the plan working:

| Golden | Files | Command | Why |
|---|---|---|---|
| `.ast` `"kind": "sub"` | 141 | `rg -l '"kind": "sub"' tests -g '*.ast' \| wc -l` | kind → `func`, returns → `Nothing` |
| `.ast` `"target": "sub"` | 15 | `rg -l '"target": "sub"' tests -g '*.ast' \| wc -l` | `EXIT SUB` → `RETURN` node |
| `.ir` `"kind": "sub"` | 182 | `rg -l '"kind": "sub"' tests -g '*.ir' \| wc -l` | kind string |
| `.nir` `"kind": "sub"` | 14 | `rg -l '"kind": "sub"' tests -g '*.nir' \| wc -l` | kind string |
| `build.log` quoting `SUB` | 23 | `rg -l '\bSUB\b' tests -g '*.log' \| wc -l` | the quoted source line changes; columns can shift |
| `.info` (`pkg info`) | 2 | `package-shape/golden/package_shape.info`, `package-simple/golden/package_simple.info` | `SUB name` → `FUNC name` |
| `.mfp` with SUB exports | 38 of 161 tracked | `for f in $(git ls-files '*.mfp'); do mfb pkg info "$f" \| grep -q '\bSUB\b' && echo $f; done \| wc -l` → 38 (list saved at `/tmp/p159-mfp-sub.txt`) | export kind 2 → 1, `FUNCTION_FLAG_SUB` cleared, ABI hash of those exports changes |

**Expected byte-identical:** every `.ncode` and `.ncodesum` — **except** a fixture that
links a rebuilt SUB-exporting `.mfp`: its package functions' names and symbols carry the
package identity id (`package_identity_id`, a hash over the package payload), which a
SUB→FUNC export changes, so those `.ncode` move in that 16-hex prefix and nowhere else
(measured in plan-159-A 3.2; Corrections). A proved it on one pair
and on the three rewriter fixtures. Any `.ncodesum` diff is a bug trigger: `objdump`
ONE fixture, find the cause, fix the cause, and continue.

**Exclusion list (stay `SUB` until D).** These fixtures exist to pin SUB-only rules:

- `tests/syntax/functions/sub-value-less-invalid`
- `tests/syntax/functions/isolated_sub_invalid`
- the `exit-sub-invalid` fixture
- the `control-flow-sub-trap-invalid` fixture
- `func_isolated_requires_func_invalid`
- the one fixture whose golden says `must begin with FUNC or SUB`

Find their paths with
`rg -l 'SUB_RETURN_FORBIDDEN|EXIT_SUB_IN_FUNC|TYPE_SUB_HAS_NO_VALUE|ISOLATED is valid only|must begin with FUNC or SUB' tests -g '*.log'`.
Record the resolved list here in task 1.1.

**Resolved (task 1.1, 2026-09-27).** That `rg` returns 4 paths; `isolated_sub_invalid` pins its
rule through the top-level "Expected an IMPORT, LET, MUT, SUB, FUNC…" message, so it is found by
`find tests -type d -name 'isolated_sub*'`. The "must begin with FUNC or SUB" fixture IS
`func_isolated_requires_func_invalid` (its source has no `SUB`; only its golden message names
it), so the list is 5, not 6:

- `tests/syntax/functions/sub-value-less-invalid`
- `tests/syntax/functions/isolated_sub_invalid`
- `tests/syntax/control-flow/exit-sub-invalid`
- `tests/syntax/trap/control-flow-sub-trap-invalid`
- `tests/syntax/functions/func_isolated_requires_func_invalid` (nothing to rewrite)

## 3. Design Overview

Run the A rewriter over the tree one directory at a time, regenerate the goldens that
are predicted to move, and gate on byte-identical native goldens. The risk is
concentrated in two places:

1. **Golden regeneration hiding a real change.** The mitigation is the prediction
   table above. Before regenerating a golden category, diff ONE fixture of that
   category and confirm it moved only in the predicted fields.
2. **`.mfp` rebuild order.** Nested package copies (for example
   `tools/thread-package-sources/thread_import_worker/packages/thread_imp_print.mfp`)
   must be rebuilt from their rebuilt source package, leaves first. Each package
   directory's README or build script names its build command. Read it; do not guess.

Byte-identity IS this letter's gate for native code (a provably-neutral source rewrite).
For the textual goldens the gate is "moved only in the predicted fields".

## Phases

> Tick in the same commit as the work. **An unticked box means NOT DONE.**

### Phase 1 — Non-test sources (examples, packages, tools, bugs, spikes, benchmark)

- [x] 1.1 Resolve the exclusion list paths and write them into §2. — 5 paths (§2).
- [x] 1.2 Rewrite `examples/`, `benchmark/`, `bugs/` (live `bugs/*.md` repro sources
      only), `spikes/`, and `tools/**/*.mfb`. Build each example once: `mfb build` on
      each of the 19 example projects → exit 0. — `sub-to-func.py examples benchmark
      bugs spikes tools` → `53 file(s) changed; 0 unhandled`. The 19 were example
      *files*; they sit in 12 top-level examples = 15 buildable projects (Corrections).
      All build exit 0 in scratch copies: 9 single projects (`/tmp/p159/build-examples.sh`),
      `browser` dom→fetch/display→app per its README (`/tmp/p159/build-browser.sh`: 4 ×
      exit 0), `swat/{tui,canvas}` (ok), `audio` against a locally built rewritten
      `packages/libsnd` + its vendored dylib, `--unsigned` → `Wrote executable`.
- [x] 1.3 Rewrite `packages/**/*.mfb`, then rebuild every `.mfp` in
      `/tmp/p159-mfp-sub.txt` (leaves first). Re-run the §2 `pkg info` loop → 0 hits.
      Rebuild `repository/tests/fixtures/libsnd.mfp` if it is in the list. — The list
      was regenerated before any edit (`/tmp/p159/mfp-sub.sh` → `total=161 hits=38`).
      Rewrote `packages` plus the three in-tree package sources the list is built from
      (`tests/syntax/packages/{package-simple,package-import-as,package-shape}`), rebuilt
      `tools/thread-package-sources/{thread_imp_print, thread_import_worker/packages/
      thread_imp_print.mfp, thread_import_worker, thread_workers}` leaves first, then
      `scripts/sync-package-mfp.sh` → `updated 35`; second run → `updated 0` (fixpoint).
      Loop → `total=161 hits=0`. `libsnd.mfp` was not in the list. Three pre-existing
      problems found on the way were landed separately first (Corrections).
- [x] 1.4 Word the comment-prose lines the rewriter reported. — The only reports
      outside `tests/` were 12 lines of commented-out usage examples in
      `packages/zip/src/lib.mfb` (`' SUB main()` / `' END SUB`), now `' FUNC main() AS
      Nothing` / `' END FUNC`; re-run → 0 prose reports.

Acceptance: `rg` for SUB in these directories → 0 hits; the `.mfp` loop → 0; all examples build.
  Check: `rg -l "$R|\b(END|EXIT)\s+SUB\b" -g '*.mfb' examples packages tools bugs spikes benchmark` → empty (est. 5 s); the `.mfp` loop → 0 (est. 1 min).
  Result (2026-09-27): `rg` exit 1 (no matches); loop `total=161 hits=0`; 15/15 example projects build.
Commit: —

### Phase 2 — `tests/**/*.mfb` and their goldens

- [x] 2.1 Rewrite every `tests/**/*.mfb` except the exclusion list. — `sub-to-func.py
      tests` → `199 file(s) changed; 0 unhandled`; the 4 exclusion fixtures with SUB
      source restored to HEAD. Goal `rg` (§1) → only the 4 exclusion sources plus
      `func-bare-trap-rt` (comment prose, fixed in 2.5).
- [x] 2.2 Localize before bulk regeneration: pick ONE fixture per golden category
      (`.ast`, `.ir`, `.nir`, `build.log`, `.info`), rebuild it, and confirm the diff
      touches only the predicted fields. Record the fixture names here. — `overload-sub-valid`
      (`.ast`: `"kind": "sub"`→`"func"`, `"returnType": null`→`"Nothing"`; `.ir`: kind only),
      `project-entry-param` (`.nir`/`.ir`: kind only), `types-default-value-invalid`
      (`build.log`: the quoted header line only, carets unchanged), `package-simple`
      (`.info`: `SUB mark$…`→`FUNC mark$…`, their `sigHash`, the package hashes/length).
- [x] 2.3 Regenerate: `bash scripts/sync-goldens.sh target/release/mfb` for
      byte-identity + rt-behavior. Handle `syntax/**` by hand (`sync-goldens.sh` skips
      it; see `.ai/testing-gates.md`). — Done WITHOUT `sync-goldens.sh`, which copies
      every actual blindly (Corrections): full `test-accept.sh` → `325 mismatch(es) (1570
      test(s) ran)`; every mismatch classified line-by-line by `/tmp/p159/classify.py`
      against the predicted rewrites (kind/returnType, `EXIT SUB` `exit` node → `return`
      node, DOC header `SUB`→`FUNC`, quoted source lines, `pkg info` kind + hashes, the
      reworded `"after SUB "` output) → 321 predicted and copied; 1 reviewed by hand
      (`while-wend-removed-invalid`: the caret under `SUB`/`FUNC` widens `^^^`→`^^^^`);
      3 NOT caused by B — they fail identically on a detached plan-base build
      (`/tmp/p159-base`, `3 mismatch(es) (3 test(s) ran)`) and are left untouched here
      (Corrections). Covers `syntax/**` too.
- [ ] 2.4 Confirm that no `.ncode`/`.ncodesum` golden changed:
      `git diff --stat -- '*.ncode' '*.ncodesum'` → empty.
- [ ] 2.5 Fixtures whose *comment* documents SUB semantics (for example
      `functions/overload-sub-valid`, `functions/user-function-stack-args-valid`):
      reword the comment. Where a fixture's whole point is "SUB overloading", rename
      the fixture directory only if D would otherwise leave a misleading name. Record
      each rename here, and update its goldens' project name in the same commit.

Acceptance: only the exclusion list still contains `SUB`; the native goldens are byte-identical.
  Check: `bash scripts/artifact-gate.sh target/release/mfb all` → 0 diffs (est. 20 min; this is the only instrument that checks every `.ncodesum` across 5 targets, and B touches ~200 fixtures, so no smaller check covers them). Do not run it concurrently with another gate (`pgrep -f artifact-gate` first).
Commit: —

## Validation Plan

- Runtime proof: `bash scripts/test-accept.sh target/release/mfb /tmp/p159-acc` over
  `tests/rt-behavior` → all PASS (est. 15 min). The rt fixtures execute, so this
  proves behavior, not just bytes.
- Final gate: at the end of D.

## Open Decisions

- Fixtures named after `SUB` (`overload-sub-valid`, `sub-template-valid`): rename now
  (recommended; D would otherwise delete-and-recreate them) or leave them for D.

## Corrections

- **2026-09-27 — rebuilt packages move their consumers' symbol prefix (from plan-159-A
  3.2).** §1 Goal's "`.ncode`/`.ncodesum` byte-identical" does not hold for a fixture
  that links a SUB-exporting `.mfp`: the package identity id in every package symbol
  hashes the package payload, and the export kind/flag/`sigHash` in that payload change.
  A measured it: a consumer of `package_simple.mfp` rebuilt from the rewritten source
  differs from the original only in `f5a6da2ca244f894` → `14f370a06b0d4625`. The gate
  for those fixtures is therefore "identical once the identity prefix is normalized",
  checked by diffing one such fixture's `.ncode`; every other `.ncode`/`.ncodesum` must
  stay byte-identical.
- **2026-09-27 — 3 acceptance fixtures fail at the plan base (found in task 2.3).**
  `regex/replace-empty-pattern-rt` and `strings/strings-inline-trap-fallible-rt` lost their
  `TYPE_INLINE_TRAP_DEAD_HANDLER` advisory, and `collections/func_collections_helpers_invalid`
  prints `collections::sort` where its golden says `__collections_sort`. None has a source
  change in B, and all three fail the same way with a compiler built at `4fe38df41`
  (`git worktree add --detach /tmp/p159-base 4fe38df41`). Being root-caused and fixed
  separately; their goldens are not touched by B.
- **2026-09-27 — `sync-goldens.sh` was not used (task 2.3).** It copies every actual over
  its golden, so it would have laundered the 3 pre-existing failures above into the tree.
  The classifier copies only files whose every changed line is a predicted rewrite.
- **2026-09-27 — "19 example projects" were 19 files (task 1.2).** `git diff --name-only
  -- examples | awk -F/ '{print $1"/"$2}' | sort -u` → 12 examples; `browser` holds 4
  projects and `swat` 2, so 15 builds. `browser` must be built as its README's package
  chain, and `audio` imports the registry package `libsnd` (its `packages/` is
  git-ignored and empty even in the main checkout), so it was built against the in-tree
  `packages/libsnd` source with `--unsigned` and the vendored dylib placed as the
  diagnostic asks.
- **2026-09-27 — three package sources did not build at all (found in task 1.3; fixed
  as `27217cbc2`).** `state_xfer_workers`, `union_xfer_workers` and
  `union_xfer_stateless_workers` fail `EXPORT_NAMES_NON_EXPORTED_TYPE` (bug-624 B,
  `6a6c68484`); their consumers passed only on pre-rule `.mfp` copies through a
  same-name coincidence. Types exported, consumers name them, behaviour byte-identical.
- **2026-09-27 — 18 `tools/` package `.mfp` were stale too (resynced as `7481a59a2`).**
  `sync-package-mfp.sh` never refreshes the source-dir `.mfp`, only `tests/` copies;
  none has a direct reader except one already in sync.
- **2026-09-27 — the tree's `.mfp` copies were stale before B.** `sync-package-mfp.sh`
  on the unmodified tree updated 31 copies of 4 packages that export no SUB (landed
  alone as `a76df3d54`, with evidence). B's rebuild diff therefore starts from an
  in-sync tree.

## Summary

The work is mechanical. The risk is a golden regeneration that launders a real diff,
and the predicted-field check in 2.2 plus byte-identical native goldens close that off.
