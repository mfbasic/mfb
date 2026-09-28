# plan-159-C: Migrate MFBASIC embedded in Rust (man examples, injected built-ins, generators, tests)

Last updated: 2026-09-26
Effort: large (3h–1d) (guess: 482 builtin files of mostly one-line `SUB main` edits, plus 34 injected bodies whose goldens are the risk)
Depends on: plan-159-B

MFBASIC source that lives inside Rust string literals stops using `SUB`. That covers
the 821 `mfb man` examples, the 34 `SUB __…` bodies the compiler injects for built-ins,
the `mfb init` template, the test-harness generators, and the inline test programs in
Rust unit and integration tests. After C, the only `SUB` left anywhere is the
compiler's own support for it and D's exclusion-list fixtures.

References: plan-159-A (prerequisites, rewriter), `.ai/man-content.md` (the man examples
are rendered from registry descriptors; verification with `scripts/man-run-examples.sh`),
`.ai/testing-gates.md`.

## Prerequisites

The plan-wide prerequisites are in plan-159-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-159-B complete | `ls planning/plan-159-B-*` → no match | MET (2026-09-27: B archived to `planning/completed/`) |

## 1. Goal

- `rg -n '\bSUB\b' src --type rust` lists only the compiler's SUB machinery that D
  deletes (lexer, ast, ir, binary_repr, fmt, doc, rule table, and the SUB-rule unit
  tests), plus the assembler comments `x86_64/encode/emitter.rs:654,1757` and
  `aarch64/encode/emitter.rs:709` (the machine `SUB` instruction, which is not the keyword).
- `mfb init` generates `FUNC main() AS Nothing`.
- Every rendered `mfb man` example still compiles and runs.

### Non-goals

- No change to what a built-in does. The injected bodies are rewritten
  source-for-source.
- No man prose rewrite beyond the example code. The narrative-topic prose about `SUB`
  is D's job.

## 2. Current State

| What | Count | Command |
|---|---|---|
| `SUB main` in builtin descriptors (man examples) | 821 (813 at execution, 2026-09-27) | `rg -o '\bSUB main' src/codegen/builtins \| wc -l` |
| builtin files mentioning `SUB` | 482 (477 at execution) | `rg -l '\bSUB\b' src/codegen/builtins \| wc -l` |
| injected `SUB __…` bodies | 34 in 15 files | `rg -n '(^\|r#?"\|\\n)\s*SUB __' src/codegen/builtins \| wc -l`; `-l` → 15 (for example `http/func_pump.rs`, `canvas/helper_render.rs`, `datetime/helper_check_fields.rs`, `audio/func_play.rs`) |
| inline `SUB x` in non-builtin `src/` Rust | 376 lines in 33 files | `rg -n '\bSUB [A-Za-z_]' src --type rust -g '!src/codegen/builtins/**' \| wc -l`; `-l` → 33 (largest: `ir/tests.rs`, `resolver/resolution.rs`, `ast/tests.rs`, `monomorph/lower.rs`) |
| `SUB` in `tests/**/*.rs` | 1023 lines in 84 files | `rg -n '\bSUB\b' tests --type rust \| wc -l`; `-l` → 84 |
| man narrative topics | 83 lines in 16 files | `rg -c '\bSUB\b' src/docs/man` (examples here; prose goes to D) |

Generators that emit `SUB` source (from the census):

- `src/cli/init.rs:159`: the `mfb init` template.
- `src/testing.rs:221`: the generated `__mfb_test_case_N` SUBs.
- `src/ast/build.rs:175` `fn sub`: called from `src/testing/desugar/coverage.rs:199,221,232`.

**Which goldens move.** The injected bodies reach the byte-identity gate only through
fixtures that call those built-ins (`.ai/testing-gates.md`). Measured in A:
SUB → `FUNC AS Nothing` is `.ncode`-neutral, so **every `.ncodesum` must stay
byte-identical**. `.ir` goldens that dump injected helper IR move their kind string.
Unit tests that assert a SUB-rule diagnostic belong to D; leave them unchanged here.

## 3. Design Overview

