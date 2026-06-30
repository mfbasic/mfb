# Planning

Substantial features get a written plan under `planning/` (the local planning
folder) before implementation begins. A plan is a design document that an
implementer (human or agent) can execute phase-by-phase without re-deriving the
design.

For a multi-phase plan, a phase is done only when its own stated acceptance
criterion is met and verified. A phase whose verification depends on a feature that
does not exist yet cannot be marked done.

Guidelines:

- Name the file `planning/plan-NN-shortname.md` (next free `NN`, two digits; short kebab-case slug). One plan per feature.
- Cross-link the embedded specs the plan touches near the top (`mfb spec memory`, `mfb spec language`, `mfb spec package`, `mfb spec diagnostics`, `mfb spec threading`, etc. — the canonical specification lives under `src/docs/spec/**`) so the implementer reads the right source of truth first.
- State the constraints the plan must **not** violate as explicit non-goals (language surface, value/copy/move semantics, layout/ABI, thread-transfer rules). A plan that silently changes one of these is wrong.
- Break the work into ordered, independently-landable phases. Put the lowest-risk, separately-valuable work first (e.g. an audit or a runtime primitive with no callers) and the highest-risk codegen last, behind tests.
- Fold the repository's standing requirements into the plan, don't restate them generically: every new/changed function needs `tests/func_<package>_<func>_valid/**` and `_invalid/**` with full overload coverage; runtime features need an execution proof, not just golden output; error-code or diagnostic changes must update the relevant embedded spec topics under `src/docs/spec/**` (notably `mfb spec diagnostics`, whose `error-codes` table is the build input for `errorCode::`); acceptance (`scripts/test-accept.sh`) must pass.
- Record genuinely open design choices in an "Open Decisions" section with a recommendation for each — don't bury unresolved forks inside prose.
- When a plan is fully implemented, remove the plan doc in the same commit that lands the final phase (precedent: `34e526c9` removed plan-05 on completion). Keep `Last updated` current while it lives.

## Plan template

Use the template at [`plan_template.md`](plan_template.md): copy it to
`planning/plan-NN-shortname.md` and fill it in. It covers goal/non-goals,
current state, design, layout/ABI impact, phases, validation plan, open decisions,
and summary.
