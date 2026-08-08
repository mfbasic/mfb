* Use the cheapest Anthropic model that can reliably resolve the uncertainty in the task. Escalate based on ambiguity, correctness risk, and autonomy—not lines changed.

| Task | Model | Effort | Escalate when |
| --- | --- | --- | --- |
| Read + grep in parallel, summarize back | Haiku 4.5 | low | subagent returns "unclear / conflicting" → re-dispatch on Sonnet |
| Pure pattern rewrites across an explicit file list | **codemod / sed** (no LLM) | — | pattern has exceptions → Sonnet |
| Edits across many files that need per-file judgment | Sonnet 5 | low–med | file doesn't match the assumed shape → flag, don't guess |
| "Which of these is authoritative" call | Sonnet 5 | med | sources genuinely conflict → Opus |
| Multi-step search where the match isn't obvious | Sonnet 5 | med | N tries fail to converge → Opus |
| Writing / adjusting tests + goldens for a known change | Sonnet 5 | med | — |
| Bug-NN: reproduce, RED test, fix, full-suite gate | Sonnet 5 | med | repro fails, or bug is concurrency / ABI / cross-module state → Opus |
| Code review that must catch correctness bugs | Opus 5 | med | — (single pass, asymmetric cost — pay for it) |
| Planning an implementation across the codebase | Opus 5 | high | — |
| Following a plan-NN end to end (autonomous commits) | Opus 5 plan → Sonnet 5 steps | high / med | a step's diff surprises the plan → Opus takes the step |
| Hard debugging: codegen, native ABI, register lifetimes | Opus 5 | high | — |