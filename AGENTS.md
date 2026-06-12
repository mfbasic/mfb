# Agent Instructions

## Implementation Quality

When asked to implement a change, deliver production-ready, valid code.

Guidelines:

- Do not provide stubs, placeholders, mock implementations, or proof-of-concept code unless explicitly requested.
- Do not take shortcuts that leave behavior incomplete, unvalidated, or unsuitable for production use.
- Implement the complete requested behavior, including necessary error handling, integration points, and tests or validation.
- If a production-ready implementation is blocked by missing requirements, dependencies, or external access, state the blocker clearly and avoid filling the gap with non-functional code.

## Hard Completion Gate

A task is not complete unless the requested behavior works at runtime.

Compilation success, AST/IR/bytecode golden output, package generation, or native binary generation is not sufficient proof of completion unless the user explicitly asked only for compiler output.

Guidelines:

- Before changing built-ins, bytecode, native code generation, runtime helpers, package behavior, or diagnostics, inspect and report any existing stub, placeholder, default-result, `todo`, `unimplemented`, or unsupported paths related to the task.
- Do not add new stub, placeholder, default-result, mock, or proof-of-concept behavior unless explicitly requested.
- Do not route unimplemented behavior to zero values, empty strings, empty collections, `Nothing`, default records, or other fallback values that make unsupported behavior appear successful.
- Do not describe a change as production-ready, complete, fully supported, or done while any part of the requested behavior is stubbed, defaulted, mocked, unreachable, unsupported, or only represented in AST, IR, bytecode, metadata, or generated native output.
- For runtime features, add or run a runtime validation that executes the generated program and proves the requested behavior through exit code, stdout/stderr, file output, or another observable result.
- If runtime behavior cannot be implemented or validated, state that the task is blocked or incomplete and do not present compiler plumbing or golden output as functional support.
- Treat any backend helper named like `*_default_result`, or any backend path that stores default values for a built-in operation, as unsupported unless a runtime test proves the actual requested behavior.

## Validation

After completing any code or golden-output change, the acceptance suite must pass.

Guidelines:

- For every function created or modified, automatically create or update matching tests under both `tests/func_<package>_<func>_valid/**` and `tests/func_<package>_<func>_invalid/**`.
- Function test directories are mandatory and non-skippable. Do not omit them because a change seems small, internal, obvious, already covered indirectly, or difficult to exercise.
- Function tests must cover every overload of the created or modified function. If an overload cannot be tested, the task is incomplete until the blocker is resolved or explicitly accepted by the user.
- Valid function tests must prove each overload succeeds with representative runtime behavior or observable compiler behavior appropriate to the function.
- Invalid function tests must prove each overload rejects incorrect usage, including wrong argument count, wrong argument type, invalid receiver/context, and relevant boundary or error cases.
- Do not describe a function change as complete while either the valid or invalid function test directory is missing, empty, skipped, or lacking overload coverage.
- Run `scripts/test-accept.sh target/debug/mfb target/accept-actual` after compiler work or any change that can affect generated AST, IR, bytecode, native binaries, or diagnostics.
- Acceptance passing is required but not sufficient for runtime behavior changes. For runtime features, also add or run an execution test that proves the generated program behaves correctly.
- If acceptance fails, verify whether each failure is caused by the compiler update, a stale expected-output fixture, or a real regression before fixing code or updating goldens.
- Do not assume an acceptance mismatch is a test issue. Prove stale goldens by comparing deterministic regenerated output, and when necessary compare against a clean pre-change checkout.
- Do not leave acceptance failing at the end of the task unless an external blocker makes the suite impossible to run; report that blocker and the exact command/output.

## Commits

When creating commits in this repository, use detailed itemized commit messages.

Commit message format:

```text
Short imperative summary

- Describe one concrete change.
- Describe another concrete change.
- Note validation, tests, or generated files when relevant.
```

Guidelines:

- Keep the subject line concise and imperative.
- Use bullet points in the commit body for all non-trivial commits.
- Mention user-facing behavior changes separately from internal refactors.
- Mention validation commands when they were run.
- Do not include unrelated dirty worktree changes in a commit.
