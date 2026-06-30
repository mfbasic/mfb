<!-- MFBASIC plan template. Referenced by .ai/planning.md. Copy this to
     planning/plan-NN-shortname.md (the local planning folder) and fill it in. -->

# MFBASIC <Feature> Plan

Last updated: YYYY-MM-DD

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

1. <Lowest-risk, independently-landable first.>
2. ...
N. <Highest-risk codegen last, behind tests.>

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
