# Planning

Substantial features get a written plan under `planning/` (the local planning
folder) before implementation begins. A plan is a design document that an
implementer (human or agent) can execute phase-by-phase without re-deriving the
design.

For a multi-phase plan, a phase is done only when its own stated acceptance
criterion is met and verified. A phase whose verification depends on a feature that
does not exist yet cannot be marked done.

Guidelines:

- Estimate each plan's effort and record it in the template's `Effort:` field (see "Plan template"), using this scale:
  - **small** — < 1h
  - **medium** — 1h–2h
  - **large** — 3h–1d
  - **x-large** — 1d–3d
  - **huge** — > 3d
- Name the file `planning/plan-NN-shortname.md` (next free `NN`, two digits; short kebab-case slug). One plan per feature. A **small** or **medium** plan stays a single file.
- **Split large, x-large, and huge plans by effort into small/medium sub-plans.** Give each sub-plan its own whole planning document under a shared `NN`, suffixed with a capital letter (`planning/plan-NN-A-shortname.md`, `planning/plan-NN-B-shortname.md`, … `planning/plan-NN-Z-shortname.md`; precedent `plan-00-A` … `plan-00-H`). Split by **effort, not by phase**: group the work so each lettered sub-plan is itself small or medium — a sub-plan is a unit someone lands in one sitting, which may bundle several small phases together, or (for one heavy phase) split a phase across sub-plans. Each sub-plan is a complete plan in its own right (goal, current state, design, tasks, acceptance) and names the sub-plan(s) it depends on. **Section A additionally records `Overall Effort`** (the size of the whole `NN` feature — necessarily large/x-large/huge, since that is why it was split) alongside its own `Effort`. A large/x-large/huge plan that is still a single file has not been split yet — split it before implementation.
- Cross-link the embedded specs the plan touches near the top (`mfb spec memory`, `mfb spec language`, `mfb spec package`, `mfb spec diagnostics`, `mfb spec threading`, etc. — the canonical specification lives under `src/docs/spec/**`) so the implementer reads the right source of truth first.
- State the constraints the plan must **not** violate as explicit non-goals (language surface, value/copy/move semantics, layout/ABI, thread-transfer rules). A plan that silently changes one of these is wrong.
- Break the work into ordered, independently-landable phases. Put the lowest-risk, separately-valuable work first (e.g. an audit or a runtime primitive with no callers) and the highest-risk codegen last, behind tests. Each phase must list the concrete tasks to do — each task a single, checkable unit of work naming the file(s) it touches — plus the acceptance criterion that must be met and verified before the phase is done. An implementer should be able to work the checklist without re-deriving the design.
- Fold the repository's standing requirements into the plan, don't restate them generically: every new/changed function needs `tests/func_<package>_<func>_valid/**` and `_invalid/**` with full overload coverage; runtime features need an execution proof, not just golden output; error-code or diagnostic changes must update the relevant embedded spec topics under `src/docs/spec/**` (notably `mfb spec diagnostics`, whose `error-codes` table is the build input for `errorCode::`); acceptance (`scripts/test-accept.sh`) must pass.
- Record genuinely open design choices in an "Open Decisions" section with a recommendation for each — don't bury unresolved forks inside prose.
- When a phase lands, record the commit hash(es) that implemented it next to that phase in the plan (the template's per-phase `Commit:` line). This is the running ledger of what has actually shipped — keep it current as you go, so anyone reading the plan can see which phases are done and where to find the change.
- When a plan is fully implemented, remove the plan doc in the same commit that lands the final phase (precedent: `34e526c9` removed plan-05 on completion). Keep `Last updated` current while it lives. For a split plan, remove each sub-plan document (`plan-NN-A`, `plan-NN-B`, …) as it lands; the whole `NN` set is gone once the last letter is done. (`planning/` itself is gitignored, so this is bookkeeping discipline rather than a tracked deletion — but the ledger and cleanup still matter for anyone reading the plan mid-flight.)

## Plan template

Use the template at [`plan_template.md`](plan_template.md): copy it to
`planning/plan-NN-shortname.md` and fill it in. It covers goal/non-goals,
current state, design, layout/ABI impact, phases, validation plan, open decisions,
and summary.

For a split plan (see above), copy the template once per sub-plan to
`planning/plan-NN-A-shortname.md`, `planning/plan-NN-B-shortname.md`, … — each a
complete plan sized to small/medium effort. A sub-plan's `## Phases` section
enumerates the phases and concrete tasks within that sub-plan; sub-plan A also
carries `Overall Effort` for the whole `NN` feature.
