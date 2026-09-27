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
| plan-159-A complete | `ls planning/plan-159-A-*` → no match (it has moved to `planning/completed/`) | NOT MET |

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

**Expected byte-identical:** every `.ncode` and `.ncodesum`. A proved it on one pair
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

- [ ] 1.1 Resolve the exclusion list paths and write them into §2.
- [ ] 1.2 Rewrite `examples/`, `benchmark/`, `bugs/` (live `bugs/*.md` repro sources
      only), `spikes/`, and `tools/**/*.mfb`. Build each example once: `mfb build` on
      each of the 19 example projects → exit 0.
- [ ] 1.3 Rewrite `packages/**/*.mfb`, then rebuild every `.mfp` in
      `/tmp/p159-mfp-sub.txt` (leaves first). Re-run the §2 `pkg info` loop → 0 hits.
      Rebuild `repository/tests/fixtures/libsnd.mfp` if it is in the list.
- [ ] 1.4 Word the comment-prose lines the rewriter reported.

Acceptance: `rg` for SUB in these directories → 0 hits; the `.mfp` loop → 0; all examples build.
  Check: `rg -l "$R|\b(END|EXIT)\s+SUB\b" -g '*.mfb' examples packages tools bugs spikes benchmark` → empty (est. 5 s); the `.mfp` loop → 0 (est. 1 min).
Commit: —

### Phase 2 — `tests/**/*.mfb` and their goldens

- [ ] 2.1 Rewrite every `tests/**/*.mfb` except the exclusion list.
- [ ] 2.2 Localize before bulk regeneration: pick ONE fixture per golden category
      (`.ast`, `.ir`, `.nir`, `build.log`, `.info`), rebuild it, and confirm the diff
      touches only the predicted fields. Record the fixture names here.
- [ ] 2.3 Regenerate: `bash scripts/sync-goldens.sh target/release/mfb` for
      byte-identity + rt-behavior. Handle `syntax/**` by hand (`sync-goldens.sh` skips
      it; see `.ai/testing-gates.md`).
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

(none yet)

## Summary

The work is mechanical. The risk is a golden regeneration that launders a real diff,
and the predicted-field check in 2.2 plus byte-identical native goldens close that off.
