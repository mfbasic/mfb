# Agent Instructions

## Implementation Quality

When asked to implement a change, deliver production-ready, valid code.

Guidelines:

- Do not provide stubs, placeholders, mock implementations, or proof-of-concept code unless explicitly requested.
- Do not take shortcuts that leave behavior incomplete, unvalidated, or unsuitable for production use.
- Implement the complete requested behavior, including necessary error handling, integration points, and tests or validation.
- If a production-ready implementation is blocked by missing requirements, dependencies, or external access, state the blocker clearly and avoid filling the gap with non-functional code.

## Validation

After completing any code or golden-output change, the acceptance suite must pass.

Guidelines:

- Run `scripts/test-accept.sh target/debug/mfb target/accept-actual` after compiler work or any change that can affect generated AST, IR, bytecode, native binaries, or diagnostics.
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
