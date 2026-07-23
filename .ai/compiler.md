# Compiler / Runtime Work

Read this before changing built-ins, bytecode/IR, native code generation, runtime
helpers, package behavior, or diagnostics.

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
- When compiler work adds, removes, renames, renumbers, or reclassifies any error code or diagnostic rule, update the embedded `mfb spec diagnostics` topics in the same change — the `error-codes` topic's Constant Registry table is the **build input** that `build.rs` generates the `errorCode::` constants from. More broadly, keep the embedded specification current with every compiler change — see `.ai/specifications.md`.
- For runtime features, add or run a runtime validation that executes the generated program and proves the requested behavior through exit code, stdout/stderr, file output, or another observable result.
- If runtime behavior cannot be implemented or validated, state that the task is blocked or incomplete and do not present compiler plumbing or golden output as functional support.
- Treat any backend helper named like `*_default_result`, or any backend path that stores default values for a built-in operation, as unsupported unless a runtime test proves the actual requested behavior.
- For any requested feature, do not implement or present a simulation, approximation, cooperative fallback, lazy substitute, single-step substitute, metadata-only substitute, queue-only substitute, or behavior-compatible shortcut as real support unless the user explicitly asks for that kind of simulation. If the real feature requires runtime helpers, OS/library integration, scheduler work, platform ABI changes, persistence, networking, concurrency, or other integration pieces, implement those pieces and validate the real behavior at runtime, or report the task as incomplete.

## A Bug You Find Is a Bug You Fix

Compiler work surfaces unrelated defects constantly — you probe one predicate and
three others turn out to be wrong. **Fix them in the same change.** See AGENTS.md
("Never leave a bug in place"); the compiler-specific points:

- **A silent wrong value is the worst class here, and it is the one this codebase
  produces.** A predicate that under-reports does not fail loudly — it omits a data
  object, mis-types a literal, or drops a check, and the program computes a wrong
  number forever. Treat "the build succeeded and printed something" as no evidence
  at all: check the *value*. bug-367 (`LET a AS Fixed = -1.25` storing an f64 bit
  pattern, printing `-1074528256.0`) survived because nothing asserted the value.
- **When you fix one type seam, probe the siblings before you stop.** These gaps
  come in families: `static_nir_value_type`, `static_type_name_with_types`, and
  `CodeBuilder::static_type_name` are three separate walks over the same NIR, and a
  missing `MemberAccess`/variant arm in one is nearly always missing in the others
  (bug-363 → bug-366 found exactly this, one seam at a time). Grep for the sibling
  walks and test each directly.
- **Probe the whole matrix, not the one case you were handed.** Vary the numeric
  type (Integer/Float/Fixed/Money), the operand position (left/right), and the
  operand *shape* (literal, local, param, record field, union-variant field, map
  entry). bug-366's Money half fails with plain locals and would have been missed
  by testing record fields alone.
- **Confirm pre-existing vs. regression with `git worktree add --detach <path> HEAD`**
  and build there. Never `git stash` this tree to check (other clients share it).
  Being able to *state* "pre-existing, verified at HEAD" is worth the 60s build —
  but it changes only the commit message, never whether you fix it.

## Validation

After completing any code or golden-output change, the acceptance suite must pass.

Guidelines:

- For every function created or modified, automatically create or update matching fixtures. Fixtures live under four top-level trees, each `<bucket>/<feature>/<name>` (a `<feature>` directory is just a grouping dir with no `project.json` of its own):
  - `tests/syntax/<feature>/<name>` — compile-time diagnostics (a build that must fail, or must succeed, at `-ast -ir`). Example: `tests/syntax/datetime/func_datetime_localOffset_invalid`.
  - `tests/rt-error/<feature>/<name>` — runtime errors (the program builds, runs, and traps). Example: `tests/rt-error/arithmetic/<name>`.
  - `tests/rt-behavior/<feature>/<name>` — runtime behavior (the program builds, runs, and produces correct output). Example: `tests/rt-behavior/datetime/datetime-instant-valid`.
  - `tests/acceptance` — the single end-to-end TESTING app; not per-function.
  So a new/changed function gets a valid fixture under `tests/rt-behavior/<pkg>/` (or `tests/syntax/<pkg>/` for a compile-shape-only proof) AND an invalid fixture under `tests/syntax/<pkg>/`. The old flat `tests/func_<package>_<func>_{valid,invalid}/` layout no longer exists — never create it.
