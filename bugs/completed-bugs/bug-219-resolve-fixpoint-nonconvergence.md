# bug-219: dependency resolver falls through on non-convergence and writes an unstable lock (comment claims it errors)

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: correctness

Status: Fixed (2026-07-15) — the dependency resolver now tracks a `converged` flag and, if the bounded fixpoint loop exhausts its passes without stabilizing, returns an error (oscillating registry graph) instead of falling through and assembling an mfb.lock from the last unstable selection. The comment now matches the behavior.

`resolve` (`src/cli/resolve.rs:219-262`) runs a bounded fixpoint whose comment
says it "errors instead of spinning," but on non-convergence the loop simply
falls through and assembles a lock from the last (possibly unstable) selection —
no error is raised. The final `select_node` pass only catches diamond/pin
conflicts, not oscillation.

Trigger: a registry dependency graph whose import edges oscillate the selection
past `len^2+4` iterations → `resolve` writes an `mfb.lock` reflecting a
non-converged/inconsistent selection instead of failing.

Fix: after the loop, re-run one pass and error if any node's selection still
changes (or track `changed` on the last iteration and reject); at minimum correct
the comment.
