# plan-122-D: canvas adopts `color::Color`

Last updated: 2026-09-02
Effort: large (3h–1d)
Depends on: plan-122-C

Delete `canvas::Color`, `canvas::rgb` and `canvas::rgba`. `canvas::Paint`'s `fill`
and `stroke` become `color::Color`, and every canvas program builds its palette
with `color::rgb`/`rgba`/`fromHex` instead.

This is the first letter with a breaking API change. It is a rename, not a reshape:
`color::Color`'s field names, order and types are identical to `canvas::Color`'s by
construction (plan-122-A §4), so no canvas internal that reads `paint.fill.red`
changes at all.

Behavioral outcome: `examples/emoji` renders the same frame it renders today, built
against `color::` instead of `canvas::` colour calls, and `canvas::Color` no longer
resolves.

References:

- plan-122-A — Prerequisites; `COLOR_TYPE_ID`; the non-transitive-import finding
  that makes `IMPORT color` mandatory in every consumer.
- plan-122-B — canvas already gained `add_imports(["color"])` when the sRGB table
  moved, so **this letter adds no new import**.
- `.ai/canvas-threading.md` — the three-thread model; read before touching anything
  the graphics thread runs.
- `src/docs/spec/app/06_canvas.md` — the canvas specification, including `:263`
  ("The **value constructors are exempt**: `canvas::rgb`, `canvas::rgba`, …"), which
  this letter rewrites.

## Prerequisites

Stated once in plan-122-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-122-C complete | `ls planning/completed/plan-122-C-*` → one match | **MET** (2026-09-05) — `planning/completed/plan-122-C-color-names.md`, one match |
| canvas already imports `color` (landed in B) | `grep -n 'add_imports' -A 8 src/codegen/builtins/canvas/mod.rs` shows `"color"` | **MET** (2026-09-05) — `canvas/mod.rs:187` carries `"color"`, added by plan-122-B when the sRGB table moved. **This letter adds no new import.** |

Shared rows from plan-122-A, re-measured 2026-09-05: release binary built
(33,665,040 bytes); `git status --porcelain` empty; `qualify_value_type_references`
present (`src/codegen/registry/mod.rs:1733`).

D is the last unrun letter — A, B, C, E and F are all archived in
`planning/completed/`. It runs **after** E and F rather than before them, which is
the reverse of the authored order; nothing in D depended on that order (§Corrections).

If plan-122-C is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- `canvas::Color`, `canvas::rgb` and `canvas::rgba` do not resolve. A program using
  them gets a diagnostic naming the type or the call, not a silent
  `TYPE_UNKNOWN_VALUE`.
- `canvas::Paint.fill` and `.stroke` are `color::Color`.
- `canvas::fill`, `canvas::stroke` and `canvas::fillStroke` take `color::Color`.
- Every canvas test, example and doc builds against the new surface, and rendered
  output is unchanged.

### Non-goals (explicit constraints)

- **No change to `Paint`'s field set, order, or the zero-value rule.** `Paint`'s
  whole design rests on "every field's zero value is that field's no-op"
  (`canvas/mod.rs:450-453`), which requires the all-zero `Color` to be fully
  transparent. `color::Color` keeps that property (plan-122-A §4).
- **No change to any rendered pixel.** This is a rename; the geometry flattener
  still reads `paint.fill.red`/`green`/`blue`/`alpha`
  (`canvas/helper_geometry.rs:188-195`) and the stroke gate still reads
  `paint.stroke.alpha` (`canvas/helper_items.rs:51`), with the same field names.
- **No back-compat alias.** `canvas::rgb` is not kept as a forwarder (user
  decision, 2026-09-02): the measured non-transitive-import rule means a caller
  must `IMPORT color` to touch a channel anyway, so an alias buys nothing.
- **No change to the GPU backends.** They consume a flattened `List OF Float`
  (`canvas/helper_geometry.rs:188-195` writes channels into slots 8–15), not the
  `Color` record, so no backend reads a colour layout. Verified by reading the
  flattener and by `grep -rln 'Color' src/target/` returning no colour-record
  field access.

## 2. Current State

`canvas::Color` is declared at `src/codegen/builtins/canvas/mod.rs:183-210` and
referenced from:

| Site | File:line | Change |
|---|---|---|
| record declaration | `canvas/mod.rs:183` | delete |
| `Paint.fill` / `Paint.stroke` prop types | `canvas/mod.rs:458`, `:464` | `ParameterType::named(COLOR_TYPE_ID)` |
| `canvas_types_are_builtin_types` test list | `canvas/mod.rs:1110` | remove `"Color"` |
| `rgb` member | `canvas/func_rgb.rs` (whole file) | delete |
| `rgba` member | `canvas/func_rgba.rs` (whole file) | delete |
| `__canvas_clampByte` | `canvas/helper_clamp_byte.rs` (whole file) | delete — it moved to `color` in plan-122-A Phase 1 and has no other caller |
| `fill(color AS Color)` | `canvas/func_fill.rs:42`, `:59` | param type + body |
| `stroke(color AS Color, width)` | `canvas/func_stroke.rs:42`, `:60` | param type + body |
| `fillStroke(fill, stroke AS Color, width)` | `canvas/func_fill_stroke.rs:73`, `:91`, `:98` | param types + body |
| `__canvas_transparent()` | `canvas/helper_paint_defaults.rs:15-16` | returns `color::Color`, constructs `color::Color[…]` |
| man examples naming `canvas::Color`/`rgb` | `func_fill.rs:34`, `func_fill_stroke.rs:33-34`, `:52-53`, `func_present.rs:43-44`, `func_present_layers.rs:42-43`, `func_stroke.rs:34`, `func_rgba.rs` (deleted) | rewrite to `color::` and add `IMPORT color` |

