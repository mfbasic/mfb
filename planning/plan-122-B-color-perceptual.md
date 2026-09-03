# plan-122-B: Linear-light colour maths — the sRGB seam, brighten/darken, HSL

Last updated: 2026-09-02
Effort: large (3h–1d)
Depends on: plan-122-A

`color` gains the maths that make it a colour package rather than a struct: the
sRGB↔linear transfer, perceptual `brighten`/`darken`/`mix`, WCAG `luminance` and
`contrastRatio`, and the HSL model with `saturate`/`desaturate`/`rotateHue`.

The sRGB transfer table lives in canvas today
(`src/codegen/builtins/canvas/helper_color.rs:27`, a 256-entry literal deliberately
tabulated so the software rasteriser is bit-identical on every target). This letter
**moves it into `color`** and exposes it as the public pair
`color::toLinear`/`color::fromLinear`, then repoints canvas's blend functions at
that pair. That is the one edit in plan-122 that touches the rasteriser, and its
gate is that rendered pixels are unchanged.

Behavioral outcome: `color::brighten(c, 0.5)` lightens perceptually rather than
numerically, `color::contrastRatio(fg, bg)` returns the WCAG ratio, and **every
canvas raster golden renders byte-for-byte the same pixels as before the move**.

References:

- plan-122-A — Prerequisites (stated once there), the `Color` record, `COLOR_TYPE_ID`.
- `src/codegen/builtins/canvas/helper_color.rs` — the table, the binary-search
  inverse, the two blend functions, and the four Rust unit tests that pin the
  table's length, endpoints, monotonicity and agreement with the transfer function.
- `.ai/canvas-threading.md` — read before touching anything the graphics thread runs.
- `src/docs/spec/app/06_canvas.md` §"Rendering conventions" — defines the blend
  modes on **linear** values, which is the contract the move must preserve.
- WCAG 2.2 §"Relative luminance" and §"Contrast ratio" — the definitions
  `luminance`/`contrastRatio` implement.

## Prerequisites

Stated once in plan-122-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-122-A complete | `ls planning/completed/plan-122-A-*` → one match | NOT MET |

If plan-122-A is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- `color::toLinear(channel AS Byte) AS Integer` and
  `color::fromLinear(value AS Integer) AS Byte` are public members backed by the
  256-entry table moved out of canvas, and produce **exactly** the values
  `__canvas_srgbTable`/`__canvas_linearToSrgb` produce today.
- canvas's `__canvas_blendChannel` and `__canvas_blendChannelMode` call that pair
  instead of indexing a canvas-local table, and every canvas raster golden is
  pixel-identical.
- `color::brighten`, `darken`, `mix`, `grayscale`, `luminance`, `contrastRatio`,
  `isDark`, `isLight` exist and are documented.
- `color::Hsl` exists with `color::hsl`, `hsla`, `toHsl`, `saturate`, `desaturate`,
  `rotateHue`.

### Non-goals (explicit constraints)

- **No change to any rendered pixel.** The move is value-preserving by
  construction: the same 256 literals, the same binary search, the same rounding.
- **No transcendentals.** `pow`, `exp`, `log` and every trig function stay off the
  path, because canvas's GPU backends are compared against the software rasteriser
  as an oracle and libm differs across platforms (`helper_color.rs:1-11`). The HSL
  formulas and the WCAG coefficients are pure `+ - * /`.
- **`BlendMode.Normal` stays bit-for-bit what it is.** `helper_color.rs:88-95`
  states the contract: mode `0` is not merely *equivalent* to
  `__canvas_blendChannel`, it is the same expression, so it cannot drift by a
  rounding step. The move must keep that identity.
- **No consumer API change.** `canvas::Color` is still canvas's type in this
  letter; renaming it is D.

## 2. Current State

`src/codegen/builtins/canvas/helper_color.rs` registers four `always` helpers:

| Helper | What | Line |
|---|---|---|
| `canvas_srgbTable` | `FUNC __canvas_srgbTable() AS List OF Integer` returning 256 literals, plus `LET __CANVAS_SRGB AS List OF Integer = __canvas_srgbTable()` | `:22-27` |
| `canvas_linearToSrgb` | binary search over that table, 8 comparisons, returns the nearest sRGB byte | `:36-52` |
| `canvas_blendChannel` | one channel of the over-operator in linear space | `:60-67` |
| `canvas_blendChannelMode` | the same with a `BlendMode` applied to the source first | `:88-127` |

