# plan-133-A: where the browser's memory goes, and an app-sized soak test

Last updated: 2026-09-12
Overall Effort: x-large (1d–3d)
Effort: large (3h–1d)
Depends on: nothing

`planning/todo.md` § Memory § 2 asks why a browser page load leaves the fetch worker
holding 841–856 MB it never reads again, and § 1 item 3 asks for an app-sized soak test
that fails today and will say when a fix works. This letter answers § 2 with
per-stage measurements from the plan-130 `--debug` report, classifies every leaking
stage against the known open defect (bug-536 Shape C: a value of a recursive type is
never freed), files a bug for every leak that is *not* Shape C, and lands the soak test.

Behavioral outcome: `planning/todo.md` § 2 records, for each stage of a page load
(fetch, parse, index, stylesheets, the copy back to the main thread, render), the
live bytes it leaves per call and which defect owns them; and
`tests/runtime/rt_debug_soak.rs` exists with a flat-workload control that passes and a
recursive-workload soak case that fails on today's compiler for the documented reason.

This letter diagnoses; it fixes nothing. Shape C's fix is a design plan of its own by
the user's ruling (bug-536 § "USER DECISION (2026-09-06) — shape C leaves the bug
backlog"), and this plan must not absorb it.

References — read these first:

- `planning/todo.md` § Memory (§ 1 item 3, § 2).
- `bugs/bug-536-scope-drop-leaks-recursive-types-return-constructor-string-temps.md`
  — Shape C, and "Shape C is blocked on recursive COPY-insertion".
- `src/docs/spec/tooling/09_debug-report.md` — the report's `arena.*` keys.
- `tests/runtime/rt_debug_arena.rs` — build/run/parse helpers this letter reuses.
- `tests/runtime/rt_scope_drop_leaks.rs` — the N vs 2N convention.
- memory note `recursive-type-values-are-second-class`.

## Prerequisites

The plan-133 family (A–C) is gated here; B and C point to this table.

| Must be true | Command | Status |
|---|---|---|
| plan-130 landed (the `--debug` report exists) | `ls planning/completed/plan-130-E-*` → one match | MET (2026-09-12) |
| plan-133 number unclaimed elsewhere | `git log --all --oneline --grep plan-133` → only this family | MET (2026-09-12) |
| The browser Wikipedia crash is fixed | `git merge-base --is-ancestor 6a29185d3 main && echo yes` → `yes` | MET (2026-09-12) |
| Box 2223 reachable with network | `ssh -p 2223 test@127.0.0.1 'curl -sS -m 10 -o /dev/null -w %{http_code} https://en.wikipedia.org/wiki/BASIC'` → `200` | MET (2026-09-12) |

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
- A per-stage table in § 2: live bytes left per call for each stage in §4.2, and the
  owning defect (bug-536 Shape C, or a new bug number).
- One `bugs/bug-NNN-*.md` per leaking stage that Shape C does not explain (zero if none).
- `tests/runtime/rt_debug_soak.rs` with the two cases in §4.3.
- The Bucket List in `planning/todo.md` gains the Apple Silicon page-size finding (§2).

### Non-goals (explicit constraints)

- No compiler, runtime, or allocator change. The only source changes are the new test
  file and planning/bug documents.
- No fix for Shape C or for any bug this letter files.
- No change to `examples/browser` (it is the workload, not the defect).

## 2. Current State

**The report.** `mfb build --debug` prints per-arena counters at exit
(`src/codegen/debug/arena.rs`, `ARENA_COUNTERS`, 18 entries: `maps`, `mapped_bytes`,
`alloc_calls`, `free_calls`, `live_bytes`, `peak_live_bytes`, `hit_*`, `grow`,
`flushes`, …) and `process.peak_rss_bytes` (`src/codegen/debug/process.rs`).

**Shape C.** `is_freeable_flat_value` (`src/codegen/engine/value/builder_values.rs`)
is false for a type that reaches itself, so a binding of such a type gets no scope-drop
free. `json::Json`, `canvas::DrawItem`, the regex continuations, and — by the same rule
— the browser's `dom::Node` are recursive (memory `recursive-type-values-are-second-class`).

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