Unchanged, because the field names are identical: `canvas/helper_geometry.rs:188-195`
and `canvas/helper_items.rs:51`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `canvas::rgb(` sites in `src/` | 19 | `grep -rn 'canvas::rgb(' src \| wc -l` |
| `canvas::rgb(` sites in `tests/` | 110 | `grep -rn 'canvas::rgb(' tests \| wc -l` |
| `canvas::rgb(` sites in `examples/` | 8 | `grep -rn 'canvas::rgb(' examples \| wc -l` |
| `canvas::rgba(` sites in `src/` / `tests/` | 2 / 11 | `grep -rn 'canvas::rgba(' src \| wc -l`, same for `tests` |
| `canvas::Color` sites in `src/` / `tests/` / `examples/` | 24 / 27 / 5 | `for d in src tests examples; do grep -rn 'canvas::Color' $d \| wc -l; done` |
| Rust test files naming the canvas colour surface | 8 | `grep -rln 'canvas::rgb\|canvas::Color' tests/*.rs \| wc -l` |
| example `.mfb` files naming it | 1 (`examples/emoji/src/main.mfb`) | `grep -rl 'canvas::rgb\|canvas::Color' --include='*.mfb' examples/` |
| spec/man docs naming it | 4 | `grep -rln 'canvas::rgb\|canvas::Color' src/docs/ \| wc -l` |

The 8 Rust test files: `tests/cli_canvas_image_resource.rs`,
`tests/cli_canvas_package.rs`, `tests/rt_canvas_damage.rs`,
`tests/rt_canvas_font.rs`, `tests/rt_canvas_golden.rs`,
`tests/rt_canvas_graphics_thread.rs`, `tests/rt_canvas_metal.rs`,
`tests/rt_canvas_present_deep_copy.rs`, `tests/rt_canvas_rasteriser.rs`
(`grep -rln 'canvas::rgb\|canvas::Color' tests/*.rs`). They embed MFBASIC source as
Rust string literals, so each needs `IMPORT color` added to the embedded programs —
a `sed` over the call names alone will produce programs that fail to build.

### Verified properties

- **The field set is identical**, so no reader changes: `canvas::Color` is
  `red`/`green`/`blue`/`alpha`, all `Byte`, in that order
  (`canvas/mod.rs:190-209`), and `color::Color` is declared the same
  (plan-122-A §4).
- **`canvas::Color` is not special-cased in the binary representation.** Unlike
  `term::TermColor`, it has no reserved wire id: `src/binary_repr/sections.rs`
  names only `term.TermColor`/`term.TermSize` among builtin value records
  (`:173`, `:181`), and everything else resolves through `self.ids` /
  `foreign_types` (`:236-260`). So deleting `canvas::Color` touches no wire format.
- **`canvas::rgb`/`rgba` are the only two canvas calls exempt from the
  `Mode.Canvas` requirement** (`canvas/mod.rs` MODULE_DESC; `06_canvas.md:263`).
  Removing them removes the exemption entirely, which simplifies the rule rather
  than complicating it — every remaining `canvas::` call requires `Mode.Canvas`.
  The spec sentence must be rewritten, not just edited.

## 3. Design Overview

One mechanical rename with three seams that are **not** mechanical, and those are
where the work is:

1. **Descriptor types must use `COLOR_TYPE_ID`, not the bare leaf.** The registry
   refuses a bare cross-package leaf and the refusal is tested
   (`a_bare_cross_package_leaf_is_refused_in_a_signature`,
   `src/codegen/registry/mod.rs:3658`). Every `ParameterType::named("Color")` in
   canvas becomes `ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID)`,
   exactly as `tcp` spells `net.Address` (`src/codegen/builtins/tcp/mod.rs:103-112`).
2. **Injected source uses the `::` spelling.** Inside canvas's companion bodies the
   type is written `color::Color` and constructed `color::Color[...]`. The registry
   rewrites record **field** types for source rendering separately
   (`qualify_type_leaves_for_source`, `registry/mod.rs:1709-1713`), which is why
   `Paint`'s two props are handled by the descriptor and the bodies by hand.
3. **Every embedded test program needs `IMPORT color`.** This is the one that will
   bite: the failure is a `TYPE_UNKNOWN_VALUE` on a field read, not on the call, so
   a test that only *builds* a `Paint` passes while one that reads `c.red` fails.

**Where correctness risk concentrates:** nowhere in the rename itself — it is
name-for-name with an identical layout. The risk is **coverage**: a missed call site
in an embedded Rust string is a test that stops compiling MFBASIC, and a missed doc
example is a man page that lies. Phase 1 is therefore a census, and the acceptance
gate is `scripts/man-run-examples.sh canvas --run`, which compiles every example on
every canvas man page.

**Byte-identity is not the gate.** canvas `.ncode`/`.ncodesum` and every canvas
importer's `.ir`/`.ast` are **expected** to drift: the type name in every signature
changes and two members disappear. What must not drift is rendered pixels and every
`build.log`/`.run` that is not about the removed names.

### Rejected alternatives

- **A deprecation window with both spellings.** Rejected (user decision): two names
  for one type is the condition this plan removes.
- **`sed`-ing the tree.** Rejected on the standing rule against unchecked tree-wide
  scripts, and because the embedded Rust programs need an `IMPORT` line added, not
  a name substituted — a pure rename produces programs that build and then fail on
  a field read.

