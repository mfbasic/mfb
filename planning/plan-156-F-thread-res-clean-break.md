# plan-156-F: the clean break — `RES` only, corpus migration, spec and man

Last updated: 2026-09-24
Effort: large (3h–1d)
Depends on: plan-156-E

C–E gave threads resource semantics while still accepting `LET`/`MUT`. F
makes `RES` the only spelling:
- A `LET`/`MUT` thread binding is `TYPE_RESOURCE_REQUIRES_RES`, the rule
  every other resource already gets.
- Every thread program in the repository is migrated.
- The spec and `mfb man` describe the new model.
- The full suite runs once.
- bug-691/692/693 are archived as FIXED.

**Checkable outcome:**
- `rg` for a `LET`/`MUT` thread binding in `.mfb` sources, Rust test strings,
  the spec and the man sources returns 0.
- `LET t = thread::start(...)` is rejected with `TYPE_RESOURCE_REQUIRES_RES`.
- The full suite is green.

References: plan-156-A (settled semantics); `.ai/specifications.md` and
`.ai/spec-content.md` (spec rules); `.ai/man-content.md` (man rules, including
the memory-vocabulary ban); `.ai/testing-gates.md` (golden regeneration).

## Prerequisites

See plan-156-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-156-E complete | `ls planning/plan-156-E-*` → no match | MET (2026-09-25: "no matches found"; archived) |

## 1. Goal

- No thread is bound with `LET`/`MUT` anywhere, and the compiler rejects it.
- The spec (§4.8, §5, §14.1/§14.3/§14.9, §15, §15.6, §16, §19,
  `threading/*`, diagnostics) and the `thread` man pages describe exactly the
  plan-156 model.

### Non-goals

- No semantic change beyond the rejection. A–E did the semantics.
- `mfb fmt` is not taught to migrate: it is lexical by design (`src/fmt.rs`
  header).

## 2. Current State — measured populations (census 2026-09-24)

| What | Count | Command |
|---|---|---|
| `.mfb` thread bindings | 159 in 93 files (154 `LET` typed, 3 `LET` untyped, 1 `MUT`, 1 `= makeThread()`) | `/tmp/thrcensus/alias.py` over `rg -l --glob '*.mfb' -e 'thread::\|Thread OF\|ThreadWorker OF' .` (146 files) |
| — by area | rt-behavior 56 files, syntax 24, byte-identity 2 (`thread`, `resource-xfer-slots`), rt-error 3, examples 3 (`browser/app`, `network-server`, `wind`), benchmark 1, spikes 3, tools 1, packages 0 | same |
| Regex-migratable | 153/153 in-scope bindings in 89 files (`^(\s*)(LET\|MUT)\s+(\w+)(\s+AS\s+Thread OF[^=]*)?\s*=\s*thread::start`) | `/tmp/thrcensus/migrate.py`, built before/after with `buildall.sh` |
| Manual `.mfb` edits | 1 `MUT` reassignment (`rt-behavior/threads/thread-drop-cleanup/src/main.mfb:20`), 1 `LET = makeThread()` (same file:44), 3 nested temporaries (`bug630-nested-thread-handle-rt:33,49,53`), 2 record fields (`syntax/threads/func_thread_{start,send}_invalid`) | `classify.py`, `uses.py` |
| Rust test strings | 50 binding lines in 16 files (`rg -c 'thread::start' tests --glob '*.rs'` → 64 occurrences in 16); manual: `B622_ALIAS`, `B622_REASSIGNED`, 2 split-line bindings (e.g. `net/rt_tls_listener_thread_transfer.rs:139`) | census |
| Rust `src/` strings | 27 lines in 17 files: 12 in 10 thread `func_*.rs` man examples, `os/func_sleep.rs:76`, unit tests `builtins/tests/{thread_send_scalar 4, threads 3}`, `ir/shape.rs` 3, `audit/collect/source.rs` 2, `ir/tests.rs` 1 (`thread::Thread OF` spelling), `fmt.rs` 1 | `rg -n -e '(LET\|MUT)\s+\w+\s+AS\s+(thread::)?Thread OF' -e '(LET\|MUT)\s+\w+\s*=\s*thread::start' src --glob '*.rs'` |
| Spec files mentioning threads | 37 files, 134 lines (largest: `language/16_threads.md` 28, `architecture/21_type-name-encoding.md` 14, `threading/08_queue-semantics.md` 13) | `rg -c -e 'Thread OF\|ThreadWorker\|[Tt]hread handle\|`Thread`\|thread::start' src/docs/spec` |
| Spec/man binding examples | spec 1 (`16_threads.md:12`) plus `tooling/05_fmt.md:116`; man 5 (`tour/0{1..5}_*.md`) | the binding `rg` above over `src/docs` |
| Man pages | 12 `thread` function pages + `MODULE_DESC`; plus `os::sleep`, `types/package.md`, `tour/*`, `variable/package.md` | `mfb man thread`; `rg -c -i thread src/docs/man` |
| `.ast` goldens that move | 71 hold a `thread.start` binding (a binding gains `"resource": true`) | `rg -l '"kind": "binding".*"callee": "thread.start"' tests --glob '*.ast'` |
| Syntax `build.log` quoting a binding | 13 files, 82 lines | census |
| Byte-identity sets | `tests/byte-identity/{thread,resource-xfer-slots}`: `.ast`, `.ir`, `build.log`, 5 `.ncodesum` each | `find tests/byte-identity/{thread,resource-xfer-slots} -type f` |
| Worker `.mfp` copies | 90, built from 29 `tools/thread-package-sources` projects | `find tests examples -name '*.mfp'` filtered by those names |