The table is read through `collections::getOr(__CANVAS_SRGB, toInt(dst), 0)` — a
`getOr` with a `0` default, which is why a truncated table silently blended toward
black rather than failing (the defect `srgb_table_covers_every_byte` was written to
catch, `:150-160`).

Four Rust unit tests in that file's `mod tests` parse the literal out of the const
and pin: 256 entries, exact endpoints `0`/`65535`, strict monotonicity (required
for the binary search to be correct), and agreement with the standard transfer
function at every index.

### Measured populations

| What | Count | Command |
|---|---|---|
| `__CANVAS_SRGB` reference sites in canvas source bodies | 4 | `grep -rn '__CANVAS_SRGB' src/codegen/builtins/canvas/ \| wc -l` |
| Rust unit tests pinning the table | 4 | `grep -c '#\[test\]' src/codegen/builtins/canvas/helper_color.rs` |
| canvas raster/golden test files | 8 | `ls tests/rt_canvas_*.rs \| wc -l` |
| module-level `LET __` globals in canvas's companion | 19 | `grep -c '^LET __' src/codegen/builtins/canvas/*.rs \| awk -F: '{s+=$2} END {print s}'` |

### Verified properties

- **A package cannot reach another package's private `__` members.** Helper FUNCs
  are internalized per package (`#astrings_packColor` in an importer's `.ir`:
  `grep -o '"name": *"[^"]*"' tests/rt-behavior/astrings/attribute-model-rt/golden/attribute_model_rt.ir`).
  This is why the table is exposed as the **public** `toLinear`/`fromLinear` pair
  rather than moved as a private helper canvas reaches across the boundary.
- **canvas already has a non-empty companion and already pays a companion cost**,
  so `add_imports(["color"])` on canvas adds `color`'s companion but does not
  change canvas's cost *class* (§2 of plan-122-A's measurement table). `term` is
  the package where that would have mattered, and F keeps term out of it.
- **UNVERIFIED — the rasteriser's per-pixel cost after the move.** The call was
  already a `collections::getOr` through a module-level list; it becomes a call to
  a member FUNC that does the same `getOr`. Phase 3 measures it rather than
  assuming.

## 3. Design Overview

Three pieces:

1. **The sRGB seam (highest risk, scheduled first).** Move the table and the
   binary-search inverse into `color`; expose `toLinear`/`fromLinear`; rewrite
   canvas's two blend functions to call them. The four Rust unit tests move with
   the table. Risk concentrates here because it is the only edit in plan-122 that
   can change a rendered pixel — and because a wrong value does not fail, it
   *blends toward black*.
2. **Perceptual operations.** `brighten`/`darken`/`mix`/`grayscale` in linear
   light; `luminance`/`contrastRatio`/`isDark`/`isLight` from the WCAG
   definitions. All built on the seam from (1), so they cannot disagree with what
   canvas renders.
3. **HSL.** A `Hsl` record and its conversions plus the three manipulators. Pure
   arithmetic; independent of (1) and (2); lowest risk, scheduled last.

**Byte-identity is not the gate, but pixel-identity is.** canvas's `.ncode` will
drift in Phase 1 (the blend bodies change shape) and that drift is the plan
working. What must **not** change is rendered output: `tests/rt_canvas_rasteriser.rs`
and `tests/rt_canvas_golden.rs` compare pixels, and those are the acceptance
criterion. A pixel diff means root-cause it (dump one fixture's pixels, compare the
table values index by index), never "the move is impossible".

### Rejected alternatives

- **Duplicate the table in `color` and leave canvas's alone.** Rejected: two
  tables that must agree forever is exactly the drift this plan exists to remove,
  and `color::luminance` disagreeing with what canvas renders would be a bug nobody
  could see.
- **Compute the transfer function at runtime with `pow`.** Rejected outright by
  `helper_color.rs:1-11`: it puts a libm transcendental on the rasteriser path and
  makes the GPU oracle platform-dependent.
- **`brighten` as a plain channel scale in sRGB space.** Rejected (user decision,
  2026-09-02): it is not perceptually uniform, and it would leave two notions of
  "lighter" in one tree.

