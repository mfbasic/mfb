# plan-125-N: the final whole-spec contributor-doc check, and the plan-125 closeout

Last updated: 2026-09-04
Effort: large (3h–1d)
Depends on: plan-125-M (all three spec iterations complete; M declares the
spec surface final and this letter certifies it).

The last letter. Two jobs:

1. **Certify the spec surface** — six lenses, each one `codex exec` run over
   the whole rendered specification with exactly one question, because no
   single run reads 26,482 lines usefully with six questions at once. One of
   those lenses is the one no earlier letter could run: **do `mfb man` and
   `mfb spec` contradict each other?** Both surfaces are final by now, and
   this is the only point in the plan where they can be compared as finished
   artifacts.
2. **Close plan-125** — the whole-plan certificate, the tooling left behind,
   the durable lessons into memory, and all fourteen letters archived.

As in letter H, this is a **certification** in plan-108-F's sense: the
certificate is a measured, re-runnable sweep pasted into this file with its
commands, never an assembly of ticked boxes (memory
`completeness-claims-need-an-audit`). Anything found is fixed here.

References:

- plan-125-A §4.4 (the six spec lenses), §4.2, §4.3, §5 (the spec-final
  prompt).
- plan-125-H — the man surface's certificate, and this letter's counterpart
  for the man↔spec lens.
- plan-125-I Phase 4 (contract coverage, reading order, the closed
  `belongs-in-spec` ledger); plan-125-M Phase 4 (the iteration-3 result).
- `.ai/specifications.md`, `.ai/spec-content.md`.
- AGENTS.md — the auto-memory rules (record lessons, never status) and the
  archive rule for completed plans; memory
  `completed-plans-go-to-old-plans` (MOVE to `planning/completed/`, never
  delete).

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-M complete | `grep -c '^- \[ \]' planning/plan-125-M-spec-iter3-packages.md` → `0` | — |
| plan-125-H complete (the man surface is certified) | `grep -c '^- \[ \]' planning/plan-125-H-man-final-sweep.md` → `0` | — |
| the whole-spec artifact builds | the 12 `mfb spec <pkg> --all` renderings concatenate deterministically | — |

## 1. Goal

The spec certificate, pasted into this file with each command's output:

- **Coverage** — the artifact contains all **12** packages and all **146**
  files; every package renders and none is empty.
- **Citations** — `./scripts/spec-census.sh --citations` → **0 `MISS-*`**
  across the whole surface, against a **61 `MISS-SYMBOL` / 2 `MISS-PATH`**
  baseline measured 2026-09-04.
- **Links** — `--links` clean: every `mfb spec`/`mfb man` reference in the
  spec resolves to a real target.
- **Rendering** — `--render` clean: every package renders with **no leaked
  `[[` markers**.
- **Single source of truth** — lens 2 finds no fact with two full homes and no
  two topics contradicting each other.
- **Contract completeness** — lens 1 finds no observable compiler contract
  without a home.
- **Man↔spec agreement** — lens 6 finds no contradiction between the certified
  man surface and the certified spec, using the closed `belongs-in-spec`
  ledger as its checklist.
- **`--reconcile`** exits 0 over the union of every spec letter's unit list:
  A's pilot 5 + I 11 + J 47 + K 45 + L 51 + M 11 = **170 units**, plus the
  consistency and lens runs.
- `cargo build`, `cargo test --bin mfb spec`, and `cargo test errorcode` green.

And the plan-125 closeout:

- **The whole-plan certificate**: units reviewed across all fourteen letters
  (897 `codex exec` runs), findings raised / confirmed / rejected, per
  surface and per iteration — the evidence for whether the three-iteration
  structure earned its cost, on both surfaces.
- **The tooling is left in a state the next author lands in**:
  `scripts/spec-census.sh`, `scripts/man-manual.sh`,
  `scripts/doc-review-fanout.sh` documented in `scripts/README.md`, and
  AGENTS.md pointing at `.ai/man-content.md` and `.ai/spec-content.md` for the
  two audiences.
