# plan-125-H: the final whole-manual developer-doc consistency check

Last updated: 2026-09-04
Effort: large (3h–1d)
Depends on: plan-125-G (all three iterations complete across the whole man
surface; G declares the surface final and this letter certifies it).

The last look at the man surface, and the only one taken over **the entire
developer manual at once**. Six lenses, each one `codex exec` run over the
complete artifact with exactly one question, because no single run reads
51,540 lines usefully with six questions at the same time.

This is a **certification** letter in plan-108-F's sense: the certificate is a
measured, re-runnable sweep pasted into this file with its commands, never an
assembly of ticked boxes from earlier letters (memory
`completeness-claims-need-an-audit`). Anything it finds is fixed here — the
plan does not exit with a known defect recorded as future work.

References:

- plan-125-A §4.1 (why the artifact is `scripts/man-manual.sh` and not raw
  `mfb man --all`), §4.4 (the six lenses), §5 (the man-final prompt).
- plan-108-F — the certification pattern, and its two recorded blind spots:
  a grep-based vocabulary sweep is a floor, not a ceiling; and "passing by
  deleting a true contract" is the failure mode a read-through exists to catch.
- plan-125-B Phase 4 (the cross-package consistency table), plan-125-F Phase 1
  (the handle-contract table), plan-125-G Phase 1 (the terminology table as
  re-verified) — this letter checks all three survived.
- `planning/plan-125-belongs-in-spec.md` — closed out here and handed to
  letter I.

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-G complete | `grep -c '^- \[ \]' planning/plan-125-G-man-iter3-packages.md` → `0` | — |
| the complete-manual artifact builds | `./scripts/man-manual.sh \| wc -l` → non-zero, and its page count equals the census's 621 | — |

## 1. Goal

The certification, pasted into this file with each command's output:

- **Coverage** — the artifact contains every one of the **621** man pages:
  all 31 package overviews, all 538 function pages, all 20 types pages, all 32
  guide pages, *including* the `testing` and `general` pages that
  `mfb man --all` deliberately omits.
- **Fill** — `./scripts/man-census.sh --fill` → 100% on intro / desc /
  example / param-desc; every overview and types description non-empty.
- **Internals scope** — `./scripts/man-census.sh --scope` → 0, and a
  read-through by lens 3 finding no sentence that requires a compiler mental
  model even where no banned word appears.
- **Memory vocabulary** — `--memory-scope` → 0 unclassified, with only the
  two sanctioned carve-outs, **and** a read-through confirming no true handle
  contract was deleted to pass the grep.
- **Terminology** — one concept, one spelling, across the whole artifact;
  plan-125-B/G's table verified against it entry by entry.
- **Handle contract** — the seven resource packages agree, verified against
  plan-125-F's table.
- **Examples** — the union of all letters' example ledgers accounts for every
  example on every one of the 621 pages plus every guide code block: **0
  unaccounted**.
- **Cross-links** — every `mfb man X` reference in the rendered prose resolves
  to a real package, function or topic.
- **Citations** — no `[[` markers in rendered output other than MFBASIC nested
  list literals (5 at baseline, each read and classified).
- **`--reconcile`** exits 0 over the union of every man letter's unit list:
  A's pilot 31 + B 39 + C 150 + D 142 + E 142 + F 156 + G 39 = **699 units**,
  plus the consistency and lens runs.
- **`planning/plan-125-belongs-in-spec.md` is closed**: every entry carries
  the spec package it belongs to, and the file is handed to letter I as its
  coverage checklist.

### Non-goals (explicit constraints)

- Per plan-125-A: no compiler test gates for man prose; prose and markdown
  only; the reviewer never commits.
- **Not a repair pass in disguise.** A finding here is fixed here, but if this
  letter's findings are numerous rather than residual, that is recorded as a
  result about iterations 1–3 — and the fix still lands.
- No wording churn.
- The spec surface is untouched by this letter; every spec observation goes to
  the ledger for letters I–N.

## 2. Current State