- Function fixtures are mandatory and non-skippable. Do not omit them because a change seems small, internal, obvious, already covered indirectly, or difficult to exercise.
- Function fixtures must cover every overload of the created or modified function. If an overload cannot be tested, the task is incomplete until the blocker is resolved or explicitly accepted by the user.
- Valid fixtures must prove each overload succeeds with representative runtime behavior or observable compiler behavior appropriate to the function.
- Invalid fixtures must prove each overload rejects incorrect usage, including wrong argument count, wrong argument type, invalid receiver/context, and relevant boundary or error cases.
- Do not describe a function change as complete while either the valid or invalid fixture is missing, empty, skipped, or lacking overload coverage.
- Run `scripts/test-accept.sh target/debug/mfb target/accept-actual` after compiler work or any change that can affect generated AST, IR, bytecode, native binaries, or diagnostics.
- Acceptance passing is required but not sufficient for runtime behavior changes. For runtime features, also add or run an execution test that proves the generated program behaves correctly.
- If acceptance fails, verify whether each failure is caused by the compiler update, a stale expected-output fixture, or a real regression before fixing code or updating goldens.
- Do not assume an acceptance mismatch is a test issue. Prove stale goldens by comparing deterministic regenerated output, and when necessary compare against a clean pre-change checkout.
- Do not leave acceptance failing at the end of the task unless an external blocker makes the suite impossible to run; report that blocker and the exact command/output.
- **App-mode proof runs on the GTK boxes, and Linux app mode cannot be emulated.** macOS app mode is proved locally by `scripts/test-macapp.sh`. Linux app mode is proved by `scripts/test-appimage.sh`, which builds an AppImage here and ships it over ssh — because an AppImage carries hex `0x414902` at offset 8, which the real Linux kernel ignores but qemu-user/Rosetta's ELF loader **rejects**, so it fails under Docker/binfmt on the Mac before its runtime ever runs. App mode covers BOTH libc worlds (plan-56-B), so run `--libc both`: glibc on 2228, musl on 2227/2224. ⚠️ **Launching a musl AppImage proves nothing** — musl's loader absorbs `libc.so.6`/`libpthread.so.0`, so a wrongly-linked musl binary runs identically to a correct one. Only the inner ELF's `DT_NEEDED` distinguishes them, which is why the script asserts on it. riscv64 remains impossible (no ported GTK entry, no upstream runtime).

## Native Codegen Register Lifetimes

Internal runtime helpers called via `bl` are not register-transparent. Treat any value held in a caller-saved scratch register as destroyed across the call unless you have proven otherwise from the callee's source.

Guidelines:

- `_mfb_arena_alloc` (`lower_arena_alloc` in `src/target/shared/code/entry_and_arena.rs`) is a vreg-allocated, PCS-framed helper: all caller-saved integer registers (`x0`–`x17`) are clobbered; callee-saved (`x19`–`x28`) are preserved by its frame. There is no survivor set. Never keep a live value in a caller-saved register across any `bl _mfb_*` runtime-helper call — spill to a stack slot instead (the register allocator's clobber model already treats every `_mfb_*` call as destroying all integer registers).
- When a quantity is computed before a runtime `bl` and consumed after it (lengths, counts, pointers, sizes, header fields), store it to a stack slot before the call and reload it afterward. Do not assume a register holds its value across the call.
- This class of bug is layout- and value-sensitive: the corrupted value may still produce correct results for small inputs and only fail past a threshold (e.g. a poisoned `DATA_LENGTH` field read as a huge size on the next operation, causing a runaway allocation or `SIGSEGV` only after N iterations). A passing small test does not prove the register lifetime is safe.
- When adding or auditing any helper that calls a runtime routine and then writes a collection/record/string header from registers, verify every header-field source register against the callee's clobber set. The same pattern recurs across insert, remove, concat, and map-mutation lowerings.
- Reproduce register-clobber crashes with a debugger: stale values leaking from the caller (registers the callee does not touch) plus a faulting helper pinpoint exactly which live register was destroyed. See the memory note `arena-alloc-clobbers-x14-x15` for the worked example.