- **Durable lessons into memory**, per AGENTS.md's auto-memory rules —
  lessons only, never plan status, never "done", no commit hashes.
- **All fourteen letters archived** to `planning/completed/`.

### Non-goals (explicit constraints)

- **Not a repair pass in disguise.** Findings are fixed here, but if they are
  numerous rather than residual, that is recorded as a result about iterations
  1–3.
- The man surface is not re-edited by this letter except where lens 6 finds a
  genuine contradiction — and then the fix goes to whichever side is wrong,
  with the man surface's probe evidence from letters C–F on the record.
- No renumbering or reordering of packages or topics.
- Compiler behavior is not changed; a defect found goes through `write-bug`.

## 2. Current State

Entering N, every spec file has been read three times at two granularities,
the citation surface is clean, and the man surface has been certified. What
has **not** happened: any single look at the whole specification at once, and
any comparison of the two finished surfaces against each other.

### Measured populations

| What | Count | Command |
|---|---|---|
| spec packages / files / lines | 12 / 146 / 26,482 | `src/docs/spec/mod.rs:PACKAGE_ORDER`; `find src/docs/spec -name '*.md' \| wc -l`; `cat $(find src/docs/spec -name '*.md') \| wc -w` → 223,085 words |
| broken symbol citations at the plan's start | **61** of 1,280 | plan-125-A §2 citation script, 2026-09-04 |
| malformed citations at the plan's start | 2 | same |
| spec units reviewed across A/I/J/K/L/M | **170** | 5 + 11 + 47 + 45 + 51 + 11 |
| man units reviewed across A/B/C/D/E/F/G | **699** | plan-125-H §2 |
| lens runs in this letter | 6 | plan-125-A §4.4 |
| `mfb spec --all` (global) | renders 0 lines | `./target/release/mfb spec --all \| wc -l` — the artifact is built from 12 per-package renderings |
| total `codex exec` runs across plan-125 | **897** | 869 unit runs + 16 consistency runs (B, G, I, M) + 12 final-lens runs (H, N) |

### Verified properties

- **There is no global `mfb spec --all`** — VERIFIED: it renders 0 lines,
  while `mfb spec language --all` renders 5,604. The whole-surface artifact
  must therefore be assembled from the 12 per-package renderings, in
  `PACKAGE_ORDER`. Whether that is itself a product gap worth fixing is an
  Open Decision below, not an assumption.
- **A lens can report clean without reading** — the same structural risk letter
  H recorded; the same mitigation applies (a zero-finding lens is not accepted
  without a main-thread spot-check).

## 3. Design Overview

Build the artifact, run the six lenses (plan-125-A §4.4) concurrently through
the harness, triage and fix on the main thread, re-run every sweep and paste
the outputs, then close the plan.

The six spec lenses, each one run over the whole artifact:

1. **Contract completeness** — an observable compiler contract with no home.
2. **Single source of truth** — duplicated or contradicting bodies.
3. **Citation integrity** — 0 `MISS-*`, and no claim left uncited that needed
   one.
4. **Accuracy at HEAD** — spot-verified claims, weighted to the topics that
   changed least during iterations 1–3 (a topic nobody touched is the one
   nobody checked hard).
5. **Reading order and cross-links** — `PACKAGE_ORDER`, the overviews' reading
   prose, `## See Also`, every link resolving.
6. **Man↔spec agreement** — the two certified surfaces read against each
   other, with the closed `belongs-in-spec` ledger as the checklist.

**Risk concentration:**
- **Lens 4's weighting.** The natural reading order would re-check the topics
  the plan worked hardest on, which are the least likely to be wrong.
  Weighting toward the *least-changed* topics is deliberate and is how this
  lens earns its run.
