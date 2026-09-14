# plan-133-A: where the browser worker's remaining memory goes, and an app-sized soak test

Last updated: 2026-09-13
Overall Effort: x-large (1d–3d)
Effort: large (3h–1d)
Depends on: nothing

`planning/todo.md` § Memory § 2 asks why a browser page load leaves the fetch worker
holding memory it never reads again, and § 1 item 3 asks for an app-sized soak test. When
this plan was written (2026-09-12) the answer looked like bug-536 Shape C: a value of a
recursive type was never freed. plan-134 (A–I, archived in `planning/completed/`) has
since fixed Shape C. Re-measured on 2026-09-13, the main thread's growth is gone, but the
worker still ends a page load holding **736–751 MB**, far more than the page it hands back.
This letter finds where that memory is, stage by stage, files a bug for every leak it
finds, and lands a soak test that guards the leaks plan-134 fixed.

Behavioral outcome: `planning/todo.md` § 2 records, for each stage of a page load (parse,
style links, attach css, resolve styles, index fields, the copy back to the main thread,
links/fields, paint, fetch), the live bytes it leaves per call and the bug that owns them
(or "flat"). `tests/runtime/rt_debug_soak.rs` exists: its flat cases pass, and every leak
this letter files has a case marked with that bug's number.

This letter diagnoses; it fixes nothing.

References — read these first:

- `planning/todo.md` § Memory (§ 1 item 3, § 2).
- `planning/completed/plan-134-H-collection-element-drops-and-close-out.md` — what Shape C's
  fix covers, and the `json_repeat` / `regex_repeat` flat cases it added to
  `rt_scope_drop_leaks.rs`.
- `bugs/completed/bug-536-scope-drop-leaks-recursive-types-return-constructor-string-temps.md`.
- `src/docs/spec/tooling/09_debug-report.md` — the report's `arena.*` keys.
- `tests/runtime/rt_debug_arena.rs` — build/run/parse helpers this letter reuses.
- `tests/runtime/rt_scope_drop_leaks.rs` — the N vs 2N convention.

## Prerequisites

The plan-133 family (A–C) is gated here; B and C point to this table.

| Must be true | Command | Status |
|---|---|---|
| plan-130 landed (the `--debug` report exists) | `ls planning/completed/plan-130-E-*` → one match | MET (2026-09-13) |
| plan-134 landed (Shape C fixed) | `ls planning/completed/plan-134-H-*` → one match | MET (2026-09-13) |
| plan-133 number unclaimed elsewhere | `git log --all --oneline --grep plan-133` → only this family | MET (2026-09-13) |
| The browser Wikipedia crash is fixed | `git merge-base --is-ancestor 6a29185d3 main && echo yes` → `yes` | MET (2026-09-13) |
| Box 2223 reachable with network | `ssh -p 2223 test@127.0.0.1 'curl -sS -m 10 -o /dev/null -w %{http_code} https://en.wikipedia.org/wiki/BASIC'` → `200` | MET (2026-09-13) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop.
>
> **If you stop, report the current status of *all* prerequisites** — not only the one
> that blocked you.

## 1. Goal

- `planning/todo.md` § 2 item 1 ("never freed, or freed but not reused?") answered with a
  measurement on the browser's own `dom::parse`, on the main thread, at N=1 and N=2.
- A per-stage table in § 2: live bytes left per call for each stage in §4.2, and the owner
  (a bug number, or "flat").
- One `bugs/bug-NNN-*.md` per leaking stage (zero if none).
- § 2 states whether the per-stage leaks account for the worker's measured live bytes
  (751,305,088 B for `Main_Page`, within 10%), or names what is left.
- `tests/runtime/rt_debug_soak.rs` with the cases in §4.3.
- The Bucket List in `planning/todo.md` gains the Apple Silicon page-size finding (§2).

### Non-goals (explicit constraints)

- No compiler, runtime, or allocator change. The only source change is the new test file;
  everything else is planning and bug documents.
