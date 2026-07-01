<!-- MFBASIC plan template. Referenced by .ai/planning.md. Copy this to
     planning/plan-NN-shortname.md (the local planning folder) and fill it in. -->

# MFBASIC <Feature> Plan

Last updated: YYYY-MM-DD
Effort: <small (<1h) | medium (1h–2h) | large (3h–1d) | x-large (1d–3d) | huge (>3d)>

<!-- Effort drives the split rule (.ai/planning.md): small/medium stays one file;
     large/x-large/huge splits by effort into small/medium sub-plans
     plan-NN-A/B/… For a split sub-plan, use this header instead:
       # plan-NN-A: <Sub-plan name>
       Last updated: YYYY-MM-DD
       Overall Effort: <size of the whole plan-NN feature>   (section A only)
       Effort: <size of this sub-plan — small or medium>
     Sub-plans B, C, … carry only their own `Effort:`. -->

<One or two paragraphs: what this builds and why. State the single
behavioral outcome a correct implementation produces.>

It complements:

- `./mfb spec <package> <topic>` (<what this plan touches there>; the canonical specs live under `src/docs/spec/**`)

## 1. Goal

- <Concrete, checkable outcome.>

### Non-goals (explicit constraints)

<What must NOT change: language surface, value/copy/move/freeze semantics,
layout/ABI, thread-transfer rules. Be specific — these are the guardrails.>

## 2. Current State

<How it works today, cited to files/specs (`file.rs:line`,
`src/docs/spec/<package>/<topic>.md` / `mfb spec <package> <topic>`).
Name existing precedents the design will mirror.>

## 3. Design Overview

<The shape of the solution: independent pieces and how they layer. Call out
where the correctness risk concentrates.>

## 4..N. Detailed Design

<One section per piece. Algorithms, data layout, the runtime/codegen split.>

## Layout / ABI Impact

<Exactly what changes in `mfb spec memory` / `mfb spec package` (the topics
under `src/docs/spec/**`), and — just as important — what stays unchanged so
copy/transfer/golden output is unaffected. Omit if the plan touches no layout.>

## Phases

<Ordered, independently-landable phases. Lowest-risk / separately-valuable
work first (e.g. an audit or a runtime primitive with no callers); highest-risk
codegen last, behind tests. Each phase lists the concrete tasks to do — a task
is a single, checkable unit of work naming the file(s) it touches — and the
acceptance criterion that must be met and verified before the phase is done.
When a phase lands, fill in its `Commit:` line with the hash(es) that shipped
it — the running ledger of what is actually done.>

### Phase 1 — <short name>

<One line: what this phase delivers and why it is safe to land alone.>

- [ ] <Concrete task — what to do and the file(s) it touches (`file.rs:line`).>
- [ ] <Concrete task.>
- [ ] Tests: `tests/func_<pkg>_<func>_valid/**` and `_invalid/**` for anything added/changed here.

Acceptance: <the specific, checkable outcome that proves this phase is done —
tests/goldens/runtime proof, not "code compiles".>
Commit: <hash(es) once landed, or `—` while pending.>

### Phase 2 — <short name>

<One line.>

- [ ] <Concrete task.>
- [ ] <Concrete task.>

Acceptance: <checkable outcome.>
Commit: <hash(es) once landed, or `—` while pending.>

### Phase N — <short name> (highest-risk codegen last)

<One line.>

- [ ] <Concrete task.>

Acceptance: <checkable outcome.>
Commit: <hash(es) once landed, or `—` while pending.>

## Validation Plan

- Function tests: `tests/func_<pkg>_<func>_valid/**` and `_invalid/**`,
  every overload.
- Runtime proof: <the program + observable result that proves real behavior>.
- Doc sync: <updates to the relevant `src/docs/spec/**` topics, e.g. `mfb spec diagnostics` / `language` / `package`>.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- <Decision> — <recommended option> vs. <alternative>. (§ref)

## Non-Goals

- <Explicitly out of scope for this plan / V1.>

## Summary

<Where the real engineering risk is, and what is left untouched.>
