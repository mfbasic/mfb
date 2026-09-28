# plan-149-C: Catalog, docs, benchmark and the gate

Last updated: 2026-09-22
Effort: medium (1h–2h), plus the final gate's wall-clock time
Depends on: plan-149-B (Prerequisites are in plan-149-A)

C makes the row visible: it gets a catalog entry (which also renders into `mfb man optimizations` and drives the `-v`
counts), the documentation is brought in sync, the self-update harness gains `LET cur` coverage, the 16 benchmark rows
are re-measured, and the full gate runs once.

**The single outcome:** `mfb build -v` reports `Place copy forwarding: <n>` for a program with a forwardable `LET`;
the 14 benchmark rows whose direct form is in place run at direct-form speed; and the full gate is green.

## Prerequisites

See plan-149-A. In addition: plan-149-B's phases are all ticked with their `Commit:` lines filled
(`grep -c '^- \[ \]' planning/plan-149-B-*.md` → 0).

## 1. Goal

- A catalog row exists at level 1, stage `NIR`, with its counter, and the man page table includes it.
- The harness proves the `LET cur` shape at every field site plan-145 covers, for a fixed sample of operations.
- The 14 in-scope benchmark rows (the 8 Fixed rows, plus the 6 Dynamic `removeAt` / `remove` / `removeKey` rows) reach
  the direct-form timing (probe order of magnitude: 1,443 → 7–10 ns for the record
  `set` row).
- The full suite, the artifact gate and acceptance are green, and every golden diff is a dropped copy.

### Non-goals

These are the same as plan-149-A's. The 2 Dynamic `set` rows (`test_lrd_set`, `test_lsd_set`) keep their rebuild; it
is **not** fixed here (see Open Decisions).

## 2. Current State

- The catalog lives in `src/optimizer/catalog.rs:rows()`. aggcopy's row is at `:236`. The man page embeds the table
  through `{{optimizer-catalog}}` (`src/docs/man/optimizations/package.md:52`), and `catalog.rs:601` asserts that the
  rendered table has `rows().len() + 2` lines, so adding a row updates the man page and its test together.
- The planning catalog is `planning/optimizations.md`: a table of rows. aggcopy's row is at line 269.
- The self-update harness is `tests/runtime/rt_inplace_self_update.rs`:
  - 15 field sites (`FIELD_SITES`: `S3`–`S7`, `S9`, `S10`, `T1`–`T8`; corrected from 19, see Corrections) and 79
    `cases.tsv` lines;
  - filtered by `MFB_SELF_UPDATE_SITES` and `MFB_SELF_UPDATE_FILTER` (`:1339`–`:1347`);
  - the unfiltered run took 3,009 s in plan-145's final gate.
- The benchmark build is `benchmark/runner.sh:bench_build_mfb`: `$MFB build <dir>` at the default level, `-O1`.
- The 16 rows, measured with the census in plan-149-A:

  | File | Rows |
  |---|---|
  | `list.mfb` | 1728 `test_lrf_set`, 2185 `test_lrf_removeAt`, 2452 `test_lrd_set`, 2909 `test_lrd_removeAt`, 3178 `test_lsf_set`, 3704 `test_lsf_removeAt`, 4013 `test_lsd_set`, 4539 `test_lsd_removeAt` |
  | `mapmatrix.mfb` | 547 `test_mrf_removeKey`, 731 `test_mrd_removeKey`, 930 `test_msf_removeKey`, 1141 `test_msd_removeKey` |
  | `setops.mfb` | 578 `test_srf_remove`, 827 `test_srd_remove`, 1082 `test_ssf_remove`, 1367 `test_ssd_remove` |

  The `*rf*` / `*sf*` rows are Fixed and the `*rd*` / `*sd*` rows are Dynamic.
- One full `benchmark/mfb` build and run (`--run 10`, `target/release/mfb` at `99b11f282`, the commit before A2):
  17.9 s to build (workers package + `-O1` benchmark) and 28.3 s to run (`/tmp/p149/bench.sh`, `wall.txt`). With the
  plan-149 compiler: 17.4 s and 30.8 s.

## Phases

> **NOTE — keep the checkboxes current as you go** (see plan-149-A). **An unticked box means NOT DONE.**

### Phase C1 — Catalog, stats and docs

