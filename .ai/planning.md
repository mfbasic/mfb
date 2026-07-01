# Planning

Substantial features get a written plan under `planning/` (the local planning
folder) before implementation begins. A plan is a design document that an
implementer (human or agent) can execute phase-by-phase without re-deriving the
design.

For a multi-phase plan, a phase is done only when its own stated acceptance
criterion is met and verified. A phase whose verification depends on a feature that
does not exist yet cannot be marked done.

Guidelines:

- Name the file `planning/plan-NN-shortname.md` (next free `NN`, two digits; short kebab-case slug). One plan per feature. This single-file form is for **small plans** (roughly one to three phases that a single implementer lands in one sitting).
- **Medium and larger plans split by phase, one document per phase.** When a plan is medium or large — many phases, or phases substantial enough that one implementer (or agent) would own each — give every phase its own whole planning document under a shared `NN`, suffixed with a capital letter in phase order: `planning/plan-NN-A-shortname.md`, `planning/plan-NN-B-shortname.md`, … `planning/plan-NN-Z-shortname.md` (precedent: `plan-00-A` … `plan-00-H`). All share the same `NN`; the letter is the phase. Each per-phase document is a complete plan in its own right (goal, current state, design, tasks, acceptance) scoped to that phase, and names the phase document(s) it depends on. Use your judgement on where the small/medium boundary falls; when a single-file plan grows past ~three phases or any phase balloons, auto-split it into the lettered form rather than letting one document sprawl.
- Cross-link the embedded specs the plan touches near the top (`mfb spec memory`, `mfb spec language`, `mfb spec package`, `mfb spec diagnostics`, `mfb spec threading`, etc. — the canonical specification lives under `src/docs/spec/**`) so the implementer reads the right source of truth first.
- State the constraints the plan must **not** violate as explicit non-goals (language surface, value/copy/move semantics, layout/ABI, thread-transfer rules). A plan that silently changes one of these is wrong.
- Break the work into ordered, independently-landable phases. Put the lowest-risk, separately-valuable work first (e.g. an audit or a runtime primitive with no callers) and the highest-risk codegen last, behind tests. Each phase must list the concrete tasks to do — each task a single, checkable unit of work naming the file(s) it touches — plus the acceptance criterion that must be met and verified before the phase is done. An implementer should be able to work the checklist without re-deriving the design.
- Fold the repository's standing requirements into the plan, don't restate them generically: every new/changed function needs `tests/func_<package>_<func>_valid/**` and `_invalid/**` with full overload coverage; runtime features need an execution proof, not just golden output; error-code or diagnostic changes must update the relevant embedded spec topics under `src/docs/spec/**` (notably `mfb spec diagnostics`, whose `error-codes` table is the build input for `errorCode::`); acceptance (`scripts/test-accept.sh`) must pass.
- Record genuinely open design choices in an "Open Decisions" section with a recommendation for each — don't bury unresolved forks inside prose.
- When a phase lands, record the commit hash(es) that implemented it next to that phase in the plan (the template's per-phase `Commit:` line). This is the running ledger of what has actually shipped — keep it current as you go, so anyone reading the plan can see which phases are done and where to find the change.
- When a plan is fully implemented, remove the plan doc in the same commit that lands the final phase (precedent: `34e526c9` removed plan-05 on completion). Keep `Last updated` current while it lives. For a split plan, remove each per-phase document (`plan-NN-A`, `plan-NN-B`, …) as its phase lands; the whole `NN` set is gone once the last letter is done. (`planning/` itself is gitignored, so this is bookkeeping discipline rather than a tracked deletion — but the ledger and cleanup still matter for anyone reading the plan mid-flight.)

## Plan template

Use the template at [`plan_template.md`](plan_template.md): copy it to
`planning/plan-NN-shortname.md` and fill it in. It covers goal/non-goals,
current state, design, layout/ABI impact, phases, validation plan, open decisions,
and summary.

For a split plan (see above), copy the template once per phase to
`planning/plan-NN-A-shortname.md`, `planning/plan-NN-B-shortname.md`, … — each a
complete, phase-scoped plan. The `## Phases` section of a per-phase document
enumerates the concrete tasks for that phase only.
