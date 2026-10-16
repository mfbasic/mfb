# Move Display Width to `term::measureText` Plan

Last updated: 2026-10-15
Effort: medium (1h–2h)

Move terminal-cell text measurement out of `strings` and expose it as
`term::measureText`. The public function will be available only while the TUI is
active, while internal compiler helpers retain ungated access for string padding
and other generated implementation code.

References:

- `src/docs/spec/unicode/01_tables-and-algorithms.md`
- `src/docs/spec/unicode/02_strings-model.md`
- `src/docs/spec/app/04_term-backend.md`
- `.ai/compiler.md`, `.ai/codegen-invariants.md`, `.ai/resources-packages.md`,
  `.ai/testing-gates.md`, `.ai/man-content.md`, `.ai/spec-content.md`,
  `.ai/specifications.md`, and `.ai/remote_systems.md`

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Plan number 178 is unused | `find planning -type f -name 'plan-178*' \| wc -l` → `1` (`planning/plan-178-term-measure-text.md`, this plan only) | MET |

Everything below is written against the world where this holds.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before implementation begins.

## 1. Goal

- `term::measureText(String|AttributedString)` returns exactly the number of
  terminal grid cells that `term::drawText` advances, including one cell for an
  all-zero-width grapheme, and raises the established term errors outside an
  active Console TUI.
- `strings::displayWidth` is removed from the user API with no compatibility
  alias.
- `strings::padLeftToWidth` and `strings::padRightToWidth` retain their current
  ungated availability and rejection of a zero-width `padChar`.

### Non-goals (explicit constraints)

- Do not change grapheme segmentation, East Asian width tables, terminal drawing,
  string pad result ownership, or padding's undershoot rule.
- Do not expose the raw measurement or scalar-width helpers to user source.
- Do not make `strings` initialize terminal state or require `term::on()`.
- Do not define canvas/font measurement semantics; `canvas::measureText` remains
  the distinct pixel/font API.
- Do not retain a deprecated `strings::displayWidth` spelling.

## 2. Current State

`src/codegen/builtins/strings/func_display_width.rs:display_width_of` implements
the native Unicode walk and registers the public inline
`strings::displayWidth`. It assigns zero cells to a grapheme containing only
zero-width scalars. In contrast, `src/codegen/term/grid/term_grid.rs` and
`src/codegen/term/core/term.rs` clamp such a grapheme to one cell while drawing;
`src/docs/spec/unicode/02_strings-model.md` documents that divergence.

`src/codegen/builtins/strings/helper_pad_to_width.rs` calls the public function
both to measure the value and to reject a one-scalar pad character with width
less than one. The in-place implementation repeats those calls through
`src/codegen/collection/assign/string_self_update.rs:emit_pad_to_width_measure`.
Those two jobs cannot share a clamped result: terminal measurement must clamp an
all-zero-width grapheme to one, while pad-character validation must continue to
observe raw scalar width zero.

`src/codegen/builtins/term/func_terminal_size.rs` is the error precedent:
inactive access raises `ErrUnsupported`. `src/codegen/app/hook/app.rs:prepend_wrong_mode_gate`
is the Console-mode precedent. `src/codegen/engine/builder/mod.rs:uses_term`
reserves terminal state from `_mfb_rt_term_` symbols, and
`src/resolver/resolution.rs` rejects registry members marked `internal_only`
from ordinary user source.

### Measured populations

| What | Count | Command |
|---|---:|---|
| Non-golden old-name occurrences in implementation, examples, benchmarks, tests, and spikes | 111 | `rg -o 'strings::displayWidth\|strings\.displayWidth' src examples benchmark tests spikes --glob '!**/golden/**' \| wc -l` → `111` |
| Non-golden files containing the old name | 36 | `rg -l 'strings::displayWidth\|strings\.displayWidth' src examples benchmark tests spikes --glob '!**/golden/**' \| wc -l` → `36` |
| Acceptance golden files containing the old name | 24 | `rg -l 'strings::displayWidth\|strings\.displayWidth' tests --glob '**/golden/**' \| wc -l` → `24` |
| Old-name occurrences in acceptance goldens | 98 | `rg -o 'strings::displayWidth\|strings\.displayWidth' tests --glob '**/golden/**' \| wc -l` → `98` |
| Direct display-width acceptance projects | 1 | `find tests -path '*displayWidth*' -name project.json \| wc -l` → `1` |
| Term rt-behavior projects available for the replacement coverage | 16 | `find tests/rt-behavior/term -name project.json \| wc -l` → `16` |
| Term syntax projects available for the replacement diagnostics | 23 | `find tests/syntax/term -name project.json \| wc -l` → `23` |

