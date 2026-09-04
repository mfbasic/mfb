# plan-125-E: man iteration 2, batch 3 — crypto, canvas, http, vector, os, general, bits (142 pages)

Last updated: 2026-09-04
Effort: x-large (1d–3d)
Depends on: plan-125-D (iteration 2 is one ordered pass).

Landing unit: **each Phase below is independently landable and gets its own
commit.** The letter totals x-large; it is never landed as one change, and a
session that lands one phase and stops has left the tree consistent.

Iteration 2, batch 3: **systems, graphics and cryptography** — the packages
where an inaccurate man page has the highest consequence, because a developer
cannot check a crypto or a canvas claim by intuition the way they can check a
string function.

The unit is one page. See plan-125-A §3.2.

References:

- plan-125-A §3.2/§3.3/§4.3/§5; plan-125-B's terminology table.
- `.ai/man-content.md`; plan-108-A §3 (2a) memory-vocabulary ban.
- `.ai/canvas-threading.md` — the **internals foil** for `canvas`: the
  three-thread model, the scene ring, per-thread arena state, the resize
  handshake and the closed-flag texture-free rule are spec material. A canvas
  page states what the developer calls and what they see.
- `.ai/net-tls.md` — foil for `http`.
- `.ai/gpu-backend-traps.md` equivalent (`.ai/gpu-backend-traps` memory) and
  memory `wrong-oracle-is-ratified-not-caught` — a canvas claim verified only
  against another canvas call is not verified.
- Memory `audio-play-is-48k-mono-and-never-converts` — the class of hard
  format constraint a page must state plainly; check `canvas`/`vector` for the
  same class of unstated constraint.
- Memory `a-wrong-oracle-is-ratified` / `reproduce-the-actual-shape-not-a-proxy`
  — a crypto example that "works" against itself proves nothing; verify
  against a published vector where one exists.
- Memory `hpke-official-vectors-cfrg-json` — for any `crypto` page citing a
  standard, the vectors come from the CFRG JSON, not from a model's memory.

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-D complete | `grep -c '^- \[ \]' planning/plan-125-D-man-iter2-batch2.md` → `0` | — |

## 1. Goal