Entering H, every man page has been read three times at two granularities and
every example has been compiled and run. What has **not** happened: any single
look at the whole manual at once. Letters B and G ran consistency reviews over
a *condensed* artifact (overviews + types pages + topic overviews); no run has
ever seen the function pages of two different packages side by side.

### Measured populations

| What | Count | Command |
|---|---|---|
| pages in the complete artifact | 621 | `./scripts/man-census.sh --fill` (538 + 31 + 20) + 32 guide pages |
| `mfb man --all` lines at plan-writing | 51,540 | `./target/release/mfb man --all \| wc -l` |
| pages `mfb man --all` omits | 62 | `testing` 12 + `general` 18 + 32 guide pages — `render_all_markdown` filters `is_unqualified_global()` and walks the registry only |
| man units reviewed across A–G | 699 | 31 + 39 + 150 + 142 + 142 + 156 + 39 |
| lens runs in this letter | 6 | plan-125-A §4.4 |
| leaked `[[` at plan-writing | 5, all nested-list literals | `./target/release/mfb man --all \| grep -n '\[\['` — all five read |

### Verified properties

- **Raw `mfb man --all` is not a valid certification artifact** — VERIFIED by
  reading `src/cli/man.rs:render_all_markdown` and by grep (`TESTING`/`GENERAL`
  headers absent, no guide-topic body text present). Certifying against it
  would certify 90% of the surface while claiming all of it. This is why
  plan-125-A Phase 1 built `scripts/man-manual.sh`.
- **A vocabulary grep cannot see the worst leak** — VERIFIED as plan-108-F's
  own recorded blind spot: a page can teach a borrow/ownership mental model
  with no banned word in it. Lens 3 and lens 6 are read-throughs for exactly
  that, and the grep is the floor beneath them.

## 3. Design Overview

Build the artifact, run the six lenses (plan-125-A §4.4) concurrently through
the harness, triage and fix on the main thread, then re-run every sweep and
paste the outputs.

The six lenses, each one run over the whole artifact:

1. **Terminology** — is one concept spelled one way everywhere?
2. **Example style** — do the 621 pages' examples look like one manual?
3. **Audience and scope** — any sentence requiring a compiler mental model,
   including ones no grep finds.
4. **Error documentation** — is every failure a developer can hit documented,
   and consistently across siblings?
5. **Cross-links and discoverability** — every `mfb man X` reference resolves;
   a developer can find the right page from the index.
6. **Memory vocabulary** — the ban at 0 unclassified, *and* the
   deleted-contract check.

**Risk concentration:**
- **A lens that reports "clean" because it did not really read the
  artifact.** The single most likely false pass in this letter. Held by
  requiring each lens run to cite specific page locations for at least its
  strongest observations, by the manifest's findings-count column, and by not
  accepting a zero-finding lens without a spot-check the main thread performs
  itself.
- **A fix in one place breaking consistency in another** — every fix landing
  in this letter is followed by re-running the lens that motivated it.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; `- [x] ~~text~~ — moot: <evidence>`
> rather than deleting; fill `Commit:` on landing. **Unticked means NOT DONE.**

### Phase 1 — build and verify the artifact

- [ ] Run `./scripts/man-manual.sh` and confirm the artifact's page count
      equals **621** by counting rendered page headers, reconciled against
      `./scripts/man-census.sh --fill` plus the 32 guide pages.
- [ ] Confirm `testing`, `general` and all 10 guide topics are present.
- [ ] Confirm determinism: two runs produce byte-identical output.
- [ ] Record the artifact's line count here.

Acceptance: the artifact is deterministic and its page count equals the census
denominator exactly; a missing page is a defect in `man-manual.sh`, fixed
here.
Commit: —

### Phase 2 — the six lenses

- [ ] Run all six lens reviews over the artifact through the harness.
- [ ] Triage every finding on the main thread: confirmed → fix; rejected →
      record with the disproving command.