The 111 non-golden occurrences divide into 33 under `src`, 32 under `examples`,
11 under `benchmark`, 33 under `tests`, and 2 under `spikes`, measured with:

```sh
for d in src examples benchmark tests spikes; do
  printf '%s ' "$d"
  rg -o 'strings::displayWidth|strings\.displayWidth' "$d" --glob '!**/golden/**' | wc -l
done
```

### Verified properties

- Every public `term` member is emitted through the native ABI-function path;
  `rg -n 'Body::abi_' src/codegen/builtins/term` shows `Body::abi_function`
  registrations and no public `Body::abi_inline` member. `term::measureText`
  will follow that model, which also makes terminal-state reservation automatic.
- An ungated internal inline helper does not reserve terminal state because
  `src/codegen/engine/builder/mod.rs:uses_term` keys state allocation to emitted
  `_mfb_rt_term_` runtime symbols, not to package imports.
- Generated source can call registry members marked `internal_only`, while user
  source cannot; this is enforced at `src/resolver/resolution.rs:1717-1718`.
- The existing `term`/`astrings` bridge is injected only when both packages are
  imported and already routes `term::drawText(AttributedString)` through visible
  text in `src/codegen/builtins/term/helper_astrings_bridge.rs`.

## 3. Design Overview

Create `src/codegen/builtins/term/func_measure_text.rs` as the single owner of
terminal-cell measurement. Its shared native lowering segments extended
graphemes, takes the first non-zero scalar width in each cluster, and contributes
one cell when every scalar in the cluster has raw width zero. The implementation
is exposed through these registry surfaces:

1. Public `term::measureText(String) AS Integer` is an ABI function. It first
   applies the Console-mode gate (`ErrWrongMode`), then the active-terminal gate
   (`ErrUnsupported`), then measures the String.
2. Internal-only `term::measureTextRaw(String) AS Integer` is an ungated inline
   helper with the same clamped terminal-cell result. Generated padding and
   in-place lowering use it without turning on the TUI.
3. Internal-only `term::scalarWidthRaw(String) AS Integer` is an ungated inline
   helper for the already-validated one-scalar pad character. It returns the raw
   Unicode scalar width, preserving rejection of combining marks and ZWJ.

Extend `helper_astrings_bridge.rs` with `__term_measureTextAttr`, which calls
`term::measureText(toString(value))`. Extend the same shape/lower/verify seams
used by `term::drawText` so `term::measureText(AttributedString)` selects that
bridge and requires `IMPORT astrings`.

The correctness risk is concentrated in separating clamped grid measurement
from raw pad-unit validation and in preserving term gate order. The migration is
observably behavioral, so byte identity is not a correctness gate. Diffs are
expected for projects that call the renamed API, exercise the new gates, or
measure an all-zero-width grapheme. A diff elsewhere is investigated by
objdumping one fixture before any golden change.

Rejected alternatives:

- Keeping an alias under `strings` would preserve the namespace mistake and was
  explicitly rejected by the owner.
- Making the public function inline would bypass the normal term ABI/gating and
  would not naturally reserve terminal state.
- Using the clamped helper for `padChar` would turn combining marks and ZWJ into
  valid one-cell padding, changing the approved padding contract.
- Requiring padding to call public `term::measureText` would make string padding
  depend on active TUI state.

## 4. Public API and Gate Semantics

Register `measureText` in `src/codegen/builtins/term/mod.rs`, document both
String and AttributedString forms, and remove the old member from
`src/codegen/builtins/strings/mod.rs` and the global infallible/Tier-A tables.
Gate behavior is fixed in this order:

- App mode other than `app::Mode.Console`: `ErrWrongMode`.
- Console mode with term off, including before `term::on()` and after
  `term::off()`: `ErrUnsupported`.
- Active Console TUI: return the grid-cell count.

The empty String returns zero. ASCII returns one cell per grapheme; wide CJK and
emoji clusters return two where the embedded width table says two; combining
sequences use the first non-zero scalar width; an all-zero-width grapheme returns
one to match drawing.

## 5. Internal Consumers and Migration