- No fix for any bug this letter files.
- No change to `examples/browser` (it is the workload, not the defect).
- Not worker-arena reclamation (Bucket List 1). That frees a finished worker's whole arena;
  this letter asks why the worker holds live allocations in the first place.

## 2. Current State

**The report.** `mfb build --debug` prints per-arena counters at exit
(`src/codegen/debug/arena.rs`, `ARENA_COUNTERS`, 18 entries: `maps`, `mapped_bytes`,
`alloc_calls`, `free_calls`, `live_bytes`, `peak_live_bytes`, `hit_*`, `grow`,
`flushes`, …) and `process.peak_rss_bytes` (`src/codegen/debug/process.rs`).

**Recursive values.** plan-134 made recursive values (`json::Json`, `dom::Node`, …)
copied at owning stores and freed exactly once (`_mfb_rt_graph_drop`, letters F–H). The
`json::parse` loop that leaked 53 MB per parse on 2026-09-12 is now flat (§ Measured).

**The browser pipeline** (`examples/browser`): the worker `fetch::fetch`
(`examples/browser/fetch/src/lib.mfb`) runs `http::read` (following redirects), then
`pageResult`: `dom::parse(body)` → `dom::styleLinks` → `fetchStyles` (private to `fetch`,
network) → `dom::attachCss(document, texts)` → `dom::resolveStyles` → `dom::indexFields`
→ `dom::title`, and returns a `LoadResult` that `thread::waitFor` copies into the main
arena. The main thread then runs `display::links`, `dom::fieldSpecs` and the render
(`display::paint`, via `dom::updateLayout`) (`examples/browser/app/src/main.mfb`).
Measured: `sed -n '/^FUNC pageResult/,/^END FUNC/p' examples/browser/fetch/src/lib.mfb`;
`grep -rn "^EXPORT FUNC" examples/browser/{dom,display}/src`.

### Measured populations

Browser runs: `mfb build --debug --target linux-aarch64` of `examples/browser` (its three
packages rebuilt from source), run on box 2223 in a 120x40 pty by `drive-browser.exp`
(load the page, wait 40 s, `q`). Every run below loaded and rendered the page and exited 0
(checked by replaying the captured screen).

| Run | Arena | maps | mapped B | alloc calls | alloc B | free calls | live at exit B | peak live B |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| `Main_Page`, main `f31de1d37` (2026-09-12, before plan-134) | main | 15,236 | 174,211,072 | 290,570 | 268,876,176 | 194,790 | 112,143,664 | 114,652,480 |
| same | worker | 192,273 | 895,041,536 | 109,560,293 | 4,006,566,864 | 67,918,823 | 841,424,896 | 842,257,648 |
| `Main_Page`, main `db8e34157` (2026-09-13, after plan-134) | main | 5,125 | 121,880,576 | 378,914 | 413,522,096 | 372,100 | 4,678,544 | 33,909,584 |
| same | worker | 189,654 | 858,677,248 | 113,507,047 | 5,887,193,872 | 70,952,128 | 751,305,088 | 775,704,320 |
| `BASIC`, main `f31de1d37` (2026-09-12) | main | 15,052 | 140,185,600 | 175,730 | 207,729,408 | 101,950 | 126,885,440 | 128,348,752 |
| same | worker | 194,378 | 940,584,960 | 112,504,257 | 4,309,203,872 | 71,072,133 | 856,035,392 | 857,598,432 |
| `BASIC`, main `db8e34157` (2026-09-13) | main | 4,140 | 71,380,992 | 254,343 | 325,823,296 | 249,814 | 5,479,744 | 36,252,224 |
| same | worker | 184,728 | 880,959,488 | 114,198,542 | 6,093,716,864 | 72,756,959 | 735,804,400 | 758,952,576 |

`process.peak_rss_bytes`: 1,083,760,640 / 1,095,602,176 (2026-09-12, `Main_Page` / `BASIC`)
→ 995,594,240 / 967,356,416 (2026-09-13).

