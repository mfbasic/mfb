# Agent Instructions

## "Done" and "Verify" (read this first)

**"Done" means every part of the requested work is finished and verified — the
whole thing, not the easy part, not most of it.** If any part is unfinished,
unverified, stubbed, deferred, or depends on something not yet built, the work is
**NOT done**.

**"Verify" means proving the actual goal is true** — confirming the real property
the work was supposed to achieve, with evidence that maps directly to that goal.
Running the tests and seeing them pass is **NOT** verifying. Passing tests only
prove what those specific tests check; they are a proxy, not the goal. To verify,
you must check the real requirement itself, including the parts no existing test
covers. If you cannot point to the specific evidence that the actual goal holds,
you have not verified it — and you must say so.

When asked "is it done / complete / finished / verified":

- Answer **yes** or **no** on the first line. Nothing else first.
- Answer **yes** only after you have *verified* (per above) that all of it is
  finished. When unsure, the answer is **no**.
- If **no**: add one short line naming what is left. Do not produce a status
  report, an evidence dump, or a summary of what you did — unless explicitly
  asked.
- Never report work as "done" because compilation succeeded, acceptance/tests are
  green, or goldens match. Those answer a different question than "is it done."
- For a multi-phase plan, a phase is done only when its own stated acceptance
  criterion is met and verified. A phase whose verification depends on a feature
  that does not exist yet cannot be marked done.

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
- Do not add defensive `unsupported`, `unimplemented`, `todo`, or generic error paths for requested behavior as a substitute for real implementation. Missing support must be treated as a blocker, not hidden behind a catch-all branch.
- Do not satisfy compiler exhaustiveness or runtime dispatch by adding an `unsupported` case for a feature the task is supposed to implement. Wire the feature through every required layer or report the implementation as incomplete.
- Do not route unimplemented behavior to zero values, empty strings, empty collections, `Nothing`, default records, or other fallback values that make unsupported behavior appear successful.
- Do not add diagnostics that merely report requested behavior as "unsupported" unless the user's request is explicitly to reject that behavior. If behavior is meant to work, implement and validate it instead of producing a defensive error.
- Do not describe a change as production-ready, complete, fully supported, or done while any part of the requested behavior is stubbed, defaulted, mocked, unreachable, unsupported, or only represented in AST, IR, bytecode, metadata, or generated native output.
- When compiler work adds, removes, renames, renumbers, or reclassifies any error code or diagnostic rule, update `specifications/error_codes.md` (the build input — `build.rs` generates the `errorCode` constants from it) in the same change, and keep the embedded `mfb spec diagnostics` topics in sync. More broadly, keep the embedded specification current with every compiler change — see **Specifications (`mfb spec`)** below.
- For runtime features, add or run a runtime validation that executes the generated program and proves the requested behavior through exit code, stdout/stderr, file output, or another observable result.
- If runtime behavior cannot be implemented or validated, state that the task is blocked or incomplete and do not present compiler plumbing or golden output as functional support.
- Treat any backend helper named like `*_default_result`, or any backend path that stores default values for a built-in operation, as unsupported unless a runtime test proves the actual requested behavior.
- For any requested feature, do not implement or present a simulation, approximation, cooperative fallback, lazy substitute, single-step substitute, metadata-only substitute, queue-only substitute, or behavior-compatible shortcut as real support unless the user explicitly asks for that kind of simulation. If the real feature requires runtime helpers, OS/library integration, scheduler work, platform ABI changes, persistence, networking, concurrency, or other integration pieces, implement those pieces and validate the real behavior at runtime, or report the task as incomplete.

## Specifications (`mfb spec`)

The compiler's specification lives in `src/spec/**` and is embedded in the binary
(`mfb spec`), version-locked to the code: the spec you read always matches the
binary you have. It is the **single source of truth** for every externally
observable compiler/language/format/ABI contract, and it must stay accurate to the
compiler **as-is** at all times.

**The rule: any compiler change that adds, removes, or changes an observable
contract updates the owning `src/spec` topic in the same change.** This is part of
the Hard Completion Gate, not optional cleanup — a change that leaves the spec
stale is not done. Prefer an accurate stub over a missing or wrong topic. Contracts
that require a spec update include: language surface and type rules; IR/NIR op or
value forms and lowering behavior; the `.mfp` byte format; memory layouts, the
native calling convention, runtime-helper ABI, and program startup; AArch64
encoding; diagnostics and error codes; CLI/manifest/lockfile/audit/fmt/doc output;
the registry/signing protocol; threading; Unicode; and standard-package semantics.