`helper_pad_to_width.rs` imports `term`, calls `term::measureTextRaw(value)` for
the current width, and calls `term::scalarWidthRaw(padChar)` for pad validation.
`string_self_update.rs` calls the corresponding Rust lowering entry points.
Both retain identical error codes, whole-copy undershoot behavior, and
availability without `term::on()`.

Update the existing attributed draw bridge to advance run columns with
`term::measureText`, which is safe because that bridge already runs only while
term is on. Migrate examples, benchmarks, spikes, and tests to the public name.
Programs that measure text bracket measurement with `term::on()`/`term::off()`;
programs that print after `term::off()` cache the integer while active.
Terminal-browser display/DOM packages import `term` and describe their fixed-cell
layout as terminal-specific. The language width benchmark turns term on outside
the timed loop so the loop measures calls rather than lifecycle setup.

Update Unicode runtime-use analysis, static-data planning, and runtime usage
classification for the new public/runtime and raw/internal targets. Remove the
old constant-fold path: a public term call must execute its mode and active-state
gates even when its String argument is a literal.

## 6. Test and Golden Provenance

The existing tests below need semantic substitutions rather than a mechanical
rename. Their protected behavior and evidence are fixed here before any test
edit:

- `tests/codegen/regalloc_eviction/inline_builtins.mfb` was introduced by
  `802c29611` (`git log --follow --oneline -- tests/codegen/regalloc_eviction/inline_builtins.mfb`).
  It protects against restoring/resaving registers around each instruction of a
  hot inline-builtin loop. Only `tests/codegen/codegen_regalloc_eviction.rs`
  includes it (`rg -n 'inline_builtins' tests`). The owner-approved API move
  makes public measurement an ABI function, so the old call can no longer test
  inline lowering; replace that row with `strings::graphemesCount`, which retains
  the Unicode inline-walk purpose and the existing bound.
- The display-width row in `tests/runtime/rt_strings_unowned_blocks.rs` was
  introduced by `dfec19429` (`git log --follow --oneline -- tests/runtime/rt_strings_unowned_blocks.rs`).
  It protects the flat-allocation behavior of query siblings; callers are the
  Cargo test registration, the completed bug-992 record, and the test itself
  (`rg -n 'rt_strings_unowned_blocks' Cargo.toml planning bugs tests`). The old
  symbol is removed and the replacement has a fixed term-grid allocation, so
  preserve the leak/balance assertion in a dedicated term measurement test that
  compares repeated-call deltas after one lifecycle setup.

The direct API fixture protects width semantics plus type/arity diagnostics; the
owner's removal decision disproves only its old qualified name and zero-only
result. Move that coverage to term and correct only those lines. Before changing
any golden, run the complete acceptance suite, inspect one representative diff,
and then edit only the disproved API/gate/result lines in the 24 measured golden
files. Never regenerate or rebaseline a whole golden file.

## Compatibility / Format Impact

This is a source-breaking API rename with a lifecycle requirement. Existing
`strings::displayWidth` source fails resolution and must import `term`, call
`term::on()`, and use `term::measureText`, unless it is compiler-generated
padding code using the private raw helper. Literal calls are no longer folded at
compile time. The all-zero-width grapheme result changes from zero to one so
measurement agrees with `term::drawText`.

There is no object, record-layout, wire-format, or persisted-data change.
`strings::padLeftToWidth` and `strings::padRightToWidth` remain source-compatible
and callable with term off.

## Phases

### Phase 1 — Add term measurement and preserve padding

Land the new implementation and internal split before removing the old public
callers.

- [x] Add `func_measure_text.rs`; register public `measureText` plus internal-only
      `measureTextRaw` and `scalarWidthRaw`; add the Console and active-term gates,
      runtime dispatch, error data, Unicode-table usage, and package docs. —
      `cargo check --bin mfb` passed; `cargo test --bin mfb term -- --nocapture`
      passed 72 tests.
- [x] Extend `helper_astrings_bridge.rs`, `src/ir/shape.rs`, `src/ir/lower.rs`,
      `src/ir/verify/compat.rs`, and `src/ir/tests.rs` with the AttributedString
      overload and import requirement. — `cargo test --bin mfb term_measure_text
      -- --nocapture` passed 2 tests.
- [x] Switch `helper_pad_to_width.rs` and
      `string_self_update.rs:emit_pad_to_width_measure` to the internal helpers,
      retaining raw scalar validation for `padChar`. — `rg -n
      'scalarWidthRaw|measureTextRaw'` shows the generated helper split, `rg -n
      'scalar_width_of|measure_text_of'` shows the native split, and `cargo test
      --test rt_strings_unowned_blocks` passed 6 tests.