| Probe (macOS host, `--debug`) | Result | Command |
|---|---|---|
| `json::parse` of a 1,146,842-byte array, loop N=20 vs N=40, main `f31de1d37` (2026-09-12) | `live_bytes` 1,059,198,720 → 2,118,397,440 (52,959,936 B per parse); `maps` 269,946 → 539,886; peak RSS 4,452,155,392 at N=20 | `/tmp/probe-json/p20`, `p40` |
| same, main `78550d5e0` (2026-09-13, after plan-134; code identical to `db8e34157`, which added only a backlog doc) | `live_bytes` 0 → 0; `maps` 9,016 → 9,016; `alloc_calls` = `free_calls` (31,833,542 / 63,667,082); peak RSS 165,085,184 both | same programs, fresh release build |
| `strings::split(text, ",")`, same text, N=20 vs N=40 (2026-09-12) | `live_bytes` 0 → 0; `maps` 3 → 3 | `/tmp/probe-json/s20`, `s40` |
| Page size, macOS host vs box 2223 | 16,384 vs 4,096 | `sysctl -n hw.pagesize`; `ssh -p 2223 … getconf PAGESIZE` |
| JSON probe N=20 (2026-09-12): maps × 16,384 vs peak RSS | 4,422,795,264 vs 4,452,155,392 | arithmetic on the rows above |

### Verified properties

- *The report's counters equal strace on a normal allocation workload* —
  `yamljson to-json samples/config.yaml --debug`: `arena.0.maps 12` equals the 12
  executable-IP anonymous maps counted by strace (2026-09-12).
- *plan-134 removed the recursive-value leak on the JSON workload* — the two `json::parse`
  rows above.
- *plan-134 removed the main thread's growth in the browser* — main-arena live at exit
  112,143,664 → 4,678,544 (`Main_Page`) and 126,885,440 → 5,479,744 (`BASIC`).
- *The worker's remaining live bytes are not the page it returns* — the main arena, which
  receives that page from `thread::waitFor`, never holds more than 33,909,584 B live
  (`Main_Page`) or 36,252,224 B (`BASIC`), counting the render too. The worker ends with
  751,305,088 B / 735,804,400 B. So at least 717,395,504 B (`Main_Page`) and 699,552,176 B
  (`BASIC`) in the worker is something other than the returned document.
- *plan-134's copies raised allocation volume* — the worker requests 5,887,193,872 B vs
  4,006,566,864 B before (×1.47, `Main_Page`) and 6,093,716,864 B vs 4,309,203,872 B
  (×1.41, `BASIC`).
- *On Apple Silicon each 4 KiB arena block costs a 16 KiB page* — the page-size and
  maps × 16,384 rows: RSS ≈ 4× `mapped_bytes` on macOS, ≈ `mapped_bytes` on 2223.
- UNVERIFIED — that `dom::parse` alone is flat after plan-134. Phase 1 decides it.
- UNVERIFIED — that a scratch project can depend on `examples/browser/dom` as a source
  package with `"source": "file:packages/dom"` (the form documented at the top of
  `src/cli/build/source_packages.rs`). Phase 1's first task proves it.

## 3. Design Overview

Every question is answered the same way: a small program built with `--debug` runs one
stage N times and 2N times; `live_bytes(2N) − live_bytes(N)` divided by N is that stage's
leak per call. A flat control in the same shape proves the method reads zero when
nothing leaks. No instrumentation is added.

Where each run happens: **the macOS host** for every per-stage measurement —
`live_bytes`, `alloc_calls` and `free_calls` are page-size independent, and the host
build is fastest. **Box 2223** (native aarch64, 4 KiB pages) only for the full browser
load the stages are reconciled against (the 2026-09-13 rows above).

**Risk:** misattribution — a stage measured in isolation may not leak the way it does
inside the worker (for example, only when the value crosses `thread::waitFor`, or only in
a worker arena). Phase 3 closes that by requiring the per-stage leaks to add up to the
worker's measured live bytes within 10%; if they do not, the gap is itself a finding that
gets a row, and the next measurement runs the suspect stage inside a `thread::start` worker.

### Rejected alternatives