## 4. The sRGB seam

New in `color`:

```
FUNC __color_srgbTable() AS List OF Integer
  RETURN [0, 20, 40, …, 65535]          ' the 256 literals, moved verbatim
END FUNC

LET __COLOR_SRGB AS List OF Integer = __color_srgbTable()
```

Public members:

| Member | Signature | Body |
|---|---|---|
| `toLinear` | `(channel AS Byte) AS Integer` | `collections::getOr(__COLOR_SRGB, toInt(channel), 0)` |
| `fromLinear` | `(value AS Integer) AS Byte` | the binary search moved verbatim from `__canvas_linearToSrgb` (`helper_color.rs:36-52`) |

canvas's blend functions become:

```
FUNC __canvas_blendChannel(dst AS Byte, src AS Byte, alpha AS Integer) AS Byte
  LET dstLin AS Integer = color::toLinear(dst)
  LET srcLin AS Integer = color::toLinear(src)
  LET mixed AS Integer = dstLin + ((srcLin - dstLin) * alpha + 127) / 255
  RETURN color::fromLinear(mixed)
END FUNC
```

— the same expression with the two lookups and the inverse behind calls. The
`+ 127` round-to-nearest and the `/ 255` (not 256) stay exactly as written; the
`__canvas_blendChannelMode` `Normal` arm keeps sharing that expression literally,
per the compatibility contract at `helper_color.rs:88-95`. The `/ 65535`
(not 65536) in the `Multiply`/`Screen` arms is likewise untouched — dividing by
65536 would make `multiply(x, white)` come out one step below `x`.

Linear values stay in `0..65535`. That range is part of the public contract of
`toLinear`/`fromLinear` and is documented on both man pages.

## 5. Perceptual operations

| Member | Signature | Definition |
|---|---|---|
| `brighten` | `(base AS Color, amount AS Float) AS Color` | each channel `lin + (65535 - lin) * amount`, `amount` clamped to `0.0..1.0`; alpha untouched |
| `darken` | `(base AS Color, amount AS Float) AS Color` | each channel `lin - lin * amount` |
| `mix` | `(first AS Color, second AS Color, amount AS Float) AS Color` | per-channel linear interpolation; `amount` `0.0` yields `first`, `1.0` yields `second`; alpha interpolated too |
| `grayscale` | `(base AS Color) AS Color` | every channel set to `fromLinear` of the relative luminance; alpha untouched |
| `luminance` | `(base AS Color) AS Float` | `(2126*r + 7152*g + 722*b) / 10000 / 65535.0` on **linear** channels — WCAG relative luminance, `0.0`..`1.0` |
| `contrastRatio` | `(first AS Color, second AS Color) AS Float` | `(hi + 0.05) / (lo + 0.05)` where `hi`/`lo` are the larger/smaller `luminance`; `1.0`..`21.0` |
| `isDark` | `(base AS Color) AS Boolean` | `luminance(base) < 0.5` |
| `isLight` | `(base AS Color) AS Boolean` | `NOT isDark(base)` |

`brighten`/`darken` ignore `alpha` deliberately: lightening a colour must not make
it more opaque. `mix` interpolates alpha because it is a blend of two whole
colours. Both rules go on the man pages, because both are the kind of thing a
caller otherwise discovers by surprise.

Endpoint exactness is a stated contract and a test: `brighten(c, 0.0) = c`,
`brighten(c, 1.0) = white-with-c's-alpha`, `darken(c, 1.0) = black-with-c's-alpha`,
`mix(a, b, 0.0) = a`, `mix(a, b, 1.0) = b`. The `+ 127` round-to-nearest idiom the
canvas blend uses is what makes these exact rather than one step short.

## 6. HSL

```
TYPE Hsl
  hue AS Float          ' 0.0 .. 360.0, degrees
  saturation AS Float   ' 0.0 .. 1.0
  lightness AS Float    ' 0.0 .. 1.0
END TYPE
```