- [x] Add focused Rust/IR tests for public String and AttributedString dispatch,
      rejection of user calls to both internal names, Unicode-table retention,
      gate order, and unchanged off-TUI padding. — `cargo test --bin mfb
      term_measure_text -- --nocapture` passed 5 tests; `cargo test --test
      rt_strings_unowned_blocks` passed 7 tests.

Acceptance: focused compiler tests prove the overload routing, internal boundary,
and padding behavior.
  Check: `cargo test --bin mfb term_measure_text && cargo test --test rt_strings_unowned_blocks` → 5 focused compiler tests and 7 runtime tests pass (est. 3 min).
Commit: `cdc87b0a0`

### Phase 2 — Remove the strings API and migrate source callers

Remove the obsolete surface and update every non-golden caller to obey the new
lifecycle.

- [x] Delete `func_display_width.rs` and remove `strings.displayWidth` from
      strings registration, Tier-A/infallible tables, runtime-use analysis,
      data-object planning, and runtime-usage classification. — `cargo check
      --bin mfb` passed without warnings; `cargo test --bin mfb
      unicode::runtime_tables -- --nocapture` passed 13 tests; `rg -n
      'strings\.displayWidth|func_display_width' src/codegen src/target src/ir
      --glob '*.rs'` returned no implementation references.
- [x] Replace the direct behavior and syntax fixtures with term fixtures covering
      empty, ASCII, CJK, NFD, emoji, all-zero-width, AttributedString, wrong
      arity/type, inactive use, wrong app mode, private-helper rejection, and
      rejection of the removed strings name. — Both runtime projects built;
      running the measurement project under `script -q /dev/null` produced
      literal/dynamic widths `0, 3, 6, 4, 2, 1`, attributed CJK width `6`, and
      `ok`; all four syntax projects emitted their intended diagnostics; `cargo
      test --test cli_app_canvas_mode macos_term_traps_wrong_mode_in_canvas --
      --nocapture` passed 1 test.
- [x] Preserve the regalloc and unowned-block intent exactly as specified in
      §6; keep the string padding fixture off-TUI and assert its results directly.
      — `cargo test --test codegen_regalloc_eviction -- --nocapture` passed with
      0 `main` eviction pairs against the bound of 22; `cargo test --test
      rt_strings_unowned_blocks
      term_measure_text_balances_repeated_calls_after_one_setup -- --nocapture`
      passed with balanced 100/200-call runs; the off-TUI padding assertion
      remains in `width_padding_stays_ungated_and_rejects_zero_width_pad_chars`.
- [x] Migrate the 32 example, 11 benchmark, and 2 spike occurrences measured in
      §2, including caching values before `term::off()` and moving benchmark
      lifecycle outside timed loops. — The original census command now reports
      0 old-name occurrences in each of `examples`, `benchmark`, and `spikes`;
      the affected AI chat, wide demo, browser package chain, benchmark, and
      spike projects all produced native executables/packages. The benchmark
      lifecycle wraps only the three width rows, outside their timed loops, and
      the wide demo/spike cache measurements before `term::off()`.
- [x] Run the non-golden caller census again and reduce old-name call sites to
      zero, except for the required removed-name rejection fixture; documentation
      references remain queued for Phase 3. — `rg -n
      'strings::displayWidth|strings\.displayWidth' src tests examples benchmark
      spikes --glob '!**/golden/**' --glob '!src/docs/**' --glob
      '!src/codegen/builtins/**' --glob
      '!tests/syntax/strings/displayWidth/**'` returned no references. The full
      remaining-file census contains only seven documentation/descriptor files
      and the one intentional rejection source.

Acceptance: the focused term, strings-padding, regalloc, runtime-balance, and
Canvas wrong-mode tests pass without golden edits.
  Check: `cargo test --test codegen_regalloc_eviction && cargo test --test rt_strings_unowned_blocks && cargo test --test cli_app_canvas_mode` → 1 regalloc, 8 runtime-balance, and 11 Canvas tests passed (est. 4 min).
Commit: —

### Phase 3 — Synchronize specs, man pages, and acceptance goldens

Document the observable contract and make only evidence-backed expected-output
changes after the mandated pre-rebaseline suite.

- [ ] Update Unicode algorithm/string specs and the term backend spec so cell
      ownership, gate behavior, zero-only clamping, and raw padding validation are
      explicit; update source citations.