- [ ] For any lens returning zero findings, perform and record a main-thread
      spot-check of that dimension; a zero-finding lens is not accepted on its
      own word.
- [ ] Re-run each lens whose findings were fixed.

Acceptance: six lens runs in the manifest with `exit 0`; every finding has a
verdict; every rejection has a disproving command; every zero-finding lens has
a recorded spot-check.
Commit: —

### Phase 3 — the certification sweeps

- [ ] `./scripts/man-census.sh --fill` → 100%; paste the output.
- [ ] `./scripts/man-census.sh --scope` → 0; paste the output.
- [ ] `./scripts/man-census.sh --memory-scope` → 0 unclassified, carve-outs
      only; paste the output.
- [ ] Cross-link resolution over the artifact: every `mfb man X` reference
      resolves; paste the command and the count.
- [ ] `[[` sweep over the artifact: every hit read and classified.
- [ ] The example-ledger reconciliation: union of A/C/D/E/F/G ledgers against
      the 621 pages plus guide code blocks → **0 unaccounted**; paste the
      count.
- [ ] The permitted-vocabulary read-through: `mfb man tcp accept`,
      `udp receive`, `tls accept`, `process spawn`, `audio play`, `fs open`,
      `io openFile` read end to end and recorded — plan-108-F's sharpest test
      of whether a true contract was deleted to pass a grep.
- [ ] `./scripts/doc-review-fanout.sh --reconcile` over the union of all man
      letters' unit lists: **699 units**, 0 missing, 0 `FAILED`, 0 `DIRTY`.

Acceptance: every sweep above is pasted into this file with the command that
produced it and is at its target; the 699-unit reconciliation exits 0.
Commit: —

### Phase 4 — close the man surface and hand off

- [ ] Close `planning/plan-125-belongs-in-spec.md`: every entry has the spec
      package it belongs to; record the total count and the per-package
      breakdown here — this is letter I's coverage checklist.
- [ ] Record the Codex banner used across the man letters
      (`codex --version` and the reported model).
- [ ] Record the man-surface result: units reviewed, findings raised,
      confirmed, rejected, per iteration — the evidence for whether the
      three-iteration structure earned its cost.
- [ ] Memory sync: record only durable lessons learned (never plan status,
      never "done") per AGENTS.md's auto-memory rules.
- [ ] Run any pinned-text test touched across A–H:
      `cargo test --test cli_man_summary_plain` and
      `cargo test --test cli_canvas_man_examples_compile`.

Acceptance: the ledger is closed with a per-package breakdown; the result
table is filled with measured numbers; both pinned-text tests green.
Commit: —

## Validation Plan

- Tests: `cargo test --test cli_man_summary_plain` and
  `cargo test --test cli_canvas_man_examples_compile` — the only two tests
  that see man output. No other compiler gate applies (plan-125-A Non-goals).
- Coverage check: `--reconcile` over 699 units; the example-ledger
  reconciliation to 0 unaccounted; the artifact's page count to 621.
- Runtime proof: `./scripts/man-manual.sh` renders the complete manual
  deterministically; the seven permitted-vocabulary pages read end to end.
- Doc sync: `planning/plan-125-belongs-in-spec.md` closed and handed to
  letter I.
- Acceptance: all Phase 3 sweeps at target, pasted with their commands.

## Open Decisions

- **What if a lens finds a structural problem too large to fix here** (for
  example, that error documentation is systematically thin across a whole
  family)? — Recommend fixing it in this letter if it is bounded, and if it is
  not, writing it up as its own plan with the measurement attached, per
  AGENTS.md ("too large = blocker on line 1 with repro"). It is not left as an
  unrecorded observation, and it does not silently become letter I's problem.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

The one thing that can go wrong here is a lens that reports clean without
having read the artifact, which would certify the whole man surface on six
sentences of nothing; the zero-finding spot-check rule exists solely for that.
Everything else in this letter is arithmetic: 621 pages in the artifact, 699
units reconciled, 0 unaccounted examples, and the sweeps pasted with the
commands that produced them.