The rewriter from A cannot run on Rust files directly. The literals are either raw
strings (`r#"…"#`) or `\n`-escaped strings. Extend `tools/sub-to-func/` with a
`--rust-literals` mode that rewrites `SUB`/`END SUB`/`EXIT SUB` only inside string
literals, and reports any hit it cannot classify. Rust unit tests whose **purpose** is
a SUB rule (`SUB_RETURN_FORBIDDEN`, `EXIT_SUB_IN_FUNC`, the `ISOLATED SUB` parse error,
`FunctionKind::Sub` round-trips) are excluded. D deletes or converts them together
with the rule. Find them with `rg -l 'SUB_RETURN_FORBIDDEN|EXIT_SUB_IN_FUNC|FunctionKind::Sub|ExitTarget::Sub|Keyword::Sub' src tests --type rust`.

Risk is concentrated in the **injected bodies**. They are compiler-generated MFBASIC
that runs inside real programs (canvas render, http pump, audio play). Byte-identity
of `.ncodesum` is the gate, and `canvas` is not byte-identity-covered except through
`tests/syntax/app/app-mouse-surface` (`.ai/testing-gates.md`), so C also runs
`scripts/test-canvas-vulkan.sh`-class execution where the canvas helpers changed.

## Phases

> Tick in the same commit as the work. **An unticked box means NOT DONE.**

### Phase 1 — Man examples

- [x] 1.1 Add `--rust-literals` to `tools/sub-to-func/` and update its README. — Done,
      plus `--skip-fn REGEX` (for §3's SUB-rule tests) and fence-aware rewriting of man
      pages. Dry run `--check --rust-literals src tests` → `582 file(s) would change; 2
      unhandled line(s)` (the two deliberately malformed headers in `src/ast/tests.rs`,
      handled by hand in 3.3); `.mfb` mode unchanged (`--check` over the `.mfb` dirs →
      only the 4 exclusion fixtures).
- [ ] 1.2 Rewrite the example strings in `src/codegen/builtins/**` (the 821 `SUB main`,
      with their `END SUB`/`EXIT SUB`). Skip the 15 injected-body files in this phase.
- [ ] 1.3 Migrate the example code blocks in `src/docs/man/**` (code only).

Acceptance: every man example compiles and runs.
  Check: `bash scripts/man-examples-gate.sh target/release/mfb` → pass (est. 10 min; it is the only check that compiles every example, and C edits 821 of them).
Commit: —

### Phase 2 — Injected built-in bodies

- [ ] 2.1 Rewrite the 34 `SUB __…` bodies in the 15 files (list them here).
- [ ] 2.2 `git diff --stat -- '*.ncodesum' '*.ncode'` after regenerating → empty.

Acceptance: native goldens are byte-identical, and the canvas helpers still render.
  Check: `bash scripts/artifact-gate.sh target/release/mfb all` → 0 diffs (est. 20 min; the injected bodies span 15 built-in packages, so no single-package gate covers them); `bash scripts/test-canvas-vulkan.sh target/release/mfb --box 2226 --target linux-aarch64` → pass (est. 5 min; box 2226 because that is the script's native linux-aarch64 Vulkan box and the macOS host cannot run the Linux artifact; it covers the canvas helpers the gate cannot see).
Commit: —

### Phase 3 — Generators and inline test sources

- [ ] 3.1 `src/cli/init.rs:159` → `FUNC main() AS Nothing`, and update its test.
- [ ] 3.2 `src/testing.rs:221`: generate `FUNC __mfb_test_case_N() AS Nothing`.
      `src/ast/build.rs:175` `fn sub` → build a `Func` with `returns: Nothing`, and
      update the callers in `src/testing/desugar/coverage.rs:199,221,232`. Delete
      `fn sub` if nothing else calls it.
- [ ] 3.3 Rewrite the inline sources in the 33 `src/` files and 84 `tests/` Rust files,
      except the SUB-rule tests listed in §3.
- [ ] 3.4 `mfb test` on one project with `TCASE`s (for example the smallest
      `tests/acceptance/*`) → the same pass count as before.

Acceptance: the unit and integration suites that embed the sources pass.
  Check: `cargo test --bin mfb` → pass (est. 6 min); `target/release/mfb test tests/acceptance/<the 3.4 project>` → the same pass count as before C (est. 1 min; the generator change is only observable by running `mfb test`).
Commit: —

## Validation Plan

- Runtime proof: `mfb init /tmp/p159-init && mfb run /tmp/p159-init` → prints the template greeting.
- Final gate: at the end of D.

## Open Decisions

- None.

## Corrections

(none yet)

## Summary

The risk is in the injected built-in bodies. Byte-identical native goldens plus the
canvas execution check cover it. The rest is mechanical, and `man-examples-gate`
verifies it.
