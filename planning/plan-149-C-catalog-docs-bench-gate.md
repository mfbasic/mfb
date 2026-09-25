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
  - 19 field sites (`FIELD_SITES`, `:708`) and 79 `cases.tsv` lines;
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
- UNMEASURED: the wall-clock time of one full `benchmark/mfb` build and run. Task C3 measures it first.

## Phases

> **NOTE — keep the checkboxes current as you go** (see plan-149-A). **An unticked box means NOT DONE.**

### Phase C1 — Catalog, stats and docs

- [ ] Add a `Row` to `src/optimizer/catalog.rs` with name `"Place copy forwarding"`, level 1, stage `"NIR"`, and
      counter `&stats::PLACE_COPIES_FORWARDED`. Summary: "Removes a whole-block copy `LET b = <place>` of a local, a
      global, a field or a `STATE` field when every read of `b` comes before the next write of the place's owner, so an
      update written through `b` runs in place." Place it beside aggcopy's row.
- [ ] Add a row to `planning/optimizations.md`, marked LANDED at Level 1: the module, the window rule W1–W5, and why
      it is Level 1 and not 0. Cross-reference aggcopy's row (line 269), and state that the two differ in their proof
      (a window versus never written).
- [ ] Add one paragraph to `.ai/collections.md`, in the in-place section plan-145 updated: the `LET cur = rec.xs`
      shape reaches the seam through the Level-1 forward, with the windows it declines.
      Add the same fact to `src/docs/spec/memory/05_collections.md`, where plan-145 describes the direct form.
- [ ] Update the pipeline comment in `src/optimizer/opt1/mod.rs` (the module doc lists the rows) to include the new
      row.

Acceptance: `cargo test --lib optimizer::catalog` → pass; `target/debug/mfb man optimizations | grep -c 'Place copy'`
→ 1; building `/tmp/getprobe/a` with `-v` prints a `Place copy forwarding` line with a count of at least 1
(est. 5 min).
Commit: —

### Phase C2 — `LET cur` coverage in the self-update harness

- [ ] Add a via-`LET` variant to `rt_inplace_self_update.rs`'s field path. For a case, it rewrites the site's statement
      from `<owner> = … OP(<field>, …)` to `LET cur AS <T> = <field>` followed by the same statement with `cur` as the
      operand. It expects exactly what the direct form expects (`field_expect.tsv`), because after the forward the NIR
      is the direct form.
- [ ] Run the variant for a fixed sample, meaning these 5 `cases.tsv` signatures across all 19 field sites:
      - `collections::set` on a `List`;
      - `collections::removeAt`;
      - `collections::removeKey`;
      - `collections::remove` on a `Set`;
      - `collections::append` (the grow case, which takes the rebuild at not-last sites).

      That is 95 cells, each named by its `(signature, site)` label, so a failure points at one cell. The sample, not
      all 79 × 19, keeps the variant's unfiltered cost small. The window proof does not depend on the operation (the
      operation is plan-145's, already proven for every line), so 5 operations cover every owner class at every site.
- [ ] Run the variant filtered to one site per owner class during development. The whole sample runs in the final
      gate as part of `cargo test`.

Acceptance: `MFB_SELF_UPDATE_FILTER=via_let cargo test --test rt_inplace_self_update` → pass, 95 cells (est. 15 min
because each cell builds and runs a program; a smaller filter would not prove the site coverage this task exists for).
Commit: —

### Phase C3 — Benchmark re-measure

- [ ] Measure one `benchmark/mfb` build and run with `target/release/mfb`, at the commit before plan-149-A2, and record
      the wall-clock time here (this replaces the UNMEASURED line in §2).
- [ ] Record the 16 rows' results from that run. Then rebuild with the plan-149 compiler, run again, and record the 16
      rows again: a before / after table.
- [ ] If any in-scope row does not approach its direct-form timing, root-cause it on that row. Build the row alone with
      `--nir` and check whether the binding was forwarded. Fix the gate that declined it, or record with evidence why it
      must decline.

Acceptance: a before / after table for all 16 rows in this section. Every in-scope row (the 8 Fixed and the 6 Dynamic
remove rows) is at least 5× faster, or has a recorded root cause. The probe ratios were 137× to 206× for the record
`set` rows, 8× (`removeAt`) to 18× (`set`) for the `STATE` rows, and 18× to 35× for the `String`-element removes, so
5× is a floor that a working forward clears everywhere.
Commit: —

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
  - `cargo test --no-fail-fast`, which includes the unfiltered harness: 3,009 s alone in plan-145, plus the C2 sample;
  - `scripts/artifact-gate.sh target/release/mfb all`;
  - `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

  Estimated at 90+ min, because `.ai/testing-gates.md` requires the full suite once, and nothing smaller covers the
  golden churn.

## Open Decisions

- **The variable-width list `set` rows.** `test_lrd_set` and `test_lsd_set` are slow even in the direct form
  (`dynDirect` 9,294 ns per statement, probe `/tmp/getprobe/d`): `lower_field_set`
  (`src/codegen/collection/assign/builder_inplace_assign.rs:1093`) declines a variable-width element `set` because a
  longer string grows the list block. Recommendation: file it as its own bug with that probe as the reproduction. The
  likely fix is codegen, routing the `set` through `InlineGrow` the way `Map` `set` does, when the field is the owner's
  last inline sub-block (it is in `RecDyn`). Not an optimizer row.
- **Row name.** "Place copy forwarding" is recommended. "Field-read forwarding" is too narrow, because the row also
  forwards bare locals and globals.

## Corrections

## Summary

C carries little risk. The real exposure is the golden churn and the harness sample, and both are bounded:
- the churn by the census above (30 dirs);
- the sample by its 95 named cells.

Left untouched: the 2 variable-width list `set` rows' rebuild, aggcopy's Level-3 row, and any codegen.