## Compatibility / Format Impact

**Breaking.** `canvas::Color`, `canvas::rgb`, `canvas::rgba` are removed. A program
migrates by adding `IMPORT color` and replacing `canvas::rgb`→`color::rgb`,
`canvas::rgba`→`color::rgba`, `canvas::Color`→`color::Color`.

Unchanged: `canvas::Paint`'s field names and order, the zero-value-is-no-op rule,
every `DrawItem` variant, the flattened geometry buffer the GPU backends read, and
every rendered pixel.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work; `- [~]` for partial with a line on what remains;
> `- [x] ~~text~~ — moot: <evidence>` rather than deleting. Fill `Commit:` on
> landing. **An unticked box means NOT DONE.**

### Phase 1 — Census

Land nothing; produce the work list the remaining phases execute against, so no
site is discovered late.

- [x] Enumerate every site with `grep -rn 'canvas::rgb(\|canvas::rgba(\|canvas::Color\|ParameterType::named("Color")' src tests examples src/docs`
      and write the file list into this document's Corrections section, grouped by
      phase. **Do not census by call name alone** — a `Paint[fill := c]` with `c`
      built elsewhere has no `canvas::` token on it at all, so also grep for
      `paint.fill`/`paint.stroke` readers and for `AS Color` in embedded programs.
- [x] Record the pre-change canvas `.ncodesum` set so Phase 4's drift is
      attributable: `find tests/byte-identity -name '*.ncodesum' | wc -l` → 133 today.
      **Measured 133** — agrees.

Acceptance: a written file list in Corrections whose counts match the §2 table. A
count that disagrees is a census bug — reconcile it before Phase 2.

**Met, by reconciliation.** Seven of the nine counts disagreed with §2 and the
census additionally found four sites §2 does not list at all. Each disagreement is
reconciled in Corrections C1–C7 with the command that measured it, and the phase
work lists below are rewritten against the measured population, not §2's. Nothing
is carried forward unreconciled.
Commit: `15f4ec3d0`

### Phase 2 — Descriptor and companion

- [x] `canvas/mod.rs`: delete the `Color` record (measured `:197`, not `:183`);
      repoint `Paint.fill` (`:544`) **and `Paint.stroke` (`:550`) and
      `GradientStop.color` (`:482`, which §2 missed — C7)** at `COLOR_TYPE_ID` and
      rewrite their descriptions to say `color::Color`; remove `"Color"` from the
      `canvas_types_are_builtin_types` list (`:1390`); remove `mod func_rgb;`,
      `mod func_rgba;` and their `register` calls. **`mod helper_clamp_byte;` is
      NOT removed — C2.** Also rewrote `MODULE_DESC`, which told the reader
      `canvas::rgb`/`rgba` were exempt from `Mode.Canvas` and listed `canvas::Color`
      among the bare-referenced value types.
- [x] Delete `canvas/func_rgb.rs`, `canvas/func_rgba.rs`. **`helper_clamp_byte.rs`
      is kept** (C2: `helper_items.rs:68`'s `__canvas_geoByte` calls it on the
      item-decode path); its doc comment, which explained the file as "the shared
      component clamp behind `canvas::rgb`/`rgba`", is rewritten to name the caller
      that actually keeps it alive. Orphaned-doc check: both deletions were bare
      `mod` lines with no preceding `///`
      (`git diff -U4 src/codegen/builtins/canvas/mod.rs`).
- [x] `canvas/func_fill.rs`, `func_stroke.rs`, `func_fill_stroke.rs`: param types
      to `COLOR_TYPE_ID`, bodies to `AS color::Color`.
- [x] `canvas/helper_paint_defaults.rs`: `__canvas_transparent()` returns
      `color::Color` and constructs `color::Color[...]`.
- [x] **Added (C3):** `canvas/helper_color.rs` — `__canvas_gradientStopColor` and
      `__canvas_gradientColor` return `AS color::Color` and construct
      `color::Color[...]`; `canvas/helper_items.rs:553` — `LET gc AS color::Color`.
      These name the type bare, so §2's `canvas::`-token census could not see them.