- *Per-call-site allocation attribution in the report.* Correct but a codegen feature;
  the stage programs answer § 2 without touching the compiler.
- *Asserting RSS in the soak test.* RSS is page-size dependent (4× on Apple Silicon);
  `live_bytes` is not.
- *Reclaiming the worker arena instead.* Bucket List 1 is a separate problem; it would hide
  these live bytes from the report without explaining them.

## 4. Detailed Design

### 4.1 Harness for stage programs

A scratch directory per stage, outside the repo (`/tmp/plan-133-a/<stage>-<n>`), each a
`project.json` + `src/main.mfb` with the loop count baked in (no `os::args`, matching
`rt_scope_drop_leaks.rs`). Stages needing the browser packages copy
`examples/browser/{dom,fetch,display}` into `packages/` and reference them as source
packages. The saved inputs are fetched once:
`curl -sSL -o /tmp/plan-133-a/basic.html https://en.wikipedia.org/wiki/BASIC` and
the same for `Main_Page`. Results go into § 2's table with the build command and N.

### 4.2 Stages

Each stage runs the worker's own calls in the worker's order; every stage before it runs
once, before the loop, on the saved input.

| Stage | Loop body (N times) | Signature (measured) |
|---|---|---|
| parse | `LET d AS dom::Node = dom::parse(html)` | `dom/src/parse.mfb`: `parse(html AS String) AS Node` |
| style links | `LET l = dom::styleLinks(d)` | `dom/src/lib.mfb`: `styleLinks(doc AS Node) AS List OF String` |
| attach css | `LET d2 = dom::attachCss(d, css)` — `css` = the page's stylesheets saved with `curl` once (`fetchStyles` is private to `fetch`) | `attachCss(doc AS Node, extra AS List OF String) AS Node` |
| resolve styles | `LET d3 = dom::resolveStyles(d2)` | `dom/src/resolve.mfb`: `resolveStyles(doc AS Node) AS Node` |
| index fields | `LET d4 = dom::indexFields(d3)` | `indexFields(doc AS Node) AS Node` |
| copy-back | a worker returning the resolved, indexed document in a `LoadResult` via `thread::waitFor`; N workers, sequentially | — |
| links/fields | `display::links(d4)` and `dom::fieldSpecs(d4)` | `display/src/lib.mfb`: `links(root AS Node) AS List OF Link`; `fieldSpecs(doc AS Node) AS List OF FieldSpec` |
| paint | `display::paint(d4, 120, 8, 16)` | `paint(root AS Node, widthCols AS Integer, cellPx AS Integer, linePx AS Integer) AS PaintResult` |
| fetch | `http::read(net::toUrl("https://en.wikipedia.org/wiki/BASIC"))` (network; N small) | — |
| control | `strings::split(html, "<")` — must read `live_bytes` flat | — |

### 4.3 The soak test

`tests/runtime/rt_debug_soak.rs`, reusing `rt_debug_arena.rs`'s helpers
(`build_debug`, `run_ok`, `arena_lines`, `counter` — moved to `tests/common` per Open
Decision 2):

- `a_flat_split_loop_keeps_live_bytes_constant` — `strings::split` over a generated
  1 MiB string, N=20 vs N=40: `live_bytes` equal. The method's control; passes today.