Find the owning topic with `mfb spec` (or `mfb spec <package> --all`). Current
packages: `architecture` (the compiler pipeline/passes/CLI), `language` (source
semantics), `memory` (runtime value layouts + native ABI), `linker`, `package`
(`.mfp` byte format), `threading`, `diagnostics` (rule + error-code registries),
`tooling` (manifest/source-selection/lockfile/audit/fmt/doc/CLI), `package-manager`
(registry/keys/signing), `unicode`, `app` (`-app` GUI runtime), `stdlib` (regex/
datetime/csv/json/http/url/PCG64 models).

Conventions when editing the spec:

- **Single source of truth.** Each fact has one canonical topic. Other topics give
  a short summary and a `./mfb spec <package> <topic>` (or `./mfb man <package>`)
  link — never a second full copy. Small inlined facts are fine; a rats-nest of
  references and duplicated bodies is not.
- **Provenance.** Back a non-obvious implementation claim (magic number, offset,
  ABI register, enum variant, capability list, pass ordering) with an invisible
  `[[src/file.rs:Symbol]]` citation at claim-cluster granularity — symbol-preferred,
  `[[src/file.rs:line]]` only where no symbol fits. Grep-confirm the symbol exists
  before citing. The renderer strips `[[ ]]` everywhere (including headings), so
  they never display in `mfb spec`/`man` output but keep claims traceable for
  reviewers. Do not add non-verifiable claims.
- **Adding a topic / package.** A new topic is `NN_slug.md` beside the package's
  `spec.md` (auto-discovered, ordered by the `NN` prefix). A new package is a
  directory with a `spec.md` overview plus its `## See Also`; add its name to
  `PACKAGE_ORDER` in `src/spec/mod.rs`. Update the package overview's reading-order
  prose when adding a topic.
- **`error_codes.md` is the build input.** `build.rs` generates the `errorCode`
  constants from `specifications/error_codes.md`; update that file for runtime
  error-code changes and keep `mfb spec diagnostics error-codes` in sync (the
  drift-guard test only covers the generated constants, not the spec prose). The
  legacy `specifications/standard_package.md` and `project.md` are superseded by the
  embedded topics — update the `mfb spec` topic, not those.

Verify spec changes: `cargo build` (regenerates the embedded table; if a brand-new
file is not picked up, `touch build.rs` and rebuild), `cargo test --bin mfb spec`,
and confirm `mfb spec <package> --all` renders with no leaked `[[` markers and that
every `./mfb spec`/`./mfb man` link target and `[[…:Symbol]]` citation resolves.

## Native Codegen Register Lifetimes

Internal runtime helpers called via `bl` are not register-transparent. Treat any value held in a caller-saved scratch register as destroyed across the call unless you have proven otherwise from the callee's source.

Guidelines:

- `_mfb_arena_alloc` (`lower_arena_alloc` in `src/target/shared/code/mod.rs`) has an empty `callee_saved` frame and uses `x0`, `x1`, `x9`, `x10`, `x14`, `x15`, `x16`, and `x20`–`x28` as scratch (notably `x15`/`x14` in the block-grow path). Any value live across `bl _mfb_arena_alloc` in those registers is corrupted; only `x8`, `x11`, `x12`, `x13`, and `x17` currently survive. Do not rely on that survivor list as a stable contract — spill to a stack slot instead.
- When a quantity is computed before a runtime `bl` and consumed after it (lengths, counts, pointers, sizes, header fields), store it to a stack slot before the call and reload it afterward. Do not assume a register holds its value across the call.
- This class of bug is layout- and value-sensitive: the corrupted value may still produce correct results for small inputs and only fail past a threshold (e.g. a poisoned `DATA_LENGTH` field read as a huge size on the next operation, causing a runaway allocation or `SIGSEGV` only after N iterations). A passing small test does not prove the register lifetime is safe.
- When adding or auditing any helper that calls a runtime routine and then writes a collection/record/string header from registers, verify every header-field source register against the callee's clobber set. The same pattern recurs across insert, remove, concat, and map-mutation lowerings.
- Reproduce register-clobber crashes with a debugger: stale values leaking from the caller (registers the callee does not touch) plus a faulting helper pinpoint exactly which live register was destroyed. See the memory note `arena-alloc-clobbers-x14-x15` for the worked example.

## Planning