- [x] Rewrite the man examples in those files and in `func_present.rs`,
      `func_present_layers.rs` to `color::` with an `IMPORT color` line. Measured
      wider than §2 implied: **11 files, 26 name sites, 12 `IMPORT color` lines**
      (`func_create_image`, `func_did_resize`, `func_get_size`, `func_load_font`,
      `func_load_image`, `func_present`, `func_present_layers`, `func_remove_group`,
      `func_set_bytes`, `func_set_group`, and `mod.rs`'s `MODULE_DESC`).

Acceptance: `cargo test --no-fail-fast` green except for the fixtures Phase 3
updates; `mfb man canvas` shows no `Color`/`rgb`/`rgba` entries and
`mfb man canvas types` no longer lists `canvas::Color`.
Commit: `0b3fc656f`

### Phase 3 — Tests and examples

- [x] Update the **12** (not 8 — C1) Rust canvas test files' embedded programs: add
      `IMPORT color` **and** rename the calls. **346 name sites, 55 `IMPORT color`
      lines.** The programs are embedded three ways and the insertion has to match
      the style it lands in or the *Rust* stops compiling: `"IMPORT canvas\n…"`
      (escape chars on one Rust line), `IMPORT canvas\n\` + real newline (a Rust
      line-continuation with indentation), and a bare line inside `r#"…"#`. Scoped
      per program — each embedded program holds exactly one `IMPORT canvas`, so a
      canvas program that draws no colour did not gain an unused import (16
      `IMPORT canvas` in `rt_canvas_font.rs`, only 11 needed colour).
      Built, not assumed: all 12 suites run green below.
- [x] **Added (C11), discovered by the full suite:**
      `tests/cli_canvas_man_examples_compile.rs` — a 13th canvas test file that
      names `"rgb"`/`"rgba"` as bare Rust strings and so was invisible to every
      census in this plan. Its two rows are deleted, which is the visible edit its
      own doc comment asks for.
- [x] Update `examples/emoji/src/main.mfb` and run `scripts/build-examples.sh`.
- [x] Update any `tests/rt-behavior/canvas` / `tests/syntax/canvas` fixtures the
      Phase-1 census found, regenerating all four goldens per fixture. Measured:
      **there are no `tests/rt-behavior/canvas` or `tests/syntax/canvas` fixtures**
      (`find tests -type d -name '*canvas*'` → `tests/golden/canvas` (PNGs),
      `tests/syntax/resources/canvas-setgroup-consumes-items`,
      `tests/syntax/threads/canvas-drawitem-thread-plane-invalid`). The one fixture
      carrying the surface is the `resources/` one; its single golden
      (`build.log` — not four; a `syntax/` fixture has one) is regenerated. Its
      pinned diagnostic is unchanged: still `TYPE_USE_AFTER_MOVE` on the same
      binding, one line lower because of the added `IMPORT color`.
- [x] Add a `tests/syntax/canvas/` fixture pinning that `canvas::rgb(1,2,3)` is now
      a diagnostic, so the removal is a tested contract rather than an absence.
      **Two fixtures, because the compiler stops at the first unresolved name and
      the two removals are independent registrations:**
      `canvas_color_surface_removed_invalid` (the call → *"Built-in package `canvas`
      does not export `canvas.rgb`"*) and `canvas_color_type_removed_invalid` (the
      type, reported twice — once for the annotation, once for the constructor).
      Both need `"mode": "app"` in `project.json`; without it the fixture pins
      *"the `app` package requires app mode"* and never reaches the colour surface
      at all.

Acceptance: `tests/rt_canvas_rasteriser.rs` and `tests/rt_canvas_golden.rs` pass
with **pixel-identical** output; `scripts/build-examples.sh` green;
`scripts/man-run-examples.sh canvas --run` compiles and runs every canvas example.

**Met.** All 12 canvas suites green, **161 tests, 0 failures**
(`cargo test --release --no-fail-fast --test rt_canvas_*  --test cli_canvas_* …`);
`rt_canvas_golden` 19/19 with `git status --porcelain tests/golden/canvas/` **empty**
— the repo's established pixel-identity proof (plan-116-C/D/F used the same check).
`scripts/man-run-examples.sh canvas --run`: **22 examples, 22 built, 22 ran, 0
failed.** `scripts/build-examples.sh`: 54 builds archived, 6 failures, **all
attributed and pre-existing** — 5 are `examples/audio` on every target
(`libsnd.mfp` is not in git at all: `git ls-tree main -- examples/audio/packages/`
is empty) and the 6th is `emoji` on `linux-riscv64`, which fails with *"app mode
requires a macOS, Linux, or Windows target"* — a target-capability refusal emitted
before any source is read, on a build script and `project.json` this letter did not
touch. `emoji` builds on the other four targets, including the macOS `.app`.
Commit: `45f1e391a`

### Phase 4 — Docs and golden regeneration (largest blast radius)

- [x] `src/docs/spec/app/06_canvas.md` — rewrite the "value constructors are
      exempt" sentence (measured at `:425`, not `:263`). **The plan's instruction
      for this box was wrong and is not followed as written — C8.** It says that
      with `rgb`/`rgba` gone, *every* `canvas::` call requires `Mode.Canvas`. It
      does not: `canvas::fill`, `canvas::stroke` and `canvas::fillStroke` are named
      in the same exempt sentence and are still exempt, because they build a
      `canvas::Paint` value and touch no surface. The paragraph is rewritten to
      drop `rgb`/`rgba` and keep the other three, and to say why the exemption did
      not *grow* to cover colour — colour left the gated package entirely. Also
      fixed the example at `:372`.
- [x] ~~The other 3 docs the census found (`src/docs/man/types/package.md`,
      `src/docs/spec/architecture/02_frontend.md`, `:09_modules.md`,
      `src/docs/spec/package/04_type-table.md`)~~ — **moot: those four are
      plan-122-F's `TermColor` list, already rewritten there, and each contains
      zero canvas colour mentions** (C5, with the `grep -c` output). Reconciled
      against Phase 1's list as this box instructs: the canvas doc population is
      one file, `06_canvas.md`.
- [x] Regenerate `.ncode`/`.ncodesum` and `.ir`/`.ast` goldens; **attribute the
      delta** with a `git archive` attribution binary, not a sibling worktree.
      **There is no delta to attribute — C9.** `artifact-gate.sh all`: 1381 tests,
      1546 builds, **1912 goldens checked, 0 diffs**. `find tests/byte-identity
      -name '*.ncodesum' | wc -l` is still **133**, the Phase-1 figure. The
      attribution binary was still built (`git archive main | tar -x`) and used for
      the stronger check the Validation Plan asks for: the rendered frame.

Acceptance: `./scripts/test-accept.sh` full run green (watch the `N ran` count);
`scripts/artifact-gate.sh` green, re-run uncontended if it reports `exit=98`;
the golden delta is itemized in Corrections and every entry is attributable to
this letter.

**Met.** `test-accept.sh` full: **1403 ran, 0 mismatches, 0 behavioral failures,
exit 0** (up from plan-122-F's 1390 — 2 are this letter's new fixtures, the rest
arrived on main between the two letters). `artifact-gate.sh all`: **1912 goldens,
0 diffs, exit 0**, first try and uncontended. The golden delta is itemized in C9:
it is one file.
Commit: `e85143449`

## Validation Plan

Every item below was executed; the result is recorded next to it.

- **Tests:** the ~~8~~ **13** Rust canvas suites (C1, C11); the ~~new
  `tests/syntax/canvas/` removal fixture~~ **two** removal fixtures; every canvas
  man example via `man-run-examples.sh`.
  → **12 suites 161 tests / 0 failures**; both fixtures green under
  `test-accept.sh`; `man-run-examples.sh canvas --run` **22 built, 22 ran, 0
  failed**. The 13th suite (`cli_canvas_man_examples_compile`) is fixed and
  re-verified in the clean full run below.
- **Coverage check:** confirm the rewritten `canvas/func_fill.rs`,
  `func_stroke.rs`, `func_fill_stroke.rs` and `helper_paint_defaults.rs` are in
  `scripts/coverage.sh --bin mfb`'s denominator.
  → **In the denominator.** The denominator is defined by one exclusion regex,
  `IGNORE` in `scripts/coverage-common.sh:26`:
  `(^|/)(target|tests)/|_runtime_tables\.rs$|/code/private/unicode\.rs$|/src/testutil\.rs$`.
  All four paths (and the three further files this letter rewrote —
  `helper_color.rs`, `helper_clamp_byte.rs`, `mod.rs`) fail to match it, so none is
  excluded. Answered from the rule rather than by re-running the instrumented
  suite, because the rule is what decides it.
- **Runtime proof:** build and **run** `examples/emoji`; compare the rendered frame
  against the pre-change build. A green test suite is not the proof here — the
  frame is.
  → **Byte-identical, 2,304,000 bytes** (960 x 600 x 4). Method and the exact
  commands are in C10; the pre-change side is a `git archive main` attribution
  compiler building main's own `canvas::rgb` source.
- **Doc sync:** `06_canvas.md` (the exemption sentence), ~~the 3 other docs from the
  census~~ (moot — C5), and `src/docs/spec/stdlib/18_color.md` (which now owns the
  constructor documentation).
  → `06_canvas.md`: the exemption paragraph rewritten (C8 — it keeps three of its
  five members) and its example moved to `color::rgb`. `18_color.md`: gains a
  **Cross-package use** table naming every member in the language that speaks
  `color::Color`, measured with
  `grep -rn 'COLOR_TYPE_ID' src/codegen/builtins/ | grep -v '/color/'` — `term`
  (4 members), `astrings` (2), `canvas` (3 members + 3 record fields) — and states
  that none of them constructs one, so `color`'s constructors are the only way and
  are gated by nothing.
- **Acceptance:** `cargo test --no-fail-fast`; `./scripts/test-accept.sh` full;
  `scripts/artifact-gate.sh`; `scripts/build-examples.sh`;
  `cargo check --all-targets` at the end; `cargo fmt`.
  → Results in the Acceptance ledger below.

## Acceptance ledger

| Gate | Result |
|---|---|
| `test-accept.sh` full | **1403 ran, 0 mismatches, 0 behavioral failures**, exit 0 |
| `artifact-gate.sh all` | **1381 tests, 1546 builds, 1912 goldens, 0 diffs**, exit 0, uncontended |
| `man-run-examples.sh canvas --run` | **22 examples, 22 built, 22 ran, 0 failed** |
| `build-examples.sh` | 54 archived; 6 failures, all attributed pre-existing (C-note in Phase 3) |
| emoji frame vs. pre-change build | **byte-identical**, 2,304,000 bytes |
| `tests/golden/canvas/` (7 reference PNGs) | **untouched** — `git status --porcelain` empty |
| `cargo test --release --no-fail-fast` | see below |
| `cargo check --all-targets` | see below |
| `cargo fmt --all` + `repository/` | see below |

## Open Decisions

Both resolved during execution; the recommendation was taken in each case.

- **Whether `canvas::fill`/`stroke`/`fillStroke` should also move to `color`.**
  Recommend no: they build a `canvas::Paint`, which is a canvas concept. They take
  a `color::Color` and stay in canvas. (§2)
  → **Taken: they stay in `canvas`.** Executing found a second, stronger reason
  than the ownership argument. These three are the *only* `canvas::` members left
  in the `Mode.Canvas` exemption (C8), so moving them to `color` would have made
  every remaining `canvas::` call mode-gated — which is exactly the state the plan
  wrongly assumed already held, and would have cost the exemption its last
  legitimate members. `06_canvas.md`'s rewritten paragraph documents them as
  exempt.
- **Whether the removal fixture belongs in `tests/syntax/canvas/` or
  `tests/syntax/color/`.** Recommend `canvas/`, since it pins canvas's surface. (Phase 3)
  → **Taken: `tests/syntax/canvas/`**, a new directory this letter creates (there
  was no canvas fixture directory before — C9). Two fixtures rather than one, since
  the compiler stops at the first unresolved name.

## Corrections

### C1 — every §2 population count was low; the measured census (2026-09-05)

§2's counts do not reproduce. Re-measured on the worktree base with the plan's own
commands:

| What | §2 said | Measured | Command |
|---|---|---|---|
| `canvas::rgb(` in `src` | 19 | **29** (28 code + 1 doc) | `grep -rn 'canvas::rgb(' src \| wc -l` |
| `canvas::rgb(` in `tests` | 110 | **281** | `grep -rn 'canvas::rgb(' tests \| wc -l` |
| `canvas::rgb(` in `examples` | 8 | **9** | `grep -rn 'canvas::rgb(' examples \| wc -l` |
| `canvas::rgba(` in `src` / `tests` | 2 / 11 | **2 / 11** (agrees) | `grep -rn 'canvas::rgba(' src\|tests \| wc -l` |
| `canvas::Color` in `src` / `tests` / `examples` | 24 / 27 / 5 | **28 / 34 / 5** | `for d in src tests examples; do grep -rn 'canvas::Color' $d \| wc -l; done` |
| Rust test files naming the surface | 8 | **12** | `grep -rln 'canvas::rgb\|canvas::Color' tests/*.rs` |
| example `.mfb` files | 1 | **1** (agrees) | `grep -rl 'canvas::rgb\|canvas::Color' --include='*.mfb' examples/` |
| docs naming the surface | 4 | **2** | `grep -rln 'canvas::rgb\|canvas::Color' src/docs/` |
| `tests/byte-identity` `.ncodesum` | 133 | **133** (agrees) | `find tests/byte-identity -name '*.ncodesum' \| wc -l` |

The scope is roughly **2.4x** the plan's estimate on call sites and **1.5x** on Rust
test files. No phase is re-split: it is the same work, more of it.

### C2 — `__canvas_clampByte` has a live caller; §2's "delete" is wrong

§2 says *"`canvas/helper_clamp_byte.rs` (whole file) — delete — it moved to `color`
in plan-122-A Phase 1 and **has no other caller**."* That last clause is false:

```
$ grep -rn '__canvas_clampByte' src tests
src/codegen/builtins/canvas/helper_clamp_byte.rs:1:  (declaration)
src/codegen/builtins/canvas/helper_clamp_byte.rs:12: (declaration)
src/codegen/builtins/canvas/func_rgba.rs:56:         (goes away with func_rgba)
src/codegen/builtins/color/helper_clamp_byte.rs:16:  (a doc comment, not a call)
src/codegen/builtins/canvas/helper_items.rs:68:      RETURN __canvas_clampByte(toInt(__canvas_geoAt(offset, slot)))
```

`helper_items.rs:68` is a live call on the item-decode path, unrelated to `rgb`/
`rgba`. **`helper_clamp_byte.rs` is KEPT.** Deleting it as §2 instructed would have
failed to build. Phase 2's box is amended in place rather than marked moot, because
the decision changed, not the task.

### C3 — four canvas-internal `AS Color` / `Color[…]` sites §2 does not list

§2 claims the only canvas internals touching the record are
`helper_geometry.rs:188-195` and `helper_items.rs:51`, and that both are unchanged
because the field names match. True for those two — but the census found four more
sites that name the **type**, not a field, and so *do* change:

- `canvas/helper_color.rs:119` — `FUNC __canvas_gradientStopColor(at AS Integer) AS Color`
- `canvas/helper_color.rs:120` — `RETURN Color[red := …]`
- `canvas/helper_color.rs:123` — `FUNC __canvas_gradientColor(base, count, t) AS Color`
- `canvas/helper_color.rs:172` — `RETURN Color[red := …]`
- `canvas/helper_items.rs:553` — `LET gc AS Color = __canvas_gradientColor(…)`

Found with `grep -rn 'AS Color\b\|Color\[' src/codegen/builtins/canvas/*.rs`, which
is the grep §2 should have run — its census was by `canvas::` token, and a
companion body names the type **bare** (`AS Color`), with no `canvas::` on it. These
are added to Phase 2.

### C4 — `src/ir/shape.rs:4301` is not a canvas site

The Phase-1 grep matches `ParameterType::named("Color")` at `src/ir/shape.rs:4301`.
It is not canvas: the enclosing test `package_type_validation_arms`
(`src/ir/shape.rs:4262`) builds a fake package from the source at `:4262`, which
declares its own `ENUM Color\n  Red, Green\nEND ENUM`. The assertion is that a
*declared* nominal walks silently. **Untouched.**

### C5 — Phase 4's "other 3 docs" are plan-122-F's list, not D's

Phase 4 names `src/docs/man/types/package.md`,
`src/docs/spec/architecture/02_frontend.md`, `:09_modules.md` and
`src/docs/spec/package/04_type-table.md`. None of them mentions the canvas colour
surface:

```
$ for f in src/docs/man/types/package.md src/docs/spec/architecture/02_frontend.md \
           src/docs/spec/architecture/09_modules.md src/docs/spec/package/04_type-table.md; do
    printf '%s: ' "$f"; grep -c 'canvas::rgb\|canvas::Color\|canvas\.Color' "$f"; done
src/docs/man/types/package.md: 0
src/docs/spec/architecture/02_frontend.md: 0
src/docs/spec/architecture/09_modules.md: 0
src/docs/spec/package/04_type-table.md: 0
```

That is `TermColor`'s doc list, which **plan-122-F already rewrote** (its Phase 5
names exactly these four). Phase 4's box is resolved against Phase 1's list, as the
box itself instructs.