## 3. Design

- **The rejection.** In the `ops.rs` Bind arm, remove C's temporary
  `LET`/`MUT` → resource mapping. A `ThreadHandle` binding without `RES` is
  `TYPE_RESOURCE_REQUIRES_RES` (existing rule).
- **Migration.** Save `/tmp/thrcensus/migrate.py`'s regex as a one-off script
  in this plan's working notes. Per AGENTS.md, a one-off migration never goes
  in `scripts/`. Apply it to the in-scope `.mfb` files, then the manual list
  in §2. Rust strings are edited by hand: the list is in §2.
- **The manual cases.**
  - `thread-drop-cleanup`'s `MUT` reassignment becomes two `RES` bindings in
    two scopes. Its case name stays, and the tested drop order is unchanged.
  - `makeThread`'s return becomes `RES t = makeThread()`.
  - The nested temporaries need no binding change.
  - The record fields become `t AS RES Thread…` (D's rule).
- **Goldens.**
  - `bash scripts/sync-goldens.sh target/release/mfb 'rt-behavior/**' 'rt-error/**' 'byte-identity/**'`
    for the acceptance goldens.
  - Syntax goldens are regenerated by hand, since `sync-goldens` skips
    `syntax/**`.
  - `.ncodesum`: `mfb build -q -ncode [-target T] <fixture>` + `shasum -a 256`
    for each of the 5 targets.
  - `.mfp`: `scripts/sync-package-mfp.sh`.
- **Spec.** One pass over the 37 files, per `.ai/spec-content.md`: as-is at
  HEAD, cited `[[path:Symbol]]`.
  - §4.8: thread types are resources, with two views.
  - §5: `RES` binding; the spec gap is that `statement` lists no RES binding
    in `19_grammar.md`.
  - §14.1/§14.3: remove the thread move-on-pass.
  - §15: thread rows (hidden drop op, no user close, repeatable `waitFor`,
    worker close at return).
  - §15.6: pools.
  - §16: rewritten rules and drop paragraph.
  - `threading/*`: already updated for layout in A/B.
  - Diagnostics: D added the codes.
- **Man.** The 12 thread pages' prose and examples; `MODULE_DESC`; tour,
  types, variable; the `os::sleep` example. Per `.ai/man-content.md`: no
  memory vocabulary, and say what the developer observes ("the thread keeps
  running until …"). Check with `scripts/man-census.sh --memory-scope` → 0
  unclassified, and run every example with
  `scripts/man-run-examples.sh thread --run`.
- **Archive the bugs.** bug-691/692/693 get a `STATUS: FIXED` block (the
  commit of the phase that fixed each: C, D, E) and move to `bugs/completed/`.

## Phases

> Keep checkboxes current (see plan-156-A's note). **An unticked box means NOT
> DONE.**

### Phase 1 — the rejection, plus the migration of everything it breaks

One phase: the rejection and the migration land together, or the tree does
not build.

- [x] The `ops.rs` rejection (§3).
      (C's `lenient_thread` removed; C's unit test
      `a_let_thread_binding_is_accepted_until_the_clean_break` — whose name
      promised this — became `a_let_thread_binding_requires_res`.)
- [x] Regex migration of the `.mfb` bindings, then the manual list (§2),
      including `examples/{browser/app,network-server,wind}`, `benchmark/mfb`,
      `spikes/*` and `tools/thread-package-sources`.
      (`/tmp/thrcensus/migrate.py`: tests 148 in 88 files, examples 4 in 3,
      benchmark 4 in 1, spikes 3 in 3, tools 2 in 1 — 161, the plan's 153 plus
      the thread fixtures A–E added. Residuals, as predicted: only
      `thread-drop-cleanup`. Its `MUT` reassignment could not become `RES` — a
      `RES` binding is not reassignable for any resource
      (`TYPE_ASSIGN_REQUIRES_MUT`, measured with `fs::File`) — so the old thread
      lives in an inner `IF TRUE` scope and drops at its end before the new one
      is bound; `makeThread()`'s result is `RES`; the run output is unchanged.
      A compile sweep over every thread-using fixture then found no other
      `TYPE_RESOURCE_REQUIRES_RES` and no build-status change. Built:
      `examples/{network-server,wind,browser/app}`, `benchmark/mfb` (with its
      worker package), the three spikes.)
- [x] Rust test strings and `src/` strings (§2 lists).
      (`tests/**/*.rs`: 58 in 19 files. Manual: `B622_REASSIGNED` binds a `RES`
      thread per pass plus one before the loop (same totals); `B622_ALIAS`'s
      second name is a `RES` move, `t2` waited on twice. `src/**/*.rs`: 28 in
      the plan's files — the man examples, the builtin unit tests, `ir/shape.rs`,
      `ir/tests.rs`, `audit`, my `debug/tests.rs` — plus `fmt.rs`'s casing test,
      whose input became `res t AS Thread …`. `rg` over both → 0. Unit tests
      touching them → 1059 passed.)
- [x] Goldens per §3.
      (Before regenerating, every golden diff of the 88 migrated fixtures was
      classified by `/tmp/p156_goldiff.py`: outside `thread-drop-cleanup` (hand
      edited; run output identical) each diff line is the `LET`/`MUT`→`RES`
      keyword or a binding gaining `"resource": true, "state": null` → 0
      unexpected. Then `sync-goldens.sh … 'rt-behavior/**' 'rt-error/**'
      'byte-identity/**'` (every changed golden belongs to a migrated fixture),
      the 24 syntax fixtures by hand from the classified actuals, and
      `sync-package-mfp.sh` → 5 copies updated — Correction 2. The
      `resource-xfer-slots` `.ncodesum` moved on all five targets only because its
      package copy did (with the old copy the sum matches the golden), and was
      regenerated.)
- [x] A new syntax fixture, `tests/syntax/threads/let_thread_binding_invalid/`,
      expects `TYPE_RESOURCE_REQUIRES_RES`.
      (Three: a typed `LET`, an inferred `LET`, a `MUT`; its `RES` line is clean.)

Acceptance: the binding `rg` over `.mfb` and `tests/**/*.rs` returns 0; the
new fixture passes.
  Check: `rg -n --glob '*.mfb' -e '^\s*(LET|MUT)\s+\w+(\s+AS\s+Thread OF[^=]*)?\s*=\s*thread::start' .`
  → no output (est. 5 s); `bash scripts/test-accept.sh target/release/mfb $(mktemp -d) 'rt-behavior/thread*/**' 'rt-error/threads/**' 'byte-identity/thread' 'byte-identity/resource-xfer-slots'`
  → 0 failures (est. 6 min).
  Result: the `.mfb` `rg` prints only the three deliberate lines of
  `let_thread_binding_invalid`; the Rust `rg` → none. The check glob plus
  `syntax/threads/**` → "acceptance tests passed (106 test(s) ran)";
  `artifact-gate all` → 0 diffs after the package regeneration; the Rust thread
  suites pass (`no_type_strings` failed on a bug-695 ratchet — Correction 3).
Commit: a6686c7e8

### Phase 2 — spec and man

- [x] The spec pass (§3), 37 files.
      (C, D and E each rewrote the sentences they falsified — C and D
      Correction 4 — so this pass audited the rest by grepping the 37 files for
      the retired claims and rewrote: `language/16` (the example binding; the
      sendable list; a new closing section — thread handles are resources,
      `RES`-only, not reassigned, drop cancels or joins; storage; per-view
      `STATE`), `language/15` (thread handles in the four-event rule; pools in
      §15.6), `language/04` §4.8 (the `STATE` form, `RES`), `language/05` (the
      `RES` list), `language/19` (a `resStmt` — the gap §3 names), `architecture/21`
      (a thread's trailing `STATE` is the handle's), `threading/spec.md`,
      `02_isolation` (start/send copy through the block arena and never consume;
      the "no arena but the main one is torn down" sentence was false since A),
      `06` (the entry's `x0` is the worker view), `08` (the drop: owner count,
      move-on-pass and `MUT` reassignment removed), `11` (`waitFor` closes
      nothing), `tooling/05_fmt` (its example). `language/14`, `threading/09` and
      the linker/app/stdlib mentions were already accurate. Diagnostics: the
      rule row landed in D.)
- [x] The man pass (§3).
      (The 12 thread pages' examples were migrated in Phase 1 and their prose
      in C and D; this pass added to the package page a paragraph on the `RES`
      handle — bound with `RES`, closed and stopped at scope end, lent to
      functions, kept in a pool — and one on per-end `STATE`; `types` (a `Thread`
      is a `RES` handle that can be transferred and stored; a `ThreadWorker`
      never leaves its worker); `variable` (a `Thread` is a RES handle); the five
      `tour` snippets (`RES t = thread::start(…)`).)

Acceptance: the spec gates pass and every thread man example runs.
  Check: `cargo build && cargo test --bin mfb spec` → pass (est. 4 min);
  `scripts/spec-census.sh --citations` → no stale-by-deletion citations to
  thread symbols deleted in C (est. 1 min);
  `scripts/man-census.sh --memory-scope` → 0 unclassified (est. 1 min);
  `scripts/man-run-examples.sh thread --run` → all pass (est. 2 min).
  Result: `cargo test --release --bin mfb -- spec man registry` → 449 passed;
  `scripts/spec-census.sh --citations` → 0 misses (0 stale-by-deletion),
  `--links` → 971 links, 0 unresolved; `man-census.sh --memory-scope` → 0
  unclassified; `man-run-examples.sh thread --run` → 13 built, 13 ran, 0 failed
  (and `os --run` → 27/27, for the migrated `os::sleep` example).
Commit: 80a0822b2

### Phase 3 — close out

- [x] bug-691/692/693: `STATUS: FIXED` blocks, then move them to
      `bugs/completed/`.
      (bug-691 → `a9253dbb5` (C), bug-692 → `f1e2ba176` (D; its block records
      that the RES field's copy aliases, per D's Correction 1), bug-693 →
      `30f7668a6` (E).)
- [ ] Archive plan-156-A…F to `planning/completed/` as each is done. F is
      archived last.

Acceptance: the three bug files are in `bugs/completed/`.
  Check: `ls bugs/completed/bug-69[123]-*` → 3 files (est. 1 s).
Commit: —

## Validation Plan (the whole plan-156 feature)

- Tests: every fixture added in A–F.
- Coverage check: the final `cargo test` run includes `rt_scope_drop_leaks`,
  `rt_thread_table_growth`, `rt_worker_arena_release` and
  `rt_thread_block_arena`. Confirm each by name in the output.
- Runtime proof: `examples/wind` rewritten with its original helper,
  `withNews(worker, playback)` borrowing the `RES` thread, runs to autoplay;
  its `--debug` report shows the worker arena unmapped after the forecast
  loads.
  **Done** (Correction 4): `examples/wind` itself now drains the queue in
  `drainNews(worker, p)`, lent the `RES` thread, and its stale comment ("handing
  a Thread to another function hands it over for good") is gone. A scratch copy
  that exits after collecting, built `--app --debug` and run headless with an
  open stdin: `p156 collected sent=40 playing=TRUE running=FALSE`; the worker
  arena mapped 179 chunks / 99,909,632 bytes and unmapped all 179; the block
  arena's 3 chunks unmapped; `thread.table_live 0`.
- Doc sync: listed in Phase 2.
- **Final gate (run once, here):**
  - `cargo test` (est. UNMEASURED: record the wall time in Corrections);
  - `bash scripts/test-accept.sh target/release/mfb $(mktemp -d)` (full
    corpus, est. 15 min per the `sync-goldens.sh` header);
  - `bash scripts/artifact-gate.sh target/release/mfb all` (est. 15–20 min
    per `.ai/testing-gates.md`; five targets are the only check for the
    per-target runtime).
  - Before the final gate, rebuild `target/release/mfb`: a stale release
    binary makes subprocess tests lie (`.ai/testing-gates.md`).

## Corrections

1. **The final gate is the thread tests, not the full suite** — by the user's
   instruction during this plan's execution: "Do not run the full suite as a
   final gate, only run the thread tests." The Validation Plan's full `cargo
   test`, full `test-accept.sh` and `artifact-gate all` as a gate are replaced
   by the thread fixtures (`rt-behavior/thread*/**`, `syntax/threads/**`,
   `rt-error/threads/**`, the two thread byte-identity sets), the Rust test
   targets that exercise threads (including the four the Validation Plan names),
   and the in-binary unit tests.

2. **Four package copies were stale on main.** `sync-package-mfp.sh` updated
   `self_fanout_workers` (migrated here) and also `regex_thread_workers`,
   `xfer_listener_worker` and two `xfer_tls_worker` copies, whose sources did
   not change. main's own compiler
   (`/Users/justinzaun/Development/mfb/target/release/mfb`) builds
   `regex_thread_workers` to the new bytes, not the committed ones — they were
   last synced in `0b067d0a7` (2026-08-31). The stale `self_fanout_workers`
   copy is what made `thread-package-fanout-rt` fail after the rejection (its
   package IR still bound a thread with `LET`).

3. **A ratchet the bug-695 fix broke.** `tests/guards/no_type_strings.rs`
   counted 53 `ParameterType::declared` sites in `ir` against a budget of 52:
   bug-695's `TypeIndex` resource set added one. Fixed in `b6208d7c9` by one
   shared `lower_link::native_resource_types`, which the shape pass (which built
   the same set) now uses too.

4. **The wind helper is `drainNews`, not `withNews`.** `withNews(news, p)` is
   already the per-message function the helper calls, so the new helper that
   takes the thread got its own name. One harness run printed "still running
   after 400 s": the timeout had landed just as the slower `--debug` download
   finished. Rerun waiting for the report, the process exited within 5 s of it,
   and main's compiler behaves the same on the pre-plan copy, so there is no
   hang at exit.

## Summary

Mechanical but wide: 159 bindings, 50 Rust test strings, 71 `.ast` goldens,
37 spec files and 12 man pages. The one semantic line is the rejection. The
risk is golden churn hiding a real regression, so every regenerated golden
diff must be the binding keyword or the documented `"resource": true`, and
anything else is a bug to root-cause.