Substantial features get a written plan under `specifications/` before implementation begins. A plan is a design document that an implementer (human or agent) can execute phase-by-phase without re-deriving the design.

Guidelines:

- Name the file `specifications/plan-NN-shortname.md` (next free `NN`, two digits; short kebab-case slug). One plan per feature.
- Cross-link the specs the plan touches near the top (`specifications/memory_layouts.md`, `mfbasic.md`, `standard_package.md`, `error_codes.md`, `threading.md`, etc.) so the implementer reads the right source of truth first.
- State the constraints the plan must **not** violate as explicit non-goals (language surface, value/copy/move semantics, layout/ABI, thread-transfer rules). A plan that silently changes one of these is wrong.
- Break the work into ordered, independently-landable phases. Put the lowest-risk, separately-valuable work first (e.g. an audit or a runtime primitive with no callers) and the highest-risk codegen last, behind tests.
- Fold the repository's standing requirements into the plan, don't restate them generically: every new/changed function needs `tests/func_<package>_<func>_valid/**` and `_invalid/**` with full overload coverage; runtime features need an execution proof, not just golden output; error-code or diagnostic changes must update `error_codes.md`, `mfbasic.md`, and `standard_package.md`; acceptance (`scripts/test-accept.sh`) must pass.
- Record genuinely open design choices in an "Open Decisions" section with a recommendation for each — don't bury unresolved forks inside prose.
- When a plan is fully implemented, remove the plan doc in the same commit that lands the final phase (precedent: `34e526c9` removed plan-05 on completion). Keep `Last updated` current while it lives.

### Plan template

```markdown
# MFBASIC <Feature> Plan

Last updated: YYYY-MM-DD

<One or two paragraphs: what this builds and why. State the single
behavioral outcome a correct implementation produces.>

It complements:

- `specifications/<spec>.md` (<what this plan touches there>)

## 1. Goal

- <Concrete, checkable outcome.>

### Non-goals (explicit constraints)

<What must NOT change: language surface, value/copy/move/freeze semantics,
layout/ABI, thread-transfer rules. Be specific — these are the guardrails.>

## 2. Current State

<How it works today, cited to files/specs (`file.rs:line`, `spec.md §N`).
Name existing precedents the design will mirror.>

## 3. Design Overview

<The shape of the solution: independent pieces and how they layer. Call out
where the correctness risk concentrates.>

## 4..N. Detailed Design

<One section per piece. Algorithms, data layout, the runtime/codegen split.>

## Layout / ABI Impact

<Exactly what changes in memory_layouts.md / package_format.md, and — just as
important — what stays unchanged so copy/transfer/golden output is unaffected.
Omit if the plan touches no layout.>

## Phases

1. <Lowest-risk, independently-landable first.>
2. ...
N. <Highest-risk codegen last, behind tests.>

## Validation Plan

- Function tests: `tests/func_<pkg>_<func>_valid/**` and `_invalid/**`,
  every overload.
- Runtime proof: <the program + observable result that proves real behavior>.
- Doc sync: <error_codes.md / mfbasic.md / standard_package.md updates>.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- <Decision> — <recommended option> vs. <alternative>. (§ref)

## Non-Goals

- <Explicitly out of scope for this plan / V1.>

## Summary

<Where the real engineering risk is, and what is left untouched.>
```

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
- NEVER create a branch unless the user explicitly asks for one. Always commit on
  the current branch, even when that is the default/main branch. Do not create,
  switch, or rename branches on your own initiative.
- NEVER run tree-wide `git checkout` / `git reset` / `git restore` / `git stash`
  (e.g. `git checkout .`, `git reset --hard`, `git restore .`). You may only
  touch, edit, stage, and commit files that YOU modified during the current
  session; leave every other file's working-tree state exactly as found. Multiple
  clients may be working in this repo at once — tree-wide restores have destroyed
  other clients' in-progress edits and lost real work. Scope every git operation
  to your own specific paths.

## Remote Systems

- ssh -p 2222 test@127.0.0.1 # ArchLinux (libc)
- ssh -p 2223 test@127.0.0.1 # Kali (libc)
- ssh -p 2224 test@127.0.0.1 # Alipine (musl)
- ssh -p 2225 test@127.0.0.1 # Alipine gtk (musl)
- ssh -p 2226 test@127.0.0.1 # Debian 12 gtk (libc)
- ssh -p 2227 test@127.0.0.1 # Alipine x86_64 (musl)
