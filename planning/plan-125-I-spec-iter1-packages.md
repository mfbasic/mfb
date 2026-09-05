# plan-125-I: spec iteration 1 — every spec package as a whole, plus the cross-package consistency review

Last updated: 2026-09-04
Effort: large (3h–1d)
Depends on: plan-125-H (the man surface is certified and
`planning/plan-125-belongs-in-spec.md` is closed — this letter opens that
ledger as a coverage checklist and cannot do so against a partial one).

The spec track begins. The audience flips: `src/docs/spec/**` is for **the
compiler contributor**, and for the developer who wants the internal detail.
Everything the man surface was forbidden to say is what this surface exists to
say — precisely, normatively, and with `[[path:Symbol]]` provenance.

Iteration 1 of three. The unit is **a whole spec package** read end to end
(`mfb spec <pkg> --all`). It is the only iteration that can see *contract
coverage* (an observable behavior with no home in any topic), *reading order*
(`PACKAGE_ORDER`, the overview's reading prose, `## See Also`), *single source
of truth* (`.ai/specifications.md`'s first convention — one canonical topic per
fact, never a second full copy), and *the ledger check* (does this package
already cover what the man surface cut?).

References:

- plan-125-A §3.1 (the audience table), §3.2, §3.3, §4.2
  (`scripts/spec-census.sh`), §4.3 (the harness), §5 (the spec iteration-1
  prompt).
- `.ai/spec-content.md` — the contributor-audience review standard authored in
  plan-125-A Phase 2.
- `.ai/specifications.md` — the standing rules: single source of truth,
  `[[ ]]` provenance at claim-cluster granularity, `PACKAGE_ORDER`, the
  as-is rule, and **the error-code registry as build input**.
- `planning/plan-125-belongs-in-spec.md` — closed by letter H; this letter's
  coverage checklist.
- `src/docs/spec/mod.rs:PACKAGE_ORDER` — the 12 packages in reading order.
- Memory `plan-line-citations-decay-silently`, `doc-sync-means-man-and-spec`,
  `completeness-claims-need-an-audit`.

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-H complete | `grep -c '^- \[ \]' planning/plan-125-H-man-final-sweep.md` → `0` | — |
| the "belongs in spec" ledger is closed with per-package attribution | plan-125-H Phase 4 | — |
| `scripts/spec-census.sh` reproduces its baseline | `./scripts/spec-census.sh --citations` → `MISS-SYMBOL 61`, `MISS-PATH 2`, `MISS-LINE 0` (or the re-measured Phase-1 figure) | — |

## 1. Goal

- **All 11 remaining packages** (12 minus `unicode`, done in plan-125-A's
  pilot) read end to end, reviewed by one `codex exec` each, and repaired.
- **Every entry in `planning/plan-125-belongs-in-spec.md` is resolved**: for
  each, either *already covered* (record the topic and the section) or *a spec
  gap* (a task in this letter or, if it is a whole new topic, a recorded task
  for letters J–L).
- **Contract coverage is checked, not assumed**: for each package, the
  externally observable contracts it owns are enumerated and each one is
  matched to a topic. A contract with no home is a finding.
- **Single-source-of-truth violations are found and fixed** — duplicated or
  contradicting bodies across topics, which is the defect class that only a
  whole-package (and, in the consistency review, whole-surface) read can see.
- **Reading order holds**: `PACKAGE_ORDER`, each overview's reading prose, and
  each `## See Also` reflect the topics that actually exist.
- **`--links` is clean** for every package: every `mfb spec`/`mfb man`
  reference in the text resolves.
- The cross-package consistency review (§3.2) is complete and applied.
- `--reconcile` exits 0 over the 11-unit list plus the consistency runs.

### Non-goals (explicit constraints)

- **Not a per-claim accuracy pass.** That is iteration 2 (letters J–L). Verify
  what the package-level lens surfaces.
- **No renumbering, reordering, or adding of spec packages/topics** beyond
  filling a gap the ledger proves; `PACKAGE_ORDER` is corrected only if it is
  wrong about what exists.
- **The memory-vocabulary ban does not apply here.** `borrow`, `ownership`,
  `move`, `pointer`, `heap` are the correct words on this surface
  (plan-108-A §3 (2a) carve-out 2). Do not "clean" them.
- **`src/docs/spec/diagnostics/02_error-codes.md` is build input.** Any edit
  to its Constant Registry table requires `cargo build` and
  `cargo test errorcode` (`table_matches_registry`) in the same commit.
- Compiler behavior is not changed by this letter; a code defect found goes
  through `write-bug` per plan-125-A §3.5.

## 2. Current State

`src/docs/spec/**` has **never been reviewed by any process**. There is no
tooling for it in the tree before plan-125-A (`ls scripts/ | grep -i spec` →
no matches), nothing checks a citation or a cross-link, and
`.ai/specifications.md` states rules that nothing measures.

The one measurable proxy for staleness — citation resolution — says the
surface has real rot: **61 of 1,280 symbol citations do not resolve**, and
they are not evenly spread.

### Measured populations

All commands run 2026-09-04 at HEAD `90f6c1357`.

| Package | Files | Lines | Citations | Broken citations |
|---|---|---|---|---|
| architecture | 24 | 5,030 | 510 | 6 |
| language | 23 | 3,499 | 133 | 1 |
| stdlib | 19 | 3,758 | 256 | **42** |
| package | 16 | 1,743 | 89 | 0 |
| linker | 14 | 1,837 | 148 | 2 |
| threading | 13 | 990 | 31 | 1 |
| memory | 10 | 2,109 | 139 | 4 |
| tooling | 9 | 2,553 | 241 | 4 |
| app | 7 | 1,971 | 149 | 3 |
| package-manager | 5 | 1,759 | 193 | 0 |
| diagnostics | 3 | 773 | 33 | 1 |
| unicode | 3 | 508 | 54 | 0 |
| **total** | **146** | **26,482** | **1,970** | **61** |

Commands: `find src/docs/spec/<pkg> -name '*.md' | wc -l`;
`cat src/docs/spec/<pkg>/*.md | wc -l`;
`grep -rhoE '\[\[[^]]+\]\]' src/docs/spec/<pkg> --include='*.md' | wc -l`;
broken-citation attribution by locating each of the 61 unresolved unique
citations in its source file.

| What | Count | Command |
|---|---|---|
| units in this letter | **11** | 12 packages − `unicode` (plan-125-A pilot) |
| unique citations | 1,414 | `grep -rhoE '\[\[[^]]+\]\]' src/docs/spec --include='*.md' \| sort -u \| wc -l` |
| symbol citations | 1,280 | plan-125-A §2 citation script |
| **broken symbol citations** | **61** | same — the number this track drives to 0 |
| malformed citations (symbol, no path) | 2 | `finalize_vreg_body_with_locals`, `run_register_allocation` |
| line-range citations, all in range | 19 / 0 bad | same script |
| code fences | 363 | `grep -rc '^```' src/docs/spec --include='*.md' \| awk -F: '{s+=$2} END {print s/2}'` |
| `mfb spec <pkg>` link targets, all resolving | 12 / 12 | loop over `grep -rhoE 'mfb spec [a-zA-Z-]+'` targets |
| `mfb spec --all` (global) | renders 0 lines | `./target/release/mfb spec --all \| wc -l` — the whole-surface artifact must be built from 12 per-package renderings |

### Verified properties

- **The rot is concentrated in `stdlib`: 42 of 61** — VERIFIED by locating
  each broken citation's source file. That is not random decay; `stdlib`
  documents the MFBASIC-implemented packages (`http`, `net`, `datetime`,
  `json`) whose helper symbols moved out of `mod.rs` into split
  `helper_*.rs`/`func_*.rs` files. Memory
  `splitting-package-mfb-render-order-doc-asymmetry` names the mechanism.
  This makes `stdlib` (letter K) the highest-yield spec package in the plan.
- **Two distinct rot classes, and they need different handling** — VERIFIED by
  spot-check in plan-125-A §2: `__http_dechunk` moved (`grep -rl` finds it in
  `helper_dechunk_bytes.rs`) and is fixable by re-pointing;
  `lower_io_write_helper` exists **only in the spec**
  (`grep -rl 'lower_io_write_helper' src/` → the spec file alone) and its
  surrounding *claim* is therefore suspect, not just its link.
- **`app` is being edited in the working tree at plan-writing** — VERIFIED:
  `src/docs/spec/app/04_term-backend.md` is modified and the package measures
  1,971 lines against 1,923 in an earlier run. plan-125-A's prerequisite gate
  covers this; re-measure at kickoff.
- UNVERIFIED, and the central unknown: **spec accuracy at HEAD**. Nothing has
  ever checked it. The 61 broken citations are a lower bound on staleness — a
  claim can be wrong with a perfectly resolving citation.

## 3. Design Overview

### 3.1 Order of units

1. **`stdlib`** — 42 of the 61 broken citations, and the package whose subject
   matter the man track just verified page by page. Its findings calibrate
   everything after it.
2. **`architecture`** — largest (24 files, 5,030 lines, 510 citations) and the
   one whose reading order most affects a new contributor.
3. **`memory`, `language`** — the contracts the man surface deferred to most
   often, so the `belongs-in-spec` ledger lands heaviest here.
4. **`app`, `tooling`, `linker`, `package`, `package-manager`, `threading`,
   `diagnostics`** — the remainder.

### 3.2 The cross-package consistency review

Four dimension-scoped runs over a **condensed artifact** — the 12 package
overviews plus every `## See Also` — because 26,482 lines cannot be one run:

1. **Single source of truth** — one fact, one canonical topic; find the second
   full copy and the contradicting one.
2. **Reading order** — does `PACKAGE_ORDER` plus each overview's reading prose
   describe a path a new contributor can actually walk?
3. **Contract coverage** — is there an observable compiler contract with no
   home in any package?
4. **Man↔spec agreement** — the two surfaces must not contradict. This run
   reads `planning/plan-125-belongs-in-spec.md` as its checklist and the
   certified man surface as its counterpart.

Findings are applied as a class across every affected package, not just the
one that surfaced them.

### 3.3 Risk

- **Scope creep into iteration 2.** Verifying claims here costs the plan a
  pass. Held by the prompt's lens and by classifying each finding by type in
  the ledger.
- **Fixing a broken citation without checking the claim.** The
  *stale-by-deletion* class makes the surrounding claim suspect; re-pointing
  a citation at a plausible symbol would ratify a wrong claim. Every
  `MISS-SYMBOL` whose symbol exists **nowhere** in `src/` is triaged as a
  claim, not a link.
- **A spec/code disagreement that is the code's fault.** Triaged per
  plan-125-A §3.5: spec stale → fix the spec; code wrong → `write-bug`,
  recorded either way. Not averaged, not skipped.
- **Editing the error-code registry.** It is build input; the gate is
  `cargo build` + `cargo test errorcode` in the same commit.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; `- [x] ~~text~~ — moot: <evidence>`
> rather than deleting; fill `Commit:` on landing. **Unticked means NOT DONE.**

### Phase 1 — stdlib (1 unit) and the citation-rot triage

- [ ] Re-run `./scripts/spec-census.sh --citations` and record the current
      per-package broken-citation table (the 2026-09-04 figures above are a
      snapshot).
- [ ] Split every `MISS-SYMBOL` into *stale by move* (symbol exists elsewhere
      in `src/`) and *stale by deletion* (symbol exists nowhere); record both
      lists here. The deletion list is the **suspect-claim list** letters J–L
      inherit.
- [ ] Review `stdlib` as a whole unit; repair its 42 broken citations, and for
      each deletion-class one, verify or correct the claim itself rather than
      the link.
- [ ] Resolve every `belongs-in-spec` ledger entry attributed to `stdlib`.

Acceptance: `./scripts/spec-census.sh --citations stdlib` → 0 `MISS-*`; the
two rot lists are recorded with counts; every deletion-class citation in
`stdlib` has a recorded verdict on its *claim*, not just its link;
`mfb spec stdlib --all` renders; `cargo test --bin mfb spec` green.
Commit: —

### Phase 2 — architecture, memory, language (3 units)

- [ ] Each read end to end; contract coverage enumerated and matched to
      topics; single-source-of-truth violations fixed.
- [ ] Resolve every `belongs-in-spec` entry attributed to these three — the
      heaviest concentration, since the man surface deferred its memory-model
      and type-system detail here.
- [ ] Repair their 11 broken citations (architecture 6, memory 4, language 1).

Acceptance: 3 units `exit 0`; `--citations` clean for all three; `--links`
clean; every ledger entry for these packages resolved as *covered* (with the
topic named) or *gap* (with a task); `cargo test --bin mfb spec` green.
Commit: —

### Phase 3 — the remaining 7 packages

- [ ] `app`, `tooling`, `linker`, `package`, `package-manager`, `threading`,
      `diagnostics` — one unit each.
- [ ] `diagnostics`: verify the Constant Registry table against the generated
      constants; **any edit** to `02_error-codes.md` gets `cargo build` +
      `cargo test errorcode` in the same commit.
- [ ] `app`: re-measure after the in-flight `term` work lands; reconcile
      `04_term-backend.md` against what actually changed.
- [ ] Repair their remaining broken citations (tooling 4, app 3, linker 2,
      threading 1, diagnostics 1).

Acceptance: 7 units `exit 0`; `./scripts/spec-census.sh --citations` → **0
`MISS-*` across the whole spec surface**; `--links` clean everywhere;
`cargo build` and `cargo test --bin mfb spec` green; `cargo test errorcode`
green if the registry table was touched.
Commit: —

### Phase 4 — the cross-package consistency review

- [ ] Build the condensed artifact (12 overviews + every `## See Also`).
- [ ] Run the four dimension-scoped reviews (§3.2).
- [ ] Apply each finding as a class; record which packages each touched.
- [ ] **Close `planning/plan-125-belongs-in-spec.md`**: every entry resolved
      as *covered* (topic named) or *gap* (task recorded, in this letter or
      assigned to J/K/L). Record the totals.
- [ ] `--reconcile` over the 11-unit list plus the four consistency runs.

Acceptance: four consistency runs `exit 0`; the ledger is fully resolved with
a recorded count of covered vs gap; `--reconcile` exits 0; the reading-order
check confirms `PACKAGE_ORDER` and every overview's reading prose match the
topics that exist.
Commit: —

## Validation Plan

- Tests: `cargo build` (the embedded spec table is generated — per
  `.ai/specifications.md`, `touch build.rs` if a brand-new file is not picked
  up) and `cargo test --bin mfb spec`. Plus `cargo test errorcode` if
  `diagnostics/02_error-codes.md` changed. Per memory
  `scope-the-test-run-to-the-blast-radius`, that is the blast radius.
- Coverage check: `--reconcile` over the unit list; the `belongs-in-spec`
  ledger reconciled to 0 unresolved entries.
- Runtime proof: `mfb spec <pkg> --all` renders for all 12 with no leaked
  `[[` markers; `./scripts/spec-census.sh --citations` and `--links` clean.
- Doc sync: this letter *is* spec sync; `.ai/specifications.md` updated only
  if a rule it states turns out to be wrong about the tree.
- Acceptance: 0 `MISS-*` surface-wide, `--links` clean, both cargo gates
  green, `--reconcile` 0.

## Open Decisions

- **A `belongs-in-spec` entry that needs a whole new topic** — recommend
  recording it as a task on the iteration-2 letter that owns that package
  (J, K or L), rather than authoring a topic in the middle of a whole-package
  review pass. A new topic is `NN_slug.md` beside the package's `spec.md`,
  auto-discovered, and the overview's reading prose must be updated with it
  (`.ai/specifications.md`).
- **A stale-by-deletion citation whose claim turns out to be simply wrong** —
  recommend deleting the claim rather than rewriting it from the current code,
  and recording it; rewriting it here is an iteration-2 act performed with
  iteration-1 attention.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

The measured signal entering this letter is that 42 of 61 broken citations sit
in one package, `stdlib`, from a code motion the spec never followed — so
Phase 1 is both the highest-yield unit and the calibration for everything
after it. The real unknown is that spec accuracy has never been measured at
all; iteration 1 can only find the structural defects, and it is honest about
leaving the per-claim work to J–L.