- **Lens 6 finding a contradiction and the wrong side being "fixed".** The man
  surface's claims carry probe evidence from letters C–F; the spec's carry
  citations. When they disagree, the evidence decides, and the deciding
  command is recorded.
- **Closing out with a known defect.** Held by the rule that a finding is
  fixed here or, if genuinely too large, written up as its own plan with the
  measurement attached (AGENTS.md: "too large = blocker on line 1 with
  repro") — never left as an unrecorded observation.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; `- [x] ~~text~~ — moot: <evidence>`
> rather than deleting; fill `Commit:` on landing. **Unticked means NOT DONE.**

### Phase 1 — build and verify the artifact

- [ ] Assemble the whole-spec artifact from the 12 `mfb spec <pkg> --all`
      renderings in `PACKAGE_ORDER`; confirm all 12 are present and none is
      empty.
- [ ] Confirm determinism: two runs produce byte-identical output.
- [ ] Reconcile the artifact against the 146-file census.
- [ ] Record the artifact's line count here.

Acceptance: the artifact is deterministic, contains all 12 packages, and
reconciles to 146 files.
Commit: —

### Phase 2 — the six lenses

- [ ] Run all six lens reviews over the artifact through the harness, with
      lens 4 weighted toward the least-changed topics and lens 6 given the
      certified man artifact and the closed `belongs-in-spec` ledger.
- [ ] Triage every finding: confirmed → fix; rejected → record with the
      disproving command.
- [ ] For any lens returning zero findings, perform and record a main-thread
      spot-check; a zero-finding lens is not accepted on its own word.
- [ ] Re-run each lens whose findings were fixed.

Acceptance: six lens runs `exit 0`; every finding has a verdict; every
rejection has a disproving command; every zero-finding lens has a recorded
spot-check; every man↔spec contradiction records which side the evidence
decided for.
Commit: —

### Phase 3 — the spec certification sweeps

- [ ] `./scripts/spec-census.sh --fill` → paste the per-package table.
- [ ] `./scripts/spec-census.sh --citations` → **0 `MISS-*`**; paste the
      output beside the 61/2 baseline.
- [ ] `./scripts/spec-census.sh --links` → clean; paste the output.
- [ ] `./scripts/spec-census.sh --render` → no leaked `[[` in any package;
      paste the output.
- [ ] `./scripts/spec-census.sh --fences` → every fence accounted for as
      checked or deliberately partial with a reason.
- [ ] `cargo build`, `cargo test --bin mfb spec`, `cargo test errorcode` →
      green; paste the results.
- [ ] `./scripts/doc-review-fanout.sh --reconcile` over the union of all spec
      letters' unit lists: **170 units**, 0 missing, 0 `FAILED`, 0 `DIRTY`.

Acceptance: every sweep above pasted with the command that produced it and at
its target; the citation sweep shows 0 against the recorded 61/2 baseline; the
170-unit reconciliation exits 0.
Commit: —

### Phase 4 — the plan-125 certificate

- [ ] Re-run the man certification sweeps from plan-125-H Phase 3 and confirm
      they still hold after the spec track's edits (lens 6 may have changed man
      prose): `--fill` 100%, `--scope` 0, `--memory-scope` 0 unclassified,
      `./scripts/man-manual.sh` at 621 pages.
- [ ] `--reconcile` over the union of **all fourteen letters'** unit lists:
      699 man + 170 spec = **869 units**, plus every consistency and lens run.
- [ ] Record the whole-plan result table: units, findings raised, confirmed,
      rejected — **per surface and per iteration** — and state plainly whether
      each iteration earned its cost, citing plan-125-G Phase 4 and plan-125-M
      Phase 4's seam/fact ratios.
- [ ] Record the Codex banner(s) used across the plan.
- [ ] List every bug filed via `write-bug` during plan-125, with its number
      and current state.

Acceptance: both surfaces' sweeps at target simultaneously; the 869-unit
reconciliation exits 0; the whole-plan result table is filled with measured
numbers, not impressions.
Commit: —

