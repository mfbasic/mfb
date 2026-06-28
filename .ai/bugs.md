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

## Not small-ish → write a plan

If the fix is large, risky, or touches semantics/layout/ABI/codegen broadly,
don't attempt it inline. Create a plan under `planning/plan-NN-shortname.md`
(next free `NN`) per `.ai/planning.md`, describing the bug, a failing reproduction,
the root cause if known, and the phased fix. Land the test + fix through that plan.