- `a_json_parse_loop_keeps_live_bytes_constant` — `json::parse` of a generated ~1 MiB JSON
  array (built in Rust, the probe's shape), N=20 vs N=40: `live_bytes(40) − live_bytes(20)
  < 1 MiB`. Guards plan-134's fix at an app-sized input. **Passes today** (measured 0 → 0);
  it failed on 2026-09-12 with +1,059,198,720 B.
- `a_dom_parse_loop_keeps_live_bytes_constant` — `dom::parse` of the saved `BASIC` HTML
  (committed as a test input under `tests/runtime/data/`), N=2 vs N=4. Passes if Phase 1
  finds `dom::parse` flat; otherwise it lands marked per Open Decision 1 with the bug Phase 2
  files.
- One case per leaking stage from Phase 2, same N vs 2N shape, marked per Open Decision 1.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work; `- [~]` for partial with what remains; `- [x] ~~text~~ — moot: <evidence>`
> instead of deleting; fill `Commit:` the moment a phase lands. **An unticked box
> means NOT DONE.**

### Phase 1 — parse twice (the § 2 item 1 answer)

- [x] Prove the source-package form: a scratch project importing `dom` from
      `packages/dom` builds with `--debug` (check: `mfb build --debug -q <dir>` exits 0,
      ~20 s). — `/tmp/plan-133-a/run.sh parse 1`: build exit 0 (dom, display, fetch all as
      `file:packages/<name>` source packages), program rc=0.
- [x] `parse` stage at N=1 and N=2 on `basic.html`, plus the `control` stage at N=1 and N=2
      (check: four report files; ~1 min on the host). — `out/{parse,control}-{1,2}.report`;
      parse `live_bytes` 8,332,368 → 16,651,216; control 13,520 → 13,520. Each parse run
      took 36–39 s, not ~15 s.
- [x] Record in `planning/todo.md` § 2 item 1: the four `live_bytes`/`alloc_calls`/
      `free_calls` values and the verdict (leak per parse, or flat). — recorded; verdict
      "never freed", 8,318,848 B per parse.

Acceptance: § 2 item 1 carries the four measurements and a one-line verdict; the control
reads `live_bytes` equal at N=1 and N=2 (if it does not, the method is wrong — fix the
harness before continuing).
Commit: ad6d737bc

### Phase 2 — every stage

- [x] Run the remaining §4.2 stages at N and 2N (N chosen per stage so the 2N run finishes
      under 60 s on the host; record N). Check per stage: two report files (~1–2 min each).
      — `out/{stylelinks,attachcss,resolve,index,linksfields,paint,copyback,fetch}-{1,2}.report`
      (fetch also N=4); N=1/2 everywhere because the setup parse alone takes ~40 s, so every
      2N run exceeds 60 s (resolve 64 s, copy-back 75 s). Added: plain-HTTP fetch over
      loopback (N=20/40), and a Main_Page copy-back run for Phase 3 (`copyback_mp`).
- [x] § 2 table: stage, N, leak per call (bytes), `alloc_calls`/`free_calls` per call,
      verdict. — `planning/todo.md` § 2 item 1, "Every stage": leaks in parse 8,318,848,
      resolve 718,525,504, copy-back 5,268,592, paint 89,520, fetch 384 (HTTPS) / 62,435
      (HTTP); flat: style links, attach css, index fields, links/fields, control.
- [x] Added task: prove ownership by rewriting the suspected sites in a scratch package copy
      and re-measuring — `/tmp/plan-133-a/patch_dom.py` (13 sites): parse 13,520 → 13,520,
      resolve 13,520 → 13,520, BASIC paint leak 89,520 → 1,632 B per call.
- [x] For each leaking stage, run the write-bug skill and file `bugs/bug-NNN-<slug>.md`
      with a one-screen repro and both measurements. Before filing, check the open bugs and
      `planning/bug-backlog.md` for the same shape (`grep -rli` on the leaking value's type
      and operation), and cite the match instead if one exists. — No match: a grep of open
      bugs and the backlog for `waitFor`/`control block`, condition temps, `FOR EACH` over a
      field, and `AttributedString`/`astrings` finds only unrelated hits (bug-540, bug-564,
      bug-605, bug-610). Filed:
      bug-620 (IF-condition temp on RETURN/EXIT: parse, resolve, layout),
      bug-621 (loop-condition temp per pass: parse, resolve, layout),
      bug-622 (thread result copy and plumbing: copy-back),
      bug-623 (`tcp::read` buffer and connection records: fetch),
      bug-625 (`AttributedString` drop: paint's canvas remainder), plus bug-624 (a private
      package type collides with a program type; found while probing, not a leak). Each has a
      measured one-screen repro at N and 2N.

Acceptance: every stage has a row with an owner (a bug number, or "flat"); every filed bug
has a failing reproduction per the write-bug skill.
Commit: c8a650372, a6b7d9c95

### Phase 3 — reconcile with the real worker

- [x] Compare the 2026-09-13 `Main_Page` worker live bytes (751,305,088) with the sum of
      per-stage leaks for one load (parse + style links + attach css + resolve styles +
      index fields, as the worker runs them). Check: arithmetic against § 2's table; no new
      run. — Stages were measured on `BASIC`, so that sum is compared with the `BASIC`
      worker: 732,105,824 (stage leaks + returned document) against 735,804,400 on 2223
      (99.5%) and 732,119,344 host `copyback` N=1. For `Main_Page`, one host run of the
      worker pipeline (`copyback_mp`) gives 749,061,840 against 751,305,088 (99.7%); with the
      bug-620/621 sites rewritten it drops to 4,239,824 (the returned page). Recorded in
      `planning/todo.md` § 2 "Do the stages account for the worker?".
- [x] ~~If the sum is outside ±10%, run the largest-leaking stage inside a `thread::start`
      worker on the host (one run, ~2 min) and record whether the worker context changes it;
      add a row naming any unexplained remainder and the next measurement that would
      localize it.~~ — moot: the sum is inside ±10% (99.5% `BASIC`, 99.7% `Main_Page`). The
      worker-context run was done anyway as `copyback`/`copyback_mp`: it matches the
      single-thread stage sum within 13,520 B (the program baseline).

Acceptance: § 2 states whether the stages account for the worker's live bytes (within 10%)
or names the remainder.
Commit: aef062601

### Phase 4 — the soak test and the Bucket List

- [x] `tests/runtime/rt_debug_soak.rs` with the §4.3 cases, marked per Open Decision 1.
      — `cargo test --release --test rt_debug_soak -- --include-ignored` → EXIT=101,
      `2 passed; 5 failed` in 143.54 s. Passed: `a_flat_split_loop…`, `a_json_parse_loop…`.
      Failed with their bug messages: dom parse +8,318,848 B (bug-620/621), resolve
      +24,608,640 B (bug-620/621), paint +1,920,000 B (bug-620/621/625), thread copy-back
      +2,912,000 B (bug-622), http +1,314,240 B (bug-623). The dom case is bug-marked because
      Phase 1 found `dom::parse` not flat. Resolve, paint, thread and http cases were added
      (one per leaking stage). The dom case runs N=1/2 (Corrections). Helpers moved to
      `tests/common/debug_report.rs` (Open Decision 2). Input committed as
      `tests/runtime/data/rt_debug_soak_basic.html`.
      Check: `cargo test --release --test rt_debug_soak -- --include-ignored` → the flat,
      json and (if flat) dom cases pass, and each bug-marked case fails with its bug's
      message (~3 min).
- [x] `planning/todo.md` Bucket List: add the page-size finding (4 KiB default block vs
      16 KiB pages on Apple Silicon; the JSON probe numbers) under "Look into". — item 12a
      (page sizes 16,384 vs 4,096; 269,946 maps × 16,384 = 4,422,795,264 B vs peak RSS
      4,452,155,392 B).
- [x] `planning/todo.md` § 1 item 3: record the test name and its status. — "Landed
      (plan-133-A, 2026-09-13)", with a status row per case from the full run above.
      `git grep -n "rt_debug_soak" planning/todo.md` → exactly one match (line 485).

Acceptance: the check above passes every unmarked case and fails every bug-marked case with
its message; `git grep -n "rt_debug_soak" planning/todo.md` → one match.
Commit: 477c40624

## Validation Plan

- Tests: `tests/runtime/rt_debug_soak.rs` (flat, json and dom cases active; one bug-marked
  case per leak found).
- Runtime proof: the stage measurements (host) and the reconciliation against the 2026-09-13
  2223 run, recorded in `planning/todo.md` § 2.
- Doc sync: `planning/todo.md` § 1, § 2, Bucket List; any filed bug documents.
- Full suite: none for this letter — it changes no compiler code; the one new test file is
  checked by its own run above. The family's full suite runs once at the end of plan-133-C.

## Open Decisions

1. **How a failing soak case lands** — recommended: `#[ignore = "bug-NNN: <one-line shape>;
   run with --include-ignored"]`, so CI stays green and the case is one flag away; the fix for
   bug-NNN removes the `#[ignore]` as its acceptance. Alternative: land it active and red
   (breaks CI until the fix lands); or pin today's leak as the expected value (asserts the bug).
2. **Where the shared test helpers live** — recommended: move `build_debug`, `run_ok`,
   `arena_lines`, `counter` from `rt_debug_arena.rs` into `tests/common` when the second
   file needs them; alternative: duplicate them in the new file.

## Corrections

- **2026-09-13 — re-scoped after plan-134 landed.** The plan was written (2026-09-12)
  assuming bug-536 Shape C owned the worker's 841–856 MB, and that the soak test's json case
  would fail. plan-134 fixed Shape C. Re-measured the same day on `db8e34157`: the
  `json::parse` probe is flat (`live_bytes` 0 → 0), the main thread's live bytes at exit
  fell from 112–127 MB to 4.7–5.5 MB, but the worker still holds 736–751 MB, of which at
  least 700–717 MB is not the returned page (§ Verified properties). Changes: the goal is the
  worker's remaining live bytes; leaks are classified by bug, not against Shape C; the json
  soak case is a regression guard that passes today; a `dom::parse` case is added; plan-134
  is a met prerequisite; the reconciliation target is 751,305,088 B.
- **2026-09-13 — Phase 1: a parse takes 36–39 s, not ~15 s.** Measured: `run.sh parse 1`
  `secs=36`, `parse 2` `secs=39` (`--debug` build, macOS host). Consequence for Phase 2: every
  stage after parse pays that setup, so N=1 / 2N=2 everywhere (the 2N runs take 38–75 s, over the
  60 s target). A single N=1→2 difference is enough here: every flat stage reads exactly 0 B
  per call, and the smallest leak is 384 B (fetch, confirmed linear at N=1/2/4).
- **2026-09-13 — Phase 2: "a one-screen repro of the leaking value's type" was too narrow.**
  Every leak turned out to come from a code shape, not from a value type: condition temps
  (bug-620, bug-621), the thread result copy and plumbing (bug-622), and native transport
  buffers (bug-623). Ownership was proven by an added task: the suspected sites were rewritten
  in a scratch copy of the package, and the stage was measured again (`/tmp/plan-133-a/patch_dom.py`).
- **2026-09-13 — Phase 2: the fetch stage has two numbers.** Over HTTPS to Wikipedia,
  `http::read` leaks 384 B per call. Over plain HTTP it leaks 62,435 B, because `tcp::read`'s
  64 KiB buffer is never freed (bug-623). The browser's worker uses HTTPS, so its share is 384 B.
- **2026-09-13 — Phase 3 compared unlike inputs.** The task compared the `Main_Page` worker
  with stages measured on `BASIC`. Corrected to compare like with like: `BASIC` stages against
  the `BASIC` worker, plus one host run of the `Main_Page` pipeline against the `Main_Page` worker.
- **2026-09-13 — §4.3 dom case at N=1 vs 2, not N=2 vs 4.** A parse takes ~40 s, so N=2/4 is
  ~4 min for a case whose leak (8.3 MB per parse) exceeds the 1 MiB bound 8× at N=1/2.
- **2026-09-13 — bug-619 was taken by a peer.** An uncommitted
  `bugs/bug-619-leb128-decode-misses-overflow-on-tenth-byte.md` exists in the shared main
  checkout, so this letter's bugs start at 620.

## Summary

A measurement letter aimed at the one large retention plan-134 did not remove: the worker
ends each page load holding ~740 MB that is not the page it returns. The risk is
misattributing that memory to the wrong stage, closed by reconciling against the real worker.
No compiler code changes; everything learned lands in `planning/todo.md` and bug documents,
and the soak test keeps plan-134's fix from regressing.