| Member | Signature |
|---|---|
| `hsl` | `(hue, saturation, lightness AS Float) AS Color` (alpha 255) |
| `hsla` | `(hue, saturation, lightness AS Float, alpha AS Integer) AS Color` |
| `toHsl` | `(base AS Color) AS Hsl` |
| `saturate` | `(base AS Color, amount AS Float) AS Color` |
| `desaturate` | `(base AS Color, amount AS Float) AS Color` |
| `rotateHue` | `(base AS Color, degrees AS Float) AS Color` |

HSL is computed on **sRGB** channels, not linear — that is what CSS `hsl()` and
every design tool mean by it, and a `toHsl`/`hsl` round-trip that did not agree
with the hex a designer pasted in would be useless. This is a deliberate
asymmetry with §5 and is stated on both man pages: *hue, saturation and lightness
describe the sRGB colour; `brighten` and `darken` work in linear light.*

`hue` wraps rather than clamps (`rotateHue(c, 400.0)` = `rotateHue(c, 40.0)`);
`saturation`/`lightness` clamp. A fully unsaturated colour has no meaningful hue —
`toHsl` reports `0.0` and the round-trip is still exact, which is a test.

## Compatibility / Format Impact

- **Public surface:** `color` gains 17 members and the `Hsl` record. Nothing is
  removed or renamed.
- **canvas internals:** `__CANVAS_SRGB`, `__canvas_srgbTable` and
  `__canvas_linearToSrgb` cease to exist; canvas gains `add_imports(["color"])`.
  These are private helpers, so no program can observe the change other than
  through binary size and codegen.
- **Expected golden drift:** canvas `.ncode`/`.ncodesum` for every canvas fixture
  (the blend bodies change shape and the companion set grows), and `.ir`/`.ast` for
  every canvas importer. **Not expected to drift:** any rendered pixel, any
  `build.log`, any `.run`.
- **Binary size:** every canvas program now also carries `color`'s companion.
  Record the measured delta in Corrections.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work it describes; `- [~]` for partial with one line on what remains;
> `- [x] ~~text~~ — moot: <evidence>` rather than deleting. Fill `Commit:` when the
> phase lands. **An unticked box means NOT DONE.**

### Phase 1 — Move the sRGB table into `color`

The one edit that can change a pixel. Landed alone so a pixel diff has exactly one
possible cause.

- [ ] New `src/codegen/builtins/color/helper_srgb.rs`: `__color_srgbTable` with the
      256 literals copied **verbatim** from `canvas/helper_color.rs:24`, and
      `LET __COLOR_SRGB`.
- [ ] New `func_to_linear.rs` / `func_from_linear.rs` — `fromLinear`'s body is the
      binary search moved verbatim from `__canvas_linearToSrgb`.
- [ ] Move the four Rust unit tests (`srgb_table_covers_every_byte`,
      `srgb_table_endpoints_are_exact`, `srgb_table_is_strictly_increasing`,
      `srgb_table_matches_the_transfer_function`) into `helper_srgb.rs`, repointed
      at the new const. Do not weaken any of them — in particular
      `srgb_table_matches_the_transfer_function` is the only test that catches
      entries that are present but wrong.
- [ ] Rewrite `canvas/helper_color.rs`: delete `SRGB_TABLE` and `LINEAR_TO_SRGB`,
      rewrite `BLEND_CHANNEL` and `BLEND_CHANNEL_MODE` to call
      `color::toLinear`/`color::fromLinear`, keeping every constant
      (`+ 127`, `/ 255`, `+ 32767`, `/ 65535`) and the `Normal`-arm expression
      identity exactly.
- [ ] `canvas/mod.rs:171` — add `"color"` to `add_imports`.
- [ ] Regenerate the canvas `.ncode`/`.ncodesum` goldens and **attribute the
      delta**: it must be confined to canvas fixtures and canvas importers. Use a
      `git archive` attribution binary rather than a sibling worktree.

Acceptance: `tests/rt_canvas_rasteriser.rs` and `tests/rt_canvas_golden.rs` pass
with **pixel-identical** output, and `tests/rt_canvas_metal.rs` still agrees with
the software oracle. A pixel diff is root-caused by dumping the two tables index by
index — it is never grounds to abandon the move.
Commit: —

### Phase 2 — Perceptual operations

- [ ] `func_brighten.rs`, `func_darken.rs`, `func_mix.rs`, `func_grayscale.rs`,
      `func_luminance.rs`, `func_contrast_ratio.rs`, `func_is_dark.rs`,
      `func_is_light.rs` per §5, each with full man prose.
