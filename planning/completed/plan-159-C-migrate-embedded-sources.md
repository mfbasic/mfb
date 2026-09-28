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
- [x] 1.2 Rewrite the example strings in `src/codegen/builtins/**` (the 821 `SUB main`,
      with their `END SUB`/`EXIT SUB`). Skip the 15 injected-body files in this phase. —
      `sub-to-func.py --rust-literals` over the 1424 builtin `.rs` files minus the 15 →
      `456 file(s) changed; 0 unhandled`. One assertion literal was mis-rewritten and fixed
      in 2.1 (Corrections).
- [x] 1.3 Migrate the example code blocks in `src/docs/man/**` (code only). — Fenced
      blocks only (`/tmp/p159/man-md.py`, the tool's fence mode): 14 files; the 13 prose
      lines outside fences are D 3.2's. Topic examples re-run with the final binary
      (`man-run-examples.sh <topic> --topic --run` for flow/lambda/tour/types/variable) →
      all build and run except `variable::guide#2`, which the page says "does not build"
      on purpose and which fails identically at the plan base.
- [x] 1.4 (added) Reword the built-in descriptor prose that names `SUB` — §1 Goal allows
      no `SUB` in `src` outside the compiler machinery, and the non-goal defers only the
      *narrative-topic* prose to D. `collections::forEach` (desc + example heading),
      `collections::transform`, `os::setEnv`, `os::unsetEnv`, `thread::start` (desc + the
      `f` parameter), `http::pump`.

Acceptance: every man example compiles and runs.
  Check: `bash scripts/man-examples-gate.sh target/release/mfb` → pass (est. 10 min; it is the only check that compiles every example, and C edits 821 of them).
  Result (2026-09-27): `man examples: 1116 checked, 0 failed, 0 stale not-run entr(y/ies)`.
  The 6 packages whose files Phase 2 then changed were re-run with the final binary:
  audio 11/11, canvas 32/32, datetime 116/116, json 18/18 ran; http 30/38 and term 37/47
  ran, every non-run exactly a `scripts/man-examples-not-run.txt` entry (servers, tty).
Commit: 65850e3d0 (1.1), f25cfdb4f (1.2–1.4)

### Phase 2 — Injected built-in bodies

- [x] 2.1 Rewrite the 34 `SUB __…` bodies in the 15 files (list them here). — `audio/{func_play,
      helper_play_samples}`, `canvas/{helper_damage,helper_geometry,helper_glyph_cache,
      helper_render}`, `datetime/helper_check_fields`, `http/{func_handle_request,func_pump,
      helper_linger_net,helper_linger_tls,helper_validate_pattern,helper_wait_readable}`,
      `json/helper_require_finite_number_text`, `term/helper_astrings_bridge` (all `.rs` under
      `src/codegen/builtins/`) → `15 file(s) changed; 0 unhandled`. Fixed by hand: the
      `audio/mod.rs` test assertion `contains("SUB __audio_playTracks")`, which the tool had
      turned into `"FUNC __audio_playTracks AS Nothing"` (not a substring of the real
      header), now `contains("FUNC __audio_playTracks(")`; `http::pump` prose; one MFBASIC
      comment in `canvas/helper_render.rs`.
- [x] 2.2 `git diff --stat -- '*.ncodesum' '*.ncode'` after regenerating → empty. — The
      gate with the C compiler flagged 46 diffs: 45 `.ir` + 1 `.app.nir`
      (`app-mouse-surface`), all injected-helper `"kind": "sub"` → `"func"`. A whole-tree
      `test-accept.sh` with that compiler → `46 mismatch(es) (1570 test(s) ran)`, all 46
      classified predicted and copied. `git diff --stat 4fe38df41 -- '*.ncode' '*.ncodesum'`
      → empty.

Acceptance: native goldens are byte-identical, and the canvas helpers still render.
  Result (2026-09-27): after the 2.2 regeneration, `artifact-gate.sh target/release/mfb all`
  → `1544 tests, 1723 build(s), 2182 golden(s) checked, 0 diff(s)`; `test-canvas-vulkan.sh …
  --box 2226 --target linux-aarch64` → `canvas Vulkan runtime tests passed` (groups and
  polylines reached the GPU and match their goldens).
  Check: `bash scripts/artifact-gate.sh target/release/mfb all` → 0 diffs (est. 20 min; the injected bodies span 15 built-in packages, so no single-package gate covers them); `bash scripts/test-canvas-vulkan.sh target/release/mfb --box 2226 --target linux-aarch64` → pass (est. 5 min; box 2226 because that is the script's native linux-aarch64 Vulkan box and the macOS host cannot run the Linux artifact; it covers the canvas helpers the gate cannot see).
Commit: f25cfdb4f

### Phase 3 — Generators and inline test sources

- [x] 3.1 `src/cli/init.rs:159` → `FUNC main() AS Nothing`, and update its test. — No test
      covered the template; added `hello_world_source_is_a_valid_nothing_entry` (parses,
      and `validate_entry_point` accepts it with `returns: Nothing`) → ok. Runtime proof:
      `mfb init /tmp/p159-init` → `mfb build` → `Hello World`, `[exit 0]`.
- [x] 3.2 `src/testing.rs:221`: generate `FUNC __mfb_test_case_N() AS Nothing`.
      `src/ast/build.rs:175` `fn sub` → build a `Func` with `returns: Nothing`, and
      update the callers in `src/testing/desugar/coverage.rs:199,221,232`. Delete
      `fn sub` if nothing else calls it. — Done; `fn sub` deleted (no other caller:
      `rg -n 'fn sub\b|\bsub\(' src` → only `ir/verify/tests.rs`'s own local `fn sub`, an
      IR test builder D owns). The coverage test at `coverage.rs:397` builds an
      `ExitTarget::Sub` node on purpose and is D's.
- [x] 3.3 Rewrite the inline sources in the 33 `src/` files and 84 `tests/` Rust files,
      except the SUB-rule tests listed in §3. — `--rust-literals --skip-fn '<the §3 names,
      kind…"sub", ISOLATED is valid only, must begin with FUNC or SUB, …>'` over the
      non-builtin `src` and `tests` Rust → 112 files, then 4 more after a prefilter fix
      (Corrections). Two intentionally malformed headers (`ast/tests.rs`) converted by hand
      (`SUB`→`FUNC`, keeping the unclosed `(`). SUB-rule tests the regex missed, found by
      running the suites and restored to their pre-C text for D: `ast::tests::{
      function_header_and_body_errors, native_func_missing_symbol_or_abi,
      serialize_covers_none_and_default_branches, top_level_errors_are_reported}`,
      `resolver::resolution::tests::doc_func_wrong_subkind_reports`,
      `rt_nothing_func_value_position::a_sub_call_in_value_position_is_still_rejected` (and
      the prelude's `SUB sb` it and the SUB-as-value test use), and the rule message
      `2-203-0042` in `rules/table.rs`.
- [x] 3.4 `mfb test` on one project with `TCASE`s (for example the smallest
      `tests/acceptance/*`) → the same pass count as before. — `tests/acceptance` is one
      project: plan-base compiler on the base tree → `Tests: 782  Pass: 782  Fail: 0`; C
      compiler on this tree → `Tests: 782  Pass: 782  Fail: 0`.
- [x] 3.5 (added) MFBASIC embedded outside `.mfb` and Rust — no letter covered it, and D
      would break it: shell heredocs in `scripts/test-{appimage,canvas-vulkan,macapp,winapp,
      winleak,winprocess}.sh`, `tools/canvas-bench/gen/*.sh`,
      `examples/wind/tests/cache_corruption_test.sh`, `packages/timezones/runtime-smoke.sh`,
      the Python generator `benchmark/mfb/gen_list.py`, and fenced code in `README.md` and
      `packages/{tar,timezones,zip}/README.md` (found with the §1 decl regex over the tree,
      `--glob '!*.mfb' --glob '!*.rs'`). The `printf` one-liner in `test-appimage.sh` and two
      `test-macapp.sh` descriptions by hand; `man-run-examples.sh`'s example detector now
      keys on `FUNC main` only. `gen_list.py` still regenerates the committed `list.mfb`
      byte-for-byte (`diff` → identical, as at the base). The spec files the regex also
      lists are D 3.1's.

Acceptance: the unit and integration suites that embed the sources pass.
  Check: `cargo test --bin mfb` → pass (est. 6 min); `target/release/mfb test tests/acceptance/<the 3.4 project>` → the same pass count as before C (est. 1 min; the generator change is only observable by running `mfb test`).
  Result (2026-09-27): `cargo test --bin mfb` → `4470 passed; 5 failed` — the 5 SUB-rule
  tests above, then restored and re-run with the new init test → `16 passed; 0 failed`.
  Strengthened (the criterion names integration suites too): every `[[test]]` target whose
  file C changed (91) → 2 failing targets on the first run (the hpke header miss and the
  SUB-rule test), both `ok` after the fixes (`5 passed`, `15 passed`). `mfb test` 782/782.
Commit: f25cfdb4f

## Validation Plan

- Runtime proof: `mfb init /tmp/p159-init && mfb run /tmp/p159-init` → prints the template greeting.
  (There is no `mfb run`; `mfb build` then executing the binary → `Hello World`, exit 0.)
- Final gate: at the end of D.

## Open Decisions

- None.

## Corrections

- **2026-09-27 — `--rust-literals` over-matches single-line literals (1.2, 3.3).** A literal
  counts as MFBASIC when a line is a `SUB <identifier>` header, so a message such as
  `"SUB cannot return a value"` (rule `2-203-0042`) and an assertion needle such as
  `"SUB __audio_playTracks"` were rewritten into nonsense. Found by sweeping the diff for
  one-line `"… AS Nothing"` literals (2 hits) and fixed by hand; the rule message is D's.
- **2026-09-27 — the literal prefilter hid `SUB` after a `\n` escape (3.3).** The word
  test ran on the raw literal, where `\nSUB` has the letter `n` before `SUB`. Literals that
  also held `END SUB` were rewritten anyway; header-only fragments were skipped, which
  left `rt_crypto_hpke_interop` with `SUB main()` … `END FUNC` (its build failed). The
  prefilter now tests the decoded text; the re-run over `src` + `tests` changed 4 more
  files, and the SUB-rule tests it touched were restored again.
- **2026-09-27 — MFBASIC outside `.mfb` and Rust was unowned (added 3.5).**
- **2026-09-27 — builtin descriptor prose was unowned (added 1.4).**
- **2026-09-27 — counts at execution:** `SUB main` in builtins 813 (not 821) and 477 builtin
  files (not 482), same commands.

## Summary

The risk is in the injected built-in bodies. Byte-identical native goldens plus the
canvas execution check cover it. The rest is mechanical, and `man-examples-gate`
verifies it.