### Phase 5 — tooling, memory, and archive

- [ ] Document `scripts/spec-census.sh`, `scripts/man-manual.sh` and
      `scripts/doc-review-fanout.sh` in `scripts/README.md`.
- [ ] Confirm AGENTS.md's doc section names both `.ai/man-content.md` and
      `.ai/spec-content.md` with their audiences, and that
      `rg -n 'man-content|spec-content' AGENTS.md` returns both.
- [ ] Decide the fate of `planning/plan-125-findings/` and
      `planning/plan-125-prompts/`: the prompts are reusable and stay; the
      findings are a large intermediate artifact — keep or discard, decided
      and recorded.
- [ ] Memory sync per AGENTS.md: record **only durable, transferable
      lessons** (for example, if it holds: that citation rot concentrates
      where package bodies were split, and that a rendered-output census is
      the only honest doc denominator). **No plan status, no "done", no commit
      hashes, no letter state.** Delete or update any memory this plan proved
      wrong.
- [ ] Move all fourteen letters to `planning/completed/` (MOVE, never delete —
      memory `completed-plans-go-to-old-plans`), and remove the two
      intermediate ledger files if Phase 5 decided to discard them.
- [ ] Run `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run
      1.96.0 cargo fmt)` if any Rust changed across the plan (the optional
      `render_all_markdown` change, or any `write-bug` fix).

Acceptance: `ls planning/plan-125-*` → no matches; `ls
planning/completed/plan-125-*` → 14 files; `scripts/README.md` documents all
three new scripts; memory contains lessons and no status.
Commit: —

## Validation Plan

- Tests: `cargo build`, `cargo test --bin mfb spec`, `cargo test errorcode`;
  plus `cargo test --test cli_man_summary_plain` and
  `cargo test --test cli_canvas_man_examples_compile` if lens 6 changed man
  prose. `cargo fmt` per AGENTS.md if any Rust changed.
- Coverage check: `--reconcile` over 170 spec units in Phase 3 and 869 total
  units in Phase 4; the artifact reconciled to 146 files and 621 man pages.
- Runtime proof: the whole-spec artifact assembled deterministically; every
  sweep re-run and pasted.
- Doc sync: `scripts/README.md` and AGENTS.md updated; memory synced with
  lessons only.
- Acceptance: both surfaces' certification sweeps at target simultaneously,
  cargo gates green, 869 units reconciled, fourteen letters archived.

## Open Decisions

- **Should `mfb spec --all` (global) exist?** — Recommend **recording the gap
  rather than fixing it here**: plan-125 permits exactly one renderer change
  (the `mfb man --all` topic question in plan-125-A §4.1), and adding a second
  at the closeout is scope creep at the worst moment. Write it up with the
  measurement (`mfb spec --all` → 0 lines) so the next author has it. If the
  user wants it in-plan, it belongs in plan-125-A Phase 1, not here.
- **Keep or discard `planning/plan-125-findings/`?** — Recommend **discard
  after the certificate is written**: 897 reviewer transcripts are a large
  artifact whose conclusions have all been triaged into the letters' ledgers,
  and the ledgers are what a future reader needs. Keep the prompts.
- **A lens-6 contradiction where both sides are defensible** — recommend
  treating the man surface as authoritative about *observable behavior*
  (it was probe-verified in letters C–F) and the spec as authoritative about
  *the contract*, and making them agree by stating the observable behavior in
  both, with the contract's precision only in the spec.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

Two things happen here that could not happen anywhere else in the plan: the
whole specification is read at once, and the two finished surfaces are read
against each other. Everything else is arithmetic — 0 broken citations against
a baseline of 61, 869 units reconciled, both certificates holding at the same
moment — and the honest risk is a lens that reports clean without reading,
which is why a zero-finding lens costs a spot-check instead of a tick.