- [ ] Tests: the endpoint-exactness contract from §5 as explicit assertions
      (`brighten(c, 0.0) = c`, `brighten(c, 1.0)` = white with `c`'s alpha,
      `mix(a, b, 0.0) = a`, `mix(a, b, 1.0) = b`), plus the two alpha rules
      (`brighten` leaves alpha untouched; `mix` interpolates it).
- [ ] Tests: pin `contrastRatio(white, black) = 21.0` and
      `contrastRatio(x, x) = 1.0` — the two values the WCAG definition fixes, so a
      coefficient typo cannot pass.

Acceptance: the endpoint and WCAG assertions pass in a `tests/rt-behavior/color/`
fixture; `scripts/man-run-examples.sh color --run` green.
Commit: —

### Phase 3 — Rasteriser cost check

- [ ] Measure the per-pixel cost of the Phase-1 seam against the pre-move binary,
      using the existing benchmark harness and a fill-heavy canvas scene. Record
      both numbers in Corrections.
- [ ] If the regression exceeds 10%, inline `toLinear`'s `getOr` back into the two
      canvas blend bodies **while keeping `color` the single source of the table**
      (canvas would re-index a public `color`-owned list rather than re-declare it).
      Record the decision either way — "measured, no action" is a result.

Acceptance: a recorded before/after number, and either a stated no-action decision
or the mitigation landed with the pixel goldens still identical.
Commit: —

### Phase 4 — HSL

- [ ] `Hsl` record on the package; `func_hsl.rs`, `func_hsla.rs`, `func_to_hsl.rs`,
      `func_saturate.rs`, `func_desaturate.rs`, `func_rotate_hue.rs` per §6.
- [ ] Tests: round-trip `toHsl(hsl(h, s, l))` over the six primary hues and the
      greys; hue wrap (`rotateHue(c, 400.0) = rotateHue(c, 40.0)`); the
      unsaturated-hue rule; saturation/lightness clamping at both ends.
- [ ] Man prose must state the sRGB-vs-linear asymmetry (§6) on both `toHsl` and
      `brighten`, so a reader cannot form the wrong mental model from either page
      alone.

Acceptance: the round-trip and wrap tests pass; `man-census.sh --fill color` still
100%; `--memory-scope color` and `--scope color` still 0.
Commit: —

## Validation Plan

- **Tests:** `tests/rt-behavior/color/**` for every new member; the four moved Rust
  unit tests must keep passing unmodified in their new home.
- **Coverage check:** `scripts/coverage.sh --bin mfb` — confirm the new
  `src/codegen/builtins/color/helper_srgb.rs` and the rewritten
  `canvas/helper_color.rs` are both in the denominator.
- **Runtime proof:** run `examples/emoji` (a real canvas program) before and after
  Phase 1 and compare the rendered frame — the software rasteriser's own output,
  not a test's assertion about it.
- **Doc sync:** `src/docs/spec/app/06_canvas.md` §"Rendering conventions" — the
  blend modes are still defined on linear values, now via `color::toLinear`;
  `src/docs/spec/stdlib/18_color.md` gains the maths section.
- **Acceptance:** `cargo test --no-fail-fast`; `./scripts/test-accept.sh` full run
  (watch `N ran`); `scripts/artifact-gate.sh`; `cargo check --all-targets` at the
  end; `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Whether `toLinear`/`fromLinear` should be public at all.** Recommend yes: a
  package cannot reach another's private helpers (verified, §2), so canvas needs
  them public — and a caller doing its own blending needs the same seam canvas uses.
  The alternative is a canvas-private duplicate, which is the drift this plan
  removes. (§4)
- **`luminance` returning `Float` `0.0..1.0` vs the raw `0..65535` `Integer`.**
  Recommend `Float`, matching the WCAG definition `contrastRatio` consumes. (§5)

## Corrections

_(filled in during execution)_

## Summary

The risk is entirely in Phase 1, and it is a *silent* risk: a wrong table entry
blends toward black rather than failing, which is why the four transfer-function
unit tests move with the table and why the acceptance criterion is rendered pixels
rather than a green suite.

Untouched: every public consumer API. `canvas::Color` is still canvas's type when
this letter lands.
