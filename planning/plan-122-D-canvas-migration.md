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
| plan-122-C complete | `ls planning/completed/plan-122-C-*` → one match | NOT MET |
| canvas already imports `color` (landed in B) | `grep -n 'add_imports' -A 8 src/codegen/builtins/canvas/mod.rs` shows `"color"` | NOT MET |

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

- [ ] Enumerate every site with `grep -rn 'canvas::rgb(\|canvas::rgba(\|canvas::Color\|ParameterType::named("Color")' src tests examples src/docs`
      and write the file list into this document's Corrections section, grouped by
      phase. **Do not census by call name alone** — a `Paint[fill := c]` with `c`
      built elsewhere has no `canvas::` token on it at all, so also grep for
      `paint.fill`/`paint.stroke` readers and for `AS Color` in embedded programs.
- [ ] Record the pre-change canvas `.ncodesum` set so Phase 4's drift is
      attributable: `find tests/byte-identity -name '*.ncodesum' | wc -l` → 133 today.

Acceptance: a written file list in Corrections whose counts match the §2 table. A
count that disagrees is a census bug — reconcile it before Phase 2.
Commit: —

### Phase 2 — Descriptor and companion

- [ ] `canvas/mod.rs`: delete the `Color` record (`:183-210`); repoint `Paint.fill`
      and `Paint.stroke` at `COLOR_TYPE_ID` (`:458`, `:464`) and rewrite their
      descriptions to say `color::Color`; remove `"Color"` from the
      `canvas_types_are_builtin_types` list (`:1110`); remove `mod func_rgb;`,
      `mod func_rgba;`, `mod helper_clamp_byte;` and their `register` calls.
- [ ] Delete `canvas/func_rgb.rs`, `canvas/func_rgba.rs`, `canvas/helper_clamp_byte.rs`.
      **Check for an orphaned doc comment** at each deletion site — a deleted item's
      `///` silently merges onto its neighbour.
- [ ] `canvas/func_fill.rs`, `func_stroke.rs`, `func_fill_stroke.rs`: param types
      to `COLOR_TYPE_ID`, bodies to `AS color::Color`.
- [ ] `canvas/helper_paint_defaults.rs`: `__canvas_transparent()` returns
      `color::Color` and constructs `color::Color[...]`.
- [ ] Rewrite the man examples in those files and in `func_present.rs`,
      `func_present_layers.rs` to `color::` with an `IMPORT color` line.

Acceptance: `cargo test --no-fail-fast` green except for the fixtures Phase 3
updates; `mfb man canvas` shows no `Color`/`rgb`/`rgba` entries and
`mfb man canvas types` no longer lists `canvas::Color`.
Commit: —

### Phase 3 — Tests and examples

- [ ] Update the 8 Rust canvas test files' embedded programs: add `IMPORT color`
      **and** rename the calls. Build each one, do not assume.
- [ ] Update `examples/emoji/src/main.mfb` and run `scripts/build-examples.sh`.
- [ ] Update any `tests/rt-behavior/canvas` / `tests/syntax/canvas` fixtures the
      Phase-1 census found, regenerating all four goldens per fixture.
- [ ] Add a `tests/syntax/canvas/` fixture pinning that `canvas::rgb(1,2,3)` is now
      a diagnostic, so the removal is a tested contract rather than an absence.

Acceptance: `tests/rt_canvas_rasteriser.rs` and `tests/rt_canvas_golden.rs` pass
with **pixel-identical** output; `scripts/build-examples.sh` green;
`scripts/man-run-examples.sh canvas --run` compiles and runs every canvas example.
Commit: —

### Phase 4 — Docs and golden regeneration (largest blast radius)

- [ ] `src/docs/spec/app/06_canvas.md` — rewrite `:263`'s "value constructors are
      exempt" sentence: with `rgb`/`rgba` gone, **every** `canvas::` call requires
      `app::Mode.Canvas`, and colour construction happens in `color` which needs no
      mode at all.
- [ ] The other 3 docs the census found (`src/docs/man/types/package.md`,
      `src/docs/spec/architecture/02_frontend.md`, `:09_modules.md`,
      `src/docs/spec/package/04_type-table.md` — reconcile against Phase 1's list).
- [ ] Regenerate `.ncode`/`.ncodesum` and `.ir`/`.ast` goldens; **attribute the
      delta** with a `git archive` attribution binary, not a sibling worktree.
      Confirm the delta is confined to canvas fixtures and canvas importers and
      contains no `build.log`/`.run` behavior change.

Acceptance: `./scripts/test-accept.sh` full run green (watch the `N ran` count);
`scripts/artifact-gate.sh` green, re-run uncontended if it reports `exit=98`;
the golden delta is itemized in Corrections and every entry is attributable to
this letter.
Commit: —

## Validation Plan

- **Tests:** the 8 Rust canvas suites; the new `tests/syntax/canvas/` removal
  fixture; every canvas man example via `man-run-examples.sh`.
- **Coverage check:** confirm the rewritten `canvas/func_fill.rs`,
  `func_stroke.rs`, `func_fill_stroke.rs` and `helper_paint_defaults.rs` are in
  `scripts/coverage.sh --bin mfb`'s denominator.
- **Runtime proof:** build and **run** `examples/emoji`; compare the rendered frame
  against the pre-change build. A green test suite is not the proof here — the
  frame is.
- **Doc sync:** `06_canvas.md` (the exemption sentence), the 3 other docs from the
  census, and `src/docs/spec/stdlib/18_color.md` (which now owns the constructor
  documentation).
- **Acceptance:** `cargo test --no-fail-fast`; `./scripts/test-accept.sh` full;
  `scripts/artifact-gate.sh`; `scripts/build-examples.sh`;
  `cargo check --all-targets` at the end; `cargo fmt`.

## Open Decisions

- **Whether `canvas::fill`/`stroke`/`fillStroke` should also move to `color`.**
  Recommend no: they build a `canvas::Paint`, which is a canvas concept. They take
  a `color::Color` and stay in canvas. (§2)
- **Whether the removal fixture belongs in `tests/syntax/canvas/` or
  `tests/syntax/color/`.** Recommend `canvas/`, since it pins canvas's surface. (Phase 3)

## Corrections

_(filled in during execution — Phase 1's census file list goes here)_

## Summary

The rename is safe by construction: identical field set, no wire id, no backend
reading the record. The risk is coverage — 8 Rust files carrying MFBASIC as string
literals, each of which needs an added `IMPORT` line rather than a substitution,
and where the failure mode is a field read rather than the call. Phase 1 exists to
make that list complete before any of it is edited.

Untouched: term and astrings, which are E and F.