`src/docs/spec/stdlib/18_color.md:11` names `canvas::Color` in a sentence about what
existed *before* the package ("Before this package MFBASIC had three unrelated
notions…"). It is a historical statement and stays correct; **untouched**.

### C7 — `canvas::GradientStop.color` is a fifth descriptor site §2 misses

§2's table lists exactly two `ParameterType::named("Color")` descriptor sites,
`Paint.fill` and `Paint.stroke`. There are three:

```
$ grep -n 'named("Color")' src/codegen/builtins/canvas/mod.rs
482:                ty: ParameterType::named("Color"),   <- GradientStop.color
544:                ty: ParameterType::named("Color"),   <- Paint.fill
550:                ty: ParameterType::named("Color"),   <- Paint.stroke
```

`GradientStop.color` (`canvas/mod.rs:480-484`) is the colour at one gradient offset.
It repoints to `COLOR_TYPE_ID` exactly like the other two. Missing it would have
left a record prop typed by a leaf whose record no longer exists — and because
`qualify_type_leaves_for_source` rewrites record field types for rendering
(§3 seam 2), the failure would have surfaced as an unresolved type in every
importer's companion, not as a compile error in `canvas/mod.rs`.

§2's stated line numbers are also stale throughout (`:183`/`:458`/`:464`/`:1110`
vs. the measured `:197`/`:544`/`:550`/`:1390`); the census table above carries the
measured ones.