- [x] Add a `Row` to `src/optimizer/catalog.rs` with name `"Place copy forwarding"`, level 1, stage `"NIR"`, and
      counter `&stats::PLACE_COPIES_FORWARDED`. Summary: "Removes a whole-block copy `LET b = <place>` of a local, a
      global, a field or a `STATE` field when every read of `b` comes before the next write of the place's owner, so an
      update written through `b` runs in place." Place it beside aggcopy's row.
- [x] Add a row to `planning/optimizations.md`, marked LANDED at Level 1: the module, the window rule W1–W5, and why
      it is Level 1 and not 0. Cross-reference aggcopy's row (line 269), and state that the two differ in their proof
      (a window versus never written).
- [x] Add one paragraph to `.ai/collections.md`, in the in-place section plan-145 updated: the `LET cur = rec.xs`
      shape reaches the seam through the Level-1 forward, with the windows it declines.
      Add the same fact to `src/docs/spec/memory/05_collections.md`, where plan-145 describes the direct form.
- [x] Update the pipeline comment in `src/optimizer/opt1/mod.rs` (the module doc lists the rows) to include the new
      row.

Acceptance: `cargo test --bin mfb optimizer::catalog` → pass; `target/debug/mfb man optimizations | grep -c 'Place copy'`
→ 1; building `/tmp/getprobe/a` with `-v` prints a `Place copy forwarding` line with a count of at least 1
(est. 5 min).
Verified 2026-09-27: `cargo test --bin mfb optimizer::catalog` → 1 passed; `grep -c` → 1;
`target/debug/mfb build -v /tmp/p149/a3` → `Place copy forwarding: 3` (`/tmp/getprobe/a`'s source no longer exists;
`/tmp/p149/a3` is the A3 probe with the record `set`, record `removeAt` and bare-local shapes). The spec gained a
bullet in `memory/05_collections.md`; `cargo test --bin mfb spec` → 44 passed.
Commit: d201225ce

### Phase C2 — `LET cur` coverage in the self-update harness

- [x] Add a via-`LET` variant to `rt_inplace_self_update.rs`'s field path. For a case, it rewrites the site's statement
      from `<owner> = … OP(<field>, …)` to `LET cur AS <T> = <field>` followed by the same statement with `cur` as the
      operand. It expects exactly what the direct form expects (`field_expect.tsv`), because after the forward the NIR
      is the direct form.
      Done: `FieldCase::via_let`, `field_statement_lines` (each update preceded by `LET cur<k> AS <T> = <field>`),
      `FieldSite::statement_with`, and the test `every_via_let_field_cell_meets_its_expectation`. One exception to
      "exactly what the direct form expects": at `S9` a `LAMBDA` body is one expression, so the `LET` sits outside the
      lambda, which then captures `cur<k>` — a captured binding is never forwarded, so `S9` expects
      `rebuild:via-let-captured-by-the-lambda` (Corrections).
- [x] Run the variant for a fixed sample, meaning these 5 `cases.tsv` signatures across all 15 field sites:
      - `collections::set` on a `List`;
      - `collections::removeAt`;
      - `collections::removeKey`;
      - `collections::remove` on a `Set`;
      - `collections::append` (the grow case, which takes the rebuild at not-last sites).

      That is 75 cells (5 × 15), each named by its `(signature, site)` label, so a failure points at one cell. The sample, not
      all 79 × 19, keeps the variant's unfiltered cost small. The window proof does not depend on the operation (the
      operation is plan-145's, already proven for every line), so 5 operations cover every owner class at every site.
- [x] Run the variant filtered to one site per owner class during development. The whole sample runs in CI's
      `test` job (`coverage.yml`); the harness is `#[ignore]`d in a plain `cargo test`.
      Done: `MFB_SELF_UPDATE_SITES=S4,S5,T2,S9 cargo test --release --test rt_inplace_self_update via_let -- --ignored`
      → 1 passed (20 cells, 91.8 s). The control — the same S4 cells with `MFB_TEST_EXE=/tmp/p149/mfb-before` (the
      pre-A2 compiler) — fails 5 of 5 with "marked `arm`, but 2000 more runs allocated 6000–16000 more blocks", so
      the variant measures the row. `coverage.yml` runs `cargo test --release --test rt_inplace_self_update --
      --ignored`, which includes the new test.

Acceptance: `cargo test --release --test rt_inplace_self_update via_let -- --ignored` → pass, 75 cells (est. 15 min
because each cell builds and runs a program; a smaller filter would not prove the site coverage this task exists for).
Verified 2026-09-27: 1 passed (the test asserts 75 work items when unfiltered), 127.2 s.
Commit: efa38f1d0