| What | Value | Command |
|---|---|---|
| Browser `Main_Page` load, box 2223, main `f31de1d37`, `--debug` | worker: 109,560,293 allocs, 67,918,823 frees, 841,424,896 B live at exit; main: 112,143,664 B live | `drive-browser.exp` run recorded in `planning/todo.md` § 2 (2026-09-12) |
| Browser `BASIC` load, same | worker 856,035,392 B live; main 126,885,440 B live | same |
| `json::parse` of a 1,146,842-byte array, loop N=20 vs N=40, macOS, `--debug` | `arena.0.live_bytes` 1,059,198,720 → 2,118,397,440 (linear, 52,959,936 B per parse); `maps` 269,946 → 539,886 | `/tmp/probe-json/p20`, `p40` (source in §4.3) → report lines |
| `strings::split(text, ",")` of the same text, N=20 vs N=40 | `live_bytes` 0 → 0; `maps` 3 → 3; `alloc_calls` 23 → 43 = `free_calls` | `/tmp/probe-json/s20`, `s40` |
| Page size, macOS host vs box 2223 | 16,384 vs 4,096 | `sysctl -n hw.pagesize`; `ssh -p 2223 … getconf PAGESIZE` |
| JSON probe N=20: maps × 16,384 vs peak RSS | 4,422,795,264 vs 4,452,155,392 | arithmetic on the row above |

### Verified properties

- *The report's counters equal strace on a normal allocation workload* —
  `yamljson to-json samples/config.yaml --debug`: `arena.0.maps 12` equals the 12
  executable-IP anonymous maps counted by strace (2026-09-12).
- *A flat value loop does not grow* — the `strings::split` row above.
- *A recursive value loop grows linearly* — the `json::parse` row above. This is the
  Shape C signature and the soak test's failing case.
- *On Apple Silicon each 4 KiB arena block costs a 16 KiB page* — the page-size and
  maps × 16,384 rows: RSS ≈ 4× `mapped_bytes` on macOS, ≈ `mapped_bytes` on 2223
  (browser `Main_Page`: RSS 1,083,760,640 vs mapped 1,069,252,608).
- UNVERIFIED — that `dom::Node` is recursive in the `is_freeable_flat_value` sense.
  Phase 1's parse-twice run decides it by measurement.
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
build is fastest. **Box 2223** (native aarch64, 4 KiB pages) only for the one full
browser load that re-checks the attribution sums against the real worker.

**Risk:** misattribution — a stage measured in isolation may not leak the way it does
inside the worker (e.g. only when the value crosses `thread::waitFor`). Phase 3 closes
that by requiring the per-stage leaks to add up to the worker's measured live bytes
within 10%; if they do not, the gap is itself a finding that gets a row.

### Rejected alternatives

- *Per-call-site allocation attribution in the report.* Correct but a codegen feature;
  the stage programs answer § 2 without touching the compiler.
- *Fixing Shape C here.* Ruled out by the user (bug-536).
- *Asserting RSS in the soak test.* RSS is page-size dependent (4× on Apple Silicon);
  `live_bytes` is not.

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
(`build_debug`, `run_ok`, `arena_lines`, `counter` — moved to `tests/common` if both
files need them):

- `a_flat_split_loop_keeps_live_bytes_constant` — `strings::split` over a generated
  1 MiB string, N=20 vs N=40: `live_bytes` equal. Passes today.
- `a_json_parse_loop_keeps_live_bytes_constant` — `json::parse` of a generated
  ~1 MiB JSON array (built in Rust, same shape as the probe), N=20 vs N=40:
  `live_bytes(40) − live_bytes(20) < 1 MiB`. **Fails today** (measured: +1,059,198,720 B).
  Marked per Open Decision 1.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work; `- [~]` for partial with what remains; `- [x] ~~text~~ — moot: <evidence>`
> instead of deleting; fill `Commit:` the moment a phase lands. **An unticked box
> means NOT DONE.**

### Phase 1 — parse twice (the § 2 item 1 answer)

- [ ] Prove the source-package form: a scratch project importing `dom` from
      `packages/dom` builds with `--debug` (check: `mfb build --debug -q <dir>` exits 0,
      ~20 s).