### C6 — letter order: D ran last, not third

D was authored to run before E and F. It ran after them (E and F landed
2026-09-04, D started 2026-09-05). Nothing depended on the authored order: D cites
no term or astrings symbol, and E/F cite no canvas file. §Prerequisites records the
re-measurement.

### C8 — Phase 4's rewrite instruction is wrong: three constructors stay exempt

Phase 4 says to rewrite `06_canvas.md`'s exemption sentence on the grounds that
*"with `rgb`/`rgba` gone, **every** `canvas::` call requires `app::Mode.Canvas`"*.
That is false, and I wrote the same false claim into canvas's `MODULE_DESC` in
Phase 2 before measuring. The sentence names five members, not two:

> The **value constructors are exempt**: `canvas::rgb`, `canvas::rgba`,
> `canvas::fill`, `canvas::stroke` and `canvas::fillStroke` build values and touch
> no surface…

`fill`/`stroke`/`fillStroke` survive this letter and are still exempt. The gate is
per-member and measurable — it is carried on the descriptor, not on the package:

```
$ grep -rn 'ModeRequirement::Canvas' src/ --include='*.rs' | grep -v hook/app.rs
src/codegen/builtins/canvas/func_create_image.rs:186
src/codegen/builtins/canvas/gen_present.rs:314
src/codegen/builtins/canvas/gen_group.rs:340
src/codegen/builtins/canvas/gen_group.rs:452
src/codegen/builtins/canvas/func_did_resize.rs:95
src/codegen/builtins/canvas/func_scene_hashes.rs:75
```