### Phase C3 — Benchmark re-measure

- [x] Measure one `benchmark/mfb` build and run with `target/release/mfb`, at the commit before plan-149-A2, and record
      the wall-clock time here (this replaces the UNMEASURED line in §2).
      Done: 17.9 s build + 28.3 s run (§2). The "before" compiler was built in a detached worktree at `99b11f282`
      (A1, no code change) and each run used a scratch copy of `benchmark/mfb` (`/tmp/p149/bench.sh`).
- [x] Record the 16 rows' results from that run. Then rebuild with the plan-149 compiler, run again, and record the 16
      rows again: a before / after table.
      Done (`python3 /tmp/p149/rows.py` over the two `table.log`s; the "after" compiler is `cfd7b7725` + C1's docs).
      All 363 benchmark checksums are identical before and after (`sums.log`).

      | Row | Function | In scope | Before (ms, median) | After (ms, median) | Speed-up | Checksum equal |
      |---|---|---|---|---|---|---|
      | `list (Record-Fixed)` `set` | `test_lrf_set` | yes | 2.861 | 0.008 | 357.6x | yes |
      | `list (Record-Fixed)` `removeAt` | `test_lrf_removeAt` | yes | 3.458 | 0.167 | 20.7x | yes |
      | `list (Record-Dynamic)` `set` | `test_lrd_set` | no (Non-goal) | 23.268 | 17.011 | 1.4x | yes |
      | `list (Record-Dynamic)` `removeAt` | `test_lrd_removeAt` | yes | 18.794 | 0.79 | 23.8x | yes |
      | `list (State-Fixed)` `set` | `test_lsf_set` | yes | 2.966 | 0.009 | 329.6x | yes |
      | `list (State-Fixed)` `removeAt` | `test_lsf_removeAt` | yes | 3.529 | 0.175 | 20.2x | yes |
      | `list (State-Dynamic)` `set` | `test_lsd_set` | no (Non-goal) | 22.787 | 17.105 | 1.3x | yes |
      | `list (State-Dynamic)` `removeAt` | `test_lsd_removeAt` | yes | 18.825 | 0.767 | 24.5x | yes |
      | `map (Record-Fixed)` `removeKey` | `test_mrf_removeKey` | yes | 2.849 | 0.991 | 2.9x | yes |
      | `map (Record-Dynamic)` `removeKey` | `test_mrd_removeKey` | yes | 5.054 | 1.76 | 2.9x | yes |
      | `map (State-Fixed)` `removeKey` | `test_msf_removeKey` | yes | 2.877 | 1.024 | 2.8x | yes |
      | `map (State-Dynamic)` `removeKey` | `test_msd_removeKey` | yes | 5.152 | 1.824 | 2.8x | yes |
      | `set (Record-Fixed)` `remove` | `test_srf_remove` | yes | 2.825 | 0.99 | 2.9x | yes |
      | `set (Record-Dynamic)` `remove` | `test_srd_remove` | yes | 4.49 | 1.228 | 3.7x | yes |
      | `set (State-Fixed)` `remove` | `test_ssf_remove` | yes | 2.827 | 1.0 | 2.8x | yes |
      | `set (State-Dynamic)` `remove` | `test_ssd_remove` | yes | 4.528 | 1.272 | 3.6x | yes |
- [x] If any in-scope row does not approach its direct-form timing, root-cause it on that row. Build the row alone with
      `--nir` and check whether the binding was forwarded. Fix the gate that declined it, or record with evidence why it
      must decline.
      Done. The binding is forwarded in all 16 rows: `mfb build --nir -O 1` on the benchmark leaves no
      `LET <name> = <member access>` bind in any of the 16 functions (`python3 /tmp/p149/nircur.py`), where the
      pre-A2 compiler leaves `cur` in every one. The 8 list rows clear 5× (20–358×). The 8 map / set rows are 2.8–3.7×
      and **are at their direct-form timing**. Row `test_mrf_removeKey` rebuilt as a probe (`/tmp/p149/mk`, 201 runs,
      median) times the `LET cur` loop at 1,034,000 ns and the direct-form loop at 1,034,000 ns with the plan-149
      compiler (before: 2,953,000 vs 1,057,000); the `Set` `remove` twin (`/tmp/p149/sk`) is 1,034,000 vs 1,035,000
      (before: 2,947,000 vs 1,060,000). Constructing the record is 2,000 ns, so the rest is the direct form's own
      `removeKey` / `remove`. That cost is specified: `mfb spec memory collections`, *Cost*: "`removeKey` and a `Set`'s
      `remove` cost the lookup plus the entries after the removed one, which slide down with their buckets renumbered".
      These rows remove keys `0..299` in insertion order, so every call slides every remaining entry (measured O(n) per
      call on a plain local too, `/tmp/p149/mk2`: 5.8 µs at n = 300, 27 µs at 1,200). Before the forward a call cost
      that slide plus an O(n) copy; after, only the slide. That bounds the ratio near 3× for this workload, whatever
      the forward does. Nothing declined, so no gate needs fixing (Corrections).

Acceptance: a before / after table for all 16 rows in this section. Every in-scope row (the 8 Fixed and the 6 Dynamic
remove rows) is at least 5× faster, or has a recorded root cause. The probe ratios were 137× to 206× for the record
`set` rows, 8× (`removeAt`) to 18× (`set`) for the `STATE` rows, and 18× to 35× for the `String`-element removes, so
5× is a floor that a working forward clears everywhere.
Verified 2026-09-27: the table above. 6 in-scope list rows ≥ 20×; 8 map / set rows 2.8–3.7× with the recorded root cause
(at direct-form timing, residual is the specified removal cost).
Commit: efa38f1d0 (C3 changes no code; its results were recorded with C2's commit)

## Validation Plan

- **Tests:**
  - unit tests in `placefwd.rs` (A2, B1, B2);
  - `tests/codegen/codegen_place_forward.rs` (A3, B3);
  - `tests/runtime/rt_place_forward.rs` (A3, B3), with output identical at `-O0` and `-O1`, including the negatives;
  - the harness `via_let` sample (C2).
- **Coverage check:** `codegen_place_forward.rs` asserts that `with_target` is present at `-O0` and absent at `-O1` for
  the same source. That proves the row is what changes the path, so a green result cannot come from a row that never
  fires.
- **Runtime proof:** the C3 benchmark table.
- **Doc sync:** C1 (the catalog and the man page, `planning/optimizations.md`, `.ai/collections.md`, the spec memory
  chapter).
- **Expected golden diffs:** only fixtures in the dirs that the two census commands list:

  ```
  grep -rlE '^\s*LET \w+ AS [A-Z][^=]*= [A-Za-z_]\w*(\.\w+)+\s*$' --include='*.mfb' tests examples src
  grep -rlE '^\s*LET \w+ AS (List|Set|Map|String)[^=]*= [a-z_]\w*\s*$' --include='*.mfb' tests examples src
  ```

  Together they list 30 dirs (2026-09-22). Each diff must be a dropped copy (fewer `copy_*` labels, one less owned
  slot). Regenerate only the fixtures that show one. A diff anywhere else is a bug: objdump that one fixture and fix it.
- **Final gate** (run ONCE, after C3, on the tree with main merged in if main has advanced):
  - `cargo test --no-fail-fast -- --skip artifact_gate_all > log; echo EXIT=$?` (`.ai/testing-gates.md`: the skipped
    test is the artifact gate, run separately below);
  - `cargo test --release --test rt_inplace_self_update -- --ignored`: the unfiltered harness plus the C2 sample. It
    is `#[ignore]`d, so the plain `cargo test` above does **not** include it (corrected; the earlier text said it did).
    The row can change what the harness measures (every program's `LET before = <field>` snapshot is a place copy),
    so it must run;
  - `scripts/artifact-gate.sh target/release/mfb all`;
  - `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

  Estimated at 90+ min, because `.ai/testing-gates.md` requires the full suite once, and nothing smaller covers the
  golden churn.

## Open Decisions

- **The variable-width list `set` rows.** `test_lrd_set` and `test_lsd_set` are slow even in the direct form
  (`dynDirect` 9,294 ns per statement, probe `/tmp/getprobe/d`): `lower_field_set`
  (`src/codegen/collection/assign/builder_inplace_assign.rs:1093`) declines a variable-width element `set` because a
  longer string grows the list block. Filed as bug-759 (`bugs/bug-759-field-varwidth-list-set-rebuilds-the-owner.md`), which also finds the decline's stated reason stale since bug-627. Recommendation: file it as its own bug with that probe as the reproduction. The
  likely fix is codegen, routing the `set` through `InlineGrow` the way `Map` `set` does, when the field is the owner's
  last inline sub-block (it is in `RecDyn`). Not an optimizer row.
- **Row name.** "Place copy forwarding" is recommended. "Field-read forwarding" is too narrow, because the row also
  forwards bare locals and globals.

## Corrections

- **Final-gate findings (all resolved).**
  - `rt_inplace_self_update -- --ignored` (unfiltered, 5,500 s): 13 of 1,230 pairs failed, all `io::Key` "build
    failed: TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE". Pre-existing on main: bug-699 Phase 4 (`7e554824c`) gave `io::Key`
    enum fields, so it has no default, and the pre-A2 compiler rejects `MUT x AS io::Key` identically.
    `field_kinds_gen.py` gained the `io::Key` literal (as `term::MouseEvent` has); regenerating changes exactly that
    TSV line; `MFB_SELF_UPDATE_FILTER='io::Key' … every_field_kind -- --ignored` → pass (`2159d14c0`).
  - `cargo test --bin mfb`: two plan-134-C `last_use` tests lowered their probes at the default level, where this
    row now rewrites them (`LET b = a; RETURN b.tag` → `RETURN a.tag`; `#json_parseArrayItems`' `item` forwarded,
    NIR diff exact). The hand-derived table now lowers at `-O0`, with every row unchanged. The decoder test keeps the
    default level and asserts the stronger fact: the append of `parsed.value` is that field's last read, so the
    element moves with no copy (`86677797e`).
  - `scripts/test-accept.sh target/debug/mfb target/accept-actual`: 1,570 of 1,571 matched; the one mismatch is
    `[1] acceptance`, a `timeout` (the 300 s `MFB_ACCEPT_RUN_TIMEOUT`) while the harness above loaded every core. The
    same project run directly passes 782 / 782 with the release compiler and with the debug one (`mfb test`, exit 0),
    and re-run through the harness alone and unloaded, `scripts/test-accept.sh target/debug/mfb target/accept-actual
    acceptance` → "acceptance tests passed (1 test(s) ran)". All 1,571 fixtures therefore match.
  - `cargo test --no-fail-fast -- --skip artifact_gate_all` (`EXIT=101`): 5 failures, every one a test whose
    *instrument* was a `LET b = <local or field>` the row now legitimately forwards. None is a behaviour change.
    - The 2 `last_use` unit tests (above, `86677797e`).
    - `rt_recursive_value_construction_copies::a_construction_store_copies_an_aliased_source_once_and_a_fresh_value_never`:
      `LET c = a` forwarded, so 2 graph copies instead of 3. The test now asserts 3 at `-O0` (copy insertion
      alone) and 2 at `-O1` (the forward's saving).
    - `rt_recursive_value_drop_symmetry::{churn_frees_every_copy_as_it_goes, a_million_deep_chain_drops_exactly_its_copy}`:
      the hook's `LET probe AS Node = cur` was forwarded, so the hook copied nothing. The probes build at `-O0`
      (`19c3a6504`).

    All five re-run green: `cargo test --bin mfb codegen::engine::analysis::last_use` → 6 passed;
    `cargo test --test rt_recursive_value_construction_copies --test rt_recursive_value_drop_symmetry` → 4 / 6
    passed. Every other test in the suite passed; the `mfb` unit binary reported 4,514 passed.
  - `scripts/artifact-gate.sh target/release/mfb all` after the regeneration: `2182 golden(s) checked, 0 diff(s)`.
  - **After merging main (`223145b13`, 16 commits: bug-688 / bug-758 / plan-162 / plan-163)**, the whole gate re-ran
    on the merged tree. The only conflicts were `syntax/app/app-mouse-surface` goldens, which both sides had
    regenerated; they were regenerated on the merged tree, and their `.nir` against main's differs in 11 functions, all
    exactly explained by forwarding. Results: `scripts/artifact-gate.sh target/release/mfb all` → `2182 golden(s)
    checked, 0 diff(s)`; `scripts/test-accept.sh target/debug/mfb target/accept-actual` → "acceptance tests passed
    (1571 test(s) ran)"; `cargo test --no-fail-fast -- --skip artifact_gate_all` → `EXIT=0`, 0 `failures:` blocks;
    `cargo test --release --test rt_inplace_self_update -- --ignored` → 3 passed, 4,561.7 s.
  - **A second main merge (`72c676092`, bug-756: 7 commits, all canvas text/font code plus the
    `app-mouse-surface` goldens)** landed during that gate. Conflicts again only in those goldens: regenerated (6
    rewritten, 0 failures), and `.nir` vs main's is 11 functions, all exactly explained by forwarding. Re-verified
    **scoped to bug-756's blast radius** instead of repeating the ~3 h suite (the skill's scoped-check rule: the
    rest of the tree is byte-identical to the tree the full gate just passed on): `scripts/artifact-gate.sh
    target/release/mfb all` → `2182 golden(s) checked, 0 diff(s)` (every golden, not only canvas); all 16
    `tests/canvas/rt_canvas_*` binaries → 246 passed, 0 failed; `scripts/test-accept.sh … 'app*' 'canvas*'
    '*canvas*' '*app*'` → "acceptance tests passed (33 test(s) ran)".

- **The golden-diff census looked in the wrong place (final gate).** The two `grep` commands in the Validation Plan
  scan fixture *sources*; the diffs came instead from the MFBASIC helper code of builtin packages, which is merged into
  every module that imports them and which the row also optimizes. The first full gate
  (`scripts/artifact-gate.sh target/release/mfb all`) reported `1545 tests, 2182 golden(s) checked, 60 diff(s)`
  across 12 fixtures, each on every one of its targets (a real codegen change, not a broken regen): `byte-identity/`
  `udp tls tcp strings regex net json http csv crypto`, `rt-behavior/crypto/crypto-ec-valid` and
  `syntax/app/app-mouse-surface`. Each was localized by NIR diff between the pre-A2 compiler and this one
  (`/tmp/p149/classify.sh` → `/tmp/p149/nirdiff.py`, which drops each vanished `LET <name> = <place>` bind and
  substitutes the place for its reads in the *before* body, then requires byte equality with the *after* body): 32
  changed functions, **32 exactly explained** by forwarding. Every one is a package helper, for example
  `#net_decodeQueryComponent` (`decoded`), `#json_get` (`currentValue`, then the chained `$match7`),
  `#crypto_openSealed` (`ct`, `tg`) and `#canvas_hashPaint` (`c`, `t`). No fixture's own source contributed. The 60
  goldens were regenerated with `scripts/regen-native-goldens.sh target/release/mfb <the 12 dirs>`
  (`60 golden(s) rewritten, 0 failure(s)`; the rewritten set equals the diff set).

- **C2's counts and command.** `FIELD_SITES` has 15 sites, not 19 (`grep -c 'FieldSite::[ST][0-9]*,' ` over the
  `FIELD_SITES` array → 15; `S8` and `T9`–`T12` do not exist), so the sample is 75 cells, not 95. The acceptance
  command `MFB_SELF_UPDATE_FILTER=via_let … -- --ignored` would have failed for the wrong reason: that variable
  filters `cases.tsv` signatures, and the existing grid test panics "matches no case" when nothing matches. The
  variant is selected by test name instead: `cargo test --release --test rt_inplace_self_update via_let -- --ignored`.
- **`S9` cannot expect the direct form (C2).** See the C2 task: the lambda must capture `cur<k>`, and a captured
  binding is never forwarded (plan-149-A §3), so `S9` expects the rebuild.

- **The 5× floor was miscalibrated for the map and set rows (C3).** It came from probes that removed from a
  `String`-element collection at arbitrary positions and from `set` / `removeAt` rows, where the copy the forward
  removes dominates. The benchmark's `removeKey` / `remove` rows delete in insertion order, so each call's own
  specified cost (sliding every later entry) is the same order as the copy it saved, and a perfect forward yields
  about 3×. Measured: those rows reach exactly the direct-form timing (C3), so the goal — "reach the direct-form
  timing" — holds, and the acceptance's "or has a recorded root cause" branch records why 5× is not reachable by any
  NIR rewrite. No codegen change (Non-goal).
- **The benchmark reuses `cur` (C3).** Reading the rows for C3 showed every row binds `cur` twice (in the loop and
  after it), which the function-wide "bound once" gate A2 wrote declined. Fixed before measuring, with scope-aware
  gates (plan-149-A Corrections, `cfd7b7725`).

## Summary

C carries little risk. The real exposure is the golden churn and the harness sample, and both are bounded:
- the churn by the census above (30 dirs);
- the sample by its 95 named cells.

Left untouched: the 2 variable-width list `set` rows' rebuild, aggcopy's Level-3 row, and any codegen.