- **All 142 pages** through my per-page pass, one `codex exec` each, and apply.
- **Every example compiled and run.** `http` examples that need a server are
  run by the **main thread** against a local server — the Codex sandbox cannot
  bind sockets (plan-108-C's recorded lesson); the reviewer verifies the claim
  by reading code and the main thread runs the probe.
- **Every `crypto` claim that names a standard is checked against that
  standard's published vectors**, not against the implementation agreeing with
  itself.
- **`canvas`'s 106 type descriptions verified one at a time** — the largest
  single block of unverified prose in the man surface.
- Every finding has a verdict; every rejection a disproving command.
- `--reconcile` exits 0 over the 142-unit list; sweeps clean for all seven
  packages.
- "Belongs in spec" cuts appended, with `canvas` internals expected to
  dominate.

### Non-goals (explicit constraints)

- Per plan-125-A. No cross-page reconciliation (letter G owns it).
- **No example binds a listening socket from a reviewer worktree.**
- No wording churn on verified prose.
- Plan-125 does not fix canvas behavior; a canvas bug found here goes through
  `write-bug` and is recorded, per plan-125-A §3.5.

## 2. Current State

`canvas` was **missed by every letter of plan-108** (recorded in plan-108-E's
Corrections) and has had the most churn of any package since — 99 file-touches
(`git log --since=2026-08-31 --name-only --format='' --
src/codegen/builtins/canvas | wc -l` → 99). Letter B gave it its first
whole-package review; this letter gives it its first per-page one.

### Measured populations

| What | Count | Command |
|---|---|---|
| `crypto` units | 22 | 20 pages + overview + types — `./scripts/man-census.sh --fill` |
| `canvas` units | 21 | 19 + overview + types |
| `http` units | 21 | 19 + overview + types |
| `vector` units | 21 | 19 + overview + types |
| `os` units | 20 | 19 + overview (no types page) |
| `general` units | 19 | 18 + overview (no types page) |
| `bits` units | 18 | 17 + overview (no types page) |
| **batch total** | **142** | sum |
| parameter descriptions in batch | 232 | census: crypto 55, canvas 31, http 38, vector 38, os 9, general 21, bits 27 |
| type descriptions in batch | 166 | census: **canvas 106**, crypto 28, http 25, vector 27 |
| `canvas` file-touches since plan-108 | 99 | `git log --since=2026-08-31 --name-only --format='' -- src/codegen/builtins/canvas \| wc -l` — the highest of any package |
| `http` file-touches since plan-108 | 45 | same, `http` |
| `general` reachability | not in `mfb man --all` | `render_all_markdown` filters `is_unqualified_global()`; `mfb man --all \| grep -cE '^GENERAL$'` → 0. Reviewed here via `mfb man general --all` |

### Verified properties

- **`canvas`'s types page is the largest unverified prose block in the man
  surface** — VERIFIED by census: `TYPES 106/106`, versus the next largest
  (`datetime` 40). 106 descriptions have never been checked against the
  records and resources they describe.
- **`general` is invisible to `mfb man --all`** — VERIFIED by reading
  `src/cli/man.rs:render_all_markdown` and by grep (0 hits). Its 18 pages are
  reachable only via `mfb man general …`, which is why plan-125-A built
  `scripts/man-manual.sh`. Reviewing it here is the only pass it gets.
- UNVERIFIED: whether `canvas`'s 99 file-touches changed any documented
  behavior. That reconciliation is Phase 2's first task.

## 3. Design Overview

Per page: my pass → one `codex exec` → apply, `N` concurrency, main thread
sole writer.

**Order:** `bits` (18, the plan-108-A pilot package — cheapest, confirms the
harness) → `general` (19) → `os` (20) → `vector` (21) → `http` (21) →
`crypto` (22) → `canvas` (21 + the 106-description types page, last and
slowest).

**Risk concentration:**
- **`canvas` types page.** 106 descriptions, each naming a record field or a
  resource, each checkable against the descriptor. This is mechanical,
  high-yield verification and it is where this letter's time goes.
- **Crypto claims verified against themselves.** Memory
  `wrong-oracle-is-ratified-not-caught`: agreement between the implementation
  and an example built from the implementation is not evidence. Where a
  standard has published vectors, the vector is the oracle.
- **`http` probes need a server.** The reviewer cannot bind one. The prompt
  says so; the main thread runs the probe; the ledger records which claims
  were verified by main-thread probe rather than by the reviewer.
- **`os` and `general` are platform-conditional.** A claim true on macOS and
  false on Windows is a defect if the page does not say so. Check every
  platform-conditional claim against `src/target/**` and state the condition.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; `- [x] ~~text~~ — moot: <evidence>`
> rather than deleting; fill `Commit:` on landing. **Unticked means NOT DONE.**

### Phase 1 — bits, general, os (57 units)

- [ ] `bits` 18, `general` 19, `os` 20 — one review unit per page.
- [ ] Every platform-conditional `os`/`general` claim checked against
      `src/target/**` and stated with its condition on the page.
- [ ] Every example compiled and run; ledgers recorded.

Acceptance: 57 units `exit 0`; sweeps clean for all three; every
platform-conditional claim on an `os`/`general` page names the platforms it
holds for.
Commit: —

### Phase 2 — vector and http (42 units)

- [ ] `vector` 21, `http` 21.
- [ ] Reconcile `http`'s 45 file-touches since plan-108 against its pages
      before reviewing them.
- [ ] `http` examples requiring a server are run by the main thread; the
      ledger marks which claims that covered.
- [ ] Every example compiled and run; ledgers recorded.

Acceptance: 42 units `exit 0`; sweeps clean; the ledger distinguishes
reviewer-verified from main-thread-probe-verified claims for every `http`
network claim.
Commit: —

### Phase 3 — crypto (22 units)

- [ ] 20 pages + overview + the 28-description types page.
- [ ] Every named-standard claim checked against published vectors; the ledger
      records the vector source per claim.
- [ ] Every example compiled and run — and every example checked for
      *teaching a wrong pattern* (a key handled carelessly in an example is a
      defect even if the API call is correct).
- [ ] Ledger recorded.

Acceptance: 22 units `exit 0`; sweeps clean; every standard-referencing claim
in the ledger names its oracle, and no claim's oracle is the implementation
itself.
Commit: —

### Phase 4 — canvas (21 units, including the 106-description types page)

- [ ] Reconcile `canvas`'s 99 file-touches since plan-108 against its pages
      before reviewing them; record what changed and which pages it touched.
- [ ] 19 function pages + overview, one unit each.
- [ ] **The types page: all 106 descriptions verified one at a time** against
      the `RegistryRecord`/`RegistryResource`/property descriptors they
      describe. Record the count checked, so "verified" is a number.
- [ ] No thread, ring, arena or texture-lifetime detail on any page; each cut
      appended to `planning/plan-125-belongs-in-spec.md` against
      `src/docs/spec/app/06_canvas.md`.
- [ ] Every example compiled and run; ledger recorded.

Acceptance: 21 units `exit 0`; the ledger records **106/106** type
descriptions verified; `--reconcile` exits 0 over the whole 142-unit batch;
the example ledger accounts for all 142 examples, 0 unaccounted; sweeps clean
for all seven packages.
Commit: —

## Validation Plan

- Tests: none (man prose), except `tests/cli_canvas_man_examples_compile.rs` —
  which pins canvas example compilation and **must be run after Phase 4**; a
  changed canvas example updates it in the same commit.
- Coverage check: `--reconcile` over the 142-unit list; example ledger
  reconciled against the census function list, 0 unaccounted; the canvas type
  count reconciled against the census `106/106`.
- Runtime proof: `scripts/man-run-examples.sh <pkg> --run` for all seven;
  `mfb man general --all` and `mfb man <pkg> --all` render.
- Doc sync: `planning/plan-125-belongs-in-spec.md` appended (canvas internals
  expected to dominate).
- Acceptance: `--fill` 100%; `--memory-scope` 0 unclassified; `--scope` 0;
  `--reconcile` 0; `cargo test --test cli_canvas_man_examples_compile` green.

## Open Decisions

- **How does a canvas page describe frame timing without describing the
  ring?** — Recommend stating only what the developer controls and observes
  (call `present`, it returns when the frame is accepted) and linking
  `mfb spec app canvas` for the model. The temptation to explain the
  three-thread design on a man page is exactly the leak this plan exists to
  stop.
- **Does a `crypto` page name its standard's version/parameters?** —
  Recommend yes: a crypto page that says "SHA-256" without the construction
  or the output encoding is under-specified for its reader, and that is
  developer information, not internals.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

The yield is concentrated in `canvas` — 99 file-touches since the last review,
the only package plan-108 entirely missed, and a 106-description types page
nobody has checked — and the consequence is concentrated in `crypto`, where a
wrong page is a security defect rather than an inconvenience. The `http`
network claims are the one place the reviewer structurally cannot verify, so
the ledger has to say who verified what.