- [ ] Remove the strings man entry, add the term `measureText` descriptor/examples,
      update strings padding prose and package overviews, and render both man
      surfaces.
- [ ] Run the full acceptance suite before any golden edit; inspect one migrated
      fixture diff to localize the expected API/gate/result changes. Treat every
      unrelated diff as a bug-hunt trigger.
- [ ] Correct only the disproved lines in affected goldens, then rerun the full
      acceptance suite.

Acceptance: specs/man pages describe the implemented contract, no old public
name remains in live source documentation, and the full acceptance suite passes
after narrow golden corrections.
  Check: `cargo test --bin mfb spec && target/debug/mfb man term measureText && target/debug/mfb man strings --all` → spec tests pass, the new page renders, and the old member is absent (est. 3 min).
Commit: —

## Validation Plan

- Tests: term rt-behavior covers active String/AttributedString measurement and
  all Unicode classes above; term inactive and Canvas app fixtures cover both
  errors; syntax fixtures cover arity/type/import/private/removed-name failures;
  string padding tests remain off-TUI and retain zero-width pad rejection.
- Coverage check: `rg -n 'measureText|measureTextRaw|scalarWidthRaw' src tests`
  must show each public gate, both private helpers, both padding paths, the
  AttributedString bridge, and positive/negative fixtures.
- Runtime proof: build the migrated width fixture, run it in a PTY, and observe
  cached values `0`, `1`, and `2` classes plus `1` for an all-zero-width grapheme;
  the inactive fixture must trap `ErrUnsupported` and the Canvas fixture must
  trap `ErrWrongMode`.
- Doc sync: run `cargo test --bin mfb spec` and render `mfb man term measureText`
  plus `mfb man strings --all`; run the repository citation census required by
  `.ai/specifications.md` for the touched Unicode/app topics.
- Golden discipline: run `bash scripts/test-accept.sh target/debug/mfb target/accept-plan178`
  once before any golden correction, inspect one fixture, apply narrow edits,
  then run it again. The first run is required by the repository's
  pre-rebaseline rule; the second proves the corrected suite (est. 12 min total).
- Final gate: `cargo fmt --all --check`, `cargo test --all-targets`,
  `cargo build`,
  `bash scripts/artifact-gate.sh target/debug/mfb all`,
  `bash scripts/test-accept.sh target/debug/mfb target/accept-plan178`, and
  `git diff --check` all pass (est. 15 min; the broad gates are required because
  registry, frontend overload routing, runtime analysis, all native targets, and
  acceptance goldens change together).
- App-mode gate: `bash scripts/test-macapp.sh target/debug/mfb` and
  `bash scripts/test-appimage.sh target/debug/mfb --libc both` pass, proving
  `ErrWrongMode` in real macOS and Linux app runtimes across both Linux libc
  worlds (est. 30 min; no smaller local check exercises the real GTK app entry
  and both packaged libc variants).

## Decisions

- Move the public operation from `strings` to `term` and name it
  `term::measureText`; retain no old alias — decided by the owner, 2026-10-15.
- Gate the public operation behind `term::on()` and use normal term mode errors —
  decided by the owner, 2026-10-15.
- Permit internal ungated helpers while keeping them inaccessible to user source
  — decided by the owner, 2026-10-15.
- Match `term::drawText`, including one cell for an all-zero-width grapheme —
  decided by the owner, 2026-10-15.
- Keep width-padding available without the TUI and continue rejecting combining
  marks and ZWJ as `padChar` — decided by the owner, 2026-10-15.

## Corrections

- Phase 1 acceptance originally filtered on the nonexistent module path
  `ir::tests::term_measure_text`, selecting zero tests. The test names span the
  IR and built-in test modules, so the filter is `term_measure_text`; that
  command selected and passed all 5 focused compiler tests.
- Phase 2's literal zero-occurrence census conflicted with both its required
  removed-name rejection fixture and Phase 3's explicit documentation migration.
  The Phase 2 gate therefore measures executable caller sites, leaving the one
  negative fixture and seven documentation/descriptor files for their specified
  coverage and Phase 3 updates.

## Summary

The implementation has a canonical terminal-cell algorithm, a gated public term
API, and private ungated primitives for generated code. The main risk is
preventing the terminal clamp from changing pad-character validation; focused
off-TUI padding tests and the split raw helpers contain that risk. Canvas text
measurement and all non-terminal string semantics remain untouched.