- [ ] `parse` stage at N=1 and N=2 on `basic.html`, plus the `control` stage at N=1 and N=2
      (check: four report files; ~1 min on the host).
- [ ] Record in `planning/todo.md` § 2 item 1: the four `live_bytes`/`alloc_calls`/
      `free_calls` values and the verdict (leak per parse, or flat).

Acceptance: § 2 item 1 carries the four measurements and a one-line verdict; the control
reads `live_bytes` equal at N=1 and N=2 (if it does not, the method is wrong — fix the
harness before continuing).
Commit: —

### Phase 2 — every stage

- [ ] Run the remaining §4.2 stages at N and 2N (N chosen per stage so the 2N run finishes
      under 60 s on the host; record N). Check per stage: two report files (~1–2 min each).
- [ ] § 2 table: stage, N, leak per call (bytes), `alloc_calls`/`free_calls` per call,
      verdict.
- [ ] For each leaking stage, decide ownership: build a one-screen repro of the leaking
      value's type; if the type is recursive (reaches itself), it is Shape C; otherwise
      run the write-bug skill and file `bugs/bug-NNN-<slug>.md` with the repro and both
      measurements.

Acceptance: every stage has a row with an owner (Shape C or a bug number); every filed bug
has a failing reproduction per the write-bug skill.
Commit: —

### Phase 3 — reconcile with the real worker

- [ ] On box 2223 (native aarch64, 4 KiB pages), the recorded `Main_Page` worker live bytes
      (841,424,896) vs the sum of per-stage leaks for one load (parse + style links + attach css +
      resolve styles + index fields, as the worker runs them). Check: arithmetic against § 2's table; no new
      run unless a stage's N-scaling needs confirming on Linux (then one stage run, ~2 min).
- [ ] If the sum is outside ±10% of the worker's number, add a row naming the unexplained
      remainder and the next measurement that would localize it.

Acceptance: § 2 states whether the stages account for the worker's live bytes (within 10%)
or names the remainder.
Commit: —

### Phase 4 — the soak test and the Bucket List

- [ ] `tests/runtime/rt_debug_soak.rs` with both §4.3 cases, marked per Open Decision 1.
      Check: `cargo test --release --test rt_debug_soak -- --include-ignored` → the flat
      case passes and the json case fails with the §4.3 message (~3 min).
- [ ] `planning/todo.md` Bucket List: add the page-size finding (4 KiB default block vs
      16 KiB pages on Apple Silicon; the JSON probe numbers) under "Look into".
- [ ] `planning/todo.md` § 1 item 3: record the test name and its status.

Acceptance: the check above produces exactly one pass and one failure with the Shape C
message; `git grep -n "rt_debug_soak" planning/todo.md` → one match.
Commit: —

## Validation Plan

- Tests: `tests/runtime/rt_debug_soak.rs` (flat control active; json case per Open Decision 1).
- Runtime proof: the stage measurements (host) and the reconciliation (2223) recorded in
  `planning/todo.md` § 2.
- Doc sync: `planning/todo.md` § 1, § 2, Bucket List; any filed bug documents.
- Full suite: none for this letter — it changes no compiler code; the one new test file is
  checked by its own run above. The family's full suite runs once at the end of plan-133-C.

## Open Decisions

1. **How the failing soak case lands** — recommended: `#[ignore = "bug-536 Shape C: a
   recursive value is never freed; run with --include-ignored"]`, so CI stays green and the
   case is one flag away; the Shape C plan removes the `#[ignore]` as its acceptance.
   Alternative: land it active and red (breaks CI until Shape C lands); or pin today's leak
   as the expected value (asserts the bug).
2. **Where the shared test helpers live** — recommended: move `build_debug`, `run_ok`,
   `arena_lines`, `counter` from `rt_debug_arena.rs` into `tests/common` when the second
   file needs them; alternative: duplicate them in the new file.

## Corrections

## Summary

A measurement letter: the risk is misattributing a leak to the wrong stage, closed by
reconciling against the real worker. No compiler code changes; everything learned lands in
`planning/todo.md` and in bug documents.
