# Bugs

When you find a bug while doing a task (it doesn't matter whether the bug is
related to the task you set out to do), don't just patch it silently — capture it
as a reproducible fix.

First decide whether the fix is small-ish: a contained, low-risk change that you
can land in this session without re-deriving a design.

## Small-ish bug → fix it now

1. **Write a test first.** Add a `tests/*` case that reproduces the bug and fails
   for the current behavior. Follow the existing test conventions for the area
   (e.g. `tests/func_<package>_<func>_valid/**` and `_invalid/**`, validation and
   runtime-execution proofs — see `.ai/compiler.md`).
2. **Fix the bug** so the new test passes, and confirm the rest of the suite
   (`scripts/test-accept.sh`) still passes. The failing-then-passing test is the
   proof the fix works.

Don't fold an unrelated bug fix into an unrelated commit — keep it itemized.

## Not small-ish → auto-create a bug file

If the fix is large, risky, or touches semantics/layout/ABI/codegen broadly,
don't attempt it inline. Capture it instead as a **bug file** under
`planning/bug-NN-shortname.md` — the dedicated bug namespace, parallel to
`plan-NN` (use the next free `NN` in the `bug-` series; the two series number
independently). Author it from the plan template (`.ai/plan_template.md`,
adapted: a determinism/correctness fix has no new language surface), per
`.ai/planning.md`.

Always create the file as soon as the bug is found and triaged as not-small-ish
— do not wait until you start the fix. It must describe:

- the single correct behavior a fix produces (the goal),
- a failing reproduction (the exact command/output, e.g. a `tests/*` fixture or
  a `scripts/codegen-selfdiff.sh` failure),
- the root cause cited to `file.rs:line` if known,
- the non-goals (what must NOT change — layout/ABI/semantics), and
- the phased fix (test-first), with a golden-regeneration/audit step when
  codegen output shifts.

Then land the test + fix through that bug file. If a memory note tracks the bug,
cross-link it. Example: `planning/bug-01-resource-union-drop.md` (resource-union
drop dispatch order is non-deterministic — `variants_for_union` iterates a
`HashMap`).