Six surface members carry `ModeRequirement::Canvas`. The three `Paint`
constructors are `Body::mfb` companions and carry none, exactly as before.

So the doc change is **not** "the exemption is removed". It is: the exemption
loses two of its five members, and the reason is that colour construction left the
gated package altogether — `color` is gated by nothing, in any build, which is a
strictly better outcome than an exemption. Both `MODULE_DESC` and `06_canvas.md`
now say that. Had the box been executed as written, `mfb man canvas` would have
told every reader that `canvas::fill` raises `ErrWrongMode` outside canvas mode,
which is not what the compiler does.

### C9 — Phase 4 predicted wide golden drift; the measured drift is one file

Section 3 states: *"canvas `.ncode`/`.ncodesum` and every canvas importer's
`.ir`/`.ast` are **expected** to drift."* Measured: **zero.** `artifact-gate.sh
all` reports 1912 goldens and 0 diffs; `test-accept.sh` full reports 1403 ran and
0 mismatches. The only golden this letter changed is
`tests/syntax/resources/canvas-setgroup-consumes-items/golden/build.log`, synced
in Phase 3.

Per the standing rule, an unexpected *absence* of drift gets the same scrutiny as
an unexpected presence — a gate that silently never ran canvas would look exactly
like this. It did not; the reason is structural and measurable:

```
$ ls tests/byte-identity/
audio bits collections crypto csv datetime encoding fs general http io json
link-const-pins math money net os process regex resource-xfer-slots strings tcp
term thread tls udp vector
$ grep -rl '^IMPORT canvas' --include='*.mfb' tests/
tests/syntax/canvas/canvas_color_surface_removed_invalid/src/main.mfb
tests/syntax/canvas/canvas_color_type_removed_invalid/src/main.mfb
tests/syntax/resources/canvas-setgroup-consumes-items/src/main.mfb
tests/syntax/threads/canvas-drawitem-thread-plane-invalid/src/main.mfb
```

**There is no `tests/byte-identity/canvas` directory** — canvas has never had
`.ncode`/`.ncodesum` goldens, so there was nothing there to drift. And canvas is
importable only in `--app` builds, so exactly **four** fixtures in the whole tree
import it, each carrying a single `build.log`. Two of those four are this letter's
own new fixtures and the fourth (`canvas-drawitem-thread-plane-invalid`) never
named the colour surface.

Canvas's real regression coverage is not the golden harness at all — it is the 12
Rust suites and the seven reference PNGs under `tests/golden/canvas/`, which is
why Phase 3's acceptance, not Phase 4's, is where this letter was actually at risk.

### C10 — the emoji frame, compared rather than assumed

The Validation Plan asks for the rendered frame, not a green suite. Done with a
`git archive main | tar -x` attribution binary (never a sibling worktree), which
compiles `examples/emoji`'s **pre-change** source (`canvas::rgb`, 9 sites) while
this worktree compiles the **post-change** source (`color::rgb`):

```
$ MFB_CANVAS_DUMP=... MFB_CANVAS_SYNC=1 <emoji binary>     # both, bounded at 60s
pre:  dump 2304000 bytes
post: dump 2304000 bytes
$ cmp /tmp/pD-f2-pre.rgba /tmp/pD-f2-post.rgba   ->  exit 0
FRAME IDENTICAL: 2304000 bytes
```

Both runs were bounded and killed identically rather than one being allowed to
finish — `emoji` is a GUI event loop and does not self-terminate, so an asymmetric
comparison (one completed, one killed) would not have been a comparison of the
same thing. 2,304,000 bytes is 960 x 600 x 4 (RGBA8).

### C11 — a 13th canvas test file, invisible to every census in this plan

`tests/cli_canvas_man_examples_compile.rs` names the removed members as **bare
strings in a Rust array**, with no `canvas::` token and no MFBASIC anywhere:

```rust
const MEMBERS: &[&str] = &[
    "rgb",
    "rgba",
    "fill",
    ...
```

It renders `mfb man canvas <member>` for each and compiles the example it finds.
So it does not appear in §2's population, it does not appear in Phase 1's census
(which greps `canvas::rgb(`, `canvas::Color`, `AS Color` and `named("Color")` —
none of which match a bare `"rgb"`), and it is not one of the 12 files Phase 3
migrated. It failed in the full suite with:

```
mfb man canvas rgb failed:
error: unknown canvas function `rgb`
```

**Fixed by deleting the two rows, which is what the list is designed for.** Its own
doc comment: *"Kept explicit rather than discovered, so removing a member's example
is a visible edit here rather than a silently shrinking test."* Deleting a row is
the sanctioned edit; the alternative — making the list discovered — would defeat the
property the file exists to hold. A comment records where the examples went
(`mfb man color rgb` renders them; both `color::rgb` and `color::rgba` have an
Examples section).

**The lesson, which cost a red suite in plan-122-F for the same reason and again
here.** F's census missed MFBASIC embedded in Rust *strings*; D's census missed a
member named as a Rust *string literal in a list*. A census for a removal is not
over the call syntax — it is over every place the member's NAME can appear,
including places that contain no code in this language at all. The census that
would have caught it is `grep -rn '"rgb"\|"rgba"' src tests --include='*.rs'`,
which returns exactly three files: `color/func_rgb.rs`, `color/func_rgba.rs`, and
this one.

### The census, grouped by the phase that executes it

**Phase 2 — `src/codegen/builtins/canvas/` (16 files)**

| File | Why |
|---|---|
| `mod.rs` | `Color` record decl (`:197`); **`GradientStop.color` `:482`** (C7); `Paint.fill` `:544` and `Paint.stroke` `:550`; the `"Color"` entry in `canvas_types_are_builtin_types` (`:1390`); `mod func_rgb;`/`mod func_rgba;` (`:61-62`) and their `register` calls (`:1029-1030`); the three prop descriptions that say `canvas::Color` |
| `func_rgb.rs` | delete |
| `func_rgba.rs` | delete |
| `helper_clamp_byte.rs` | **KEEP** (C2) |
| `func_fill.rs` | param type `:86`, body `:69`, man example |
| `func_stroke.rs` | param type `:60`, body `:42`, man example |
| `func_fill_stroke.rs` | param types `:92`,`:99`, body `:74`, man example |
| `helper_paint_defaults.rs` | `:15-16` return type + construction |
| `helper_color.rs` | `:119`,`:120`,`:123`,`:172` (C3) |
| `helper_items.rs` | `:553` (C3). `:51` and `:68` unchanged |
| `func_present.rs`, `func_present_layers.rs`, `func_set_group.rs`, `func_remove_group.rs`, `func_set_bytes.rs`, `func_load_image.rs`, `func_load_font.rs`, `func_get_size.rs`, `func_did_resize.rs`, `func_create_image.rs` | man examples calling `canvas::rgb` — each needs `IMPORT color` |

**Phase 3 — tests and examples (15 paths)**

12 Rust files carrying MFBASIC as string literals, with their `canvas::rgb(` counts:
`rt_canvas_rasteriser.rs` (115), `rt_canvas_golden.rs` (59), `rt_canvas_metal.rs`
(35), `rt_canvas_group_ownership.rs` (25), `rt_canvas_font.rs` (12),
`rt_canvas_damage.rs` (10), `rt_canvas_graphics_thread.rs` (8),
`cli_canvas_package.rs` (8), `rt_canvas_present_deep_copy.rs` (2),
`cli_app_canvas_mode.rs` (2), `codegen_canvas_thread_entry_saves_rbp.rs` (1),
`cli_canvas_image_resource.rs` (1). Plus `examples/emoji/src/main.mfb` (9 `rgb`, 5
`Color`), the fixture `tests/syntax/resources/canvas-setgroup-consumes-items/`
(`src/main.mfb` 2 hits, `golden/build.log` 1), and the new removal fixture.

**Phase 4 — docs (1 file)**

`src/docs/spec/app/06_canvas.md`: `:372` (an example calling `canvas::rgb`) and
`:425` (the "value constructors are exempt" sentence). Nothing else — see C5.

## Summary

The rename is safe by construction: identical field set, no wire id, no backend
reading the record. The risk is coverage — 8 Rust files carrying MFBASIC as string
literals, each of which needs an added `IMPORT` line rather than a substitution,
and where the failure mode is a field read rather than the call. Phase 1 exists to
make that list complete before any of it is edited.

Untouched: term and astrings, which are E and F.
