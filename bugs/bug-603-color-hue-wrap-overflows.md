# bug-603: `color::hsl` / `hsla` / `rotateHue` raise `ErrOverflow` for a large finite hue

Last updated: 2026-09-12
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: `tests/rt-behavior/color/color_hsl_rt` (extend — see Phase 1)

`color::hsl`, `color::hsla` and `color::rotateHue` fail at run time with
`ErrOverflow` (`7-705-0010`) when the hue — or, for `rotateHue`, the colour's hue
plus `degrees` — is a finite `Float` whose number of whole turns is past the
`Integer` range, roughly `|hue| > 3.3e21` degrees. None of the three declares any
error: their Errors tables are empty (`mfb man color hsla`), and until plan-125
their pages said "Wraps, so any value is valid."

**The single correct behavior a fix produces:** every finite hue wraps into
`[0.0, 360.0)` and the call returns a colour, whatever its magnitude —
`hsla(1e36, 1.0, 0.5, 255)` equals `hsla(304.0, 1.0, 0.5, 255)`, because
`1e36 MOD 360.0` is `304.0`. No hue value that the language lets a program hold
raises.

Found by the plan-125-A pilot's cross-model review of `mfb man color hsla`
(`planning/plan-125-findings/A-iter2/man-page-color-hsla.md`, finding 1) and
confirmed on the main thread. **Filed, not fixed**, by user instruction during a
documentation-only plan ("file all bugs, make no fixes"). Until this lands, the
three pages document the overflow as current behavior (`mfb man color hsla`,
`hue` parameter) — remove that sentence when the fix lands.

References:

- `mfb man color hsl`, `mfb man color hsla`, `mfb man color rotateHue` — the hue
  parameter contract.
- `mfb man math floor` — "returns Integer … A magnitude too large for Integer
  raises ErrOverflow."
- `mfb spec language` numeric edge cases — Float `MOD`: "the remainder has the
  same sign as a"; a non-finite Float is caught at the observation boundary.
- Memory `math-floor-returns-an-integer` — the same trap, recorded earlier.
- `planning/plan-125-A-standards-tooling-pilot.md` §5.10 / §5.11.

## Failing Reproduction

```
mfb init /tmp/hue-overflow
cat > /tmp/hue-overflow/src/main.mfb <<'EOF'
IMPORT io
IMPORT color

SUB main()
  LET big AS Float = 1000000000000000000.0 * 1000000000000000000.0
  io::print("hsla big hue: " & color::toHexAlpha(color::hsla(big, 1.0, 0.5, 255)))
END SUB
EOF
cd /tmp/hue-overflow && mfb build . && ./build/hue_overflow.out; echo "exit=$?"
```

- Observed (release `mfb`, macos-aarch64, 2026-09-12):
  ```
  Error: 7-705-0010
  Arithmetic overflow or numeric conversion outside the destination range.
  exit=255
  ```
- Expected: `hsla big hue: #ff00d4ff` — the colour of hue `304.0`, with exit 0.

The same program with `color::rotateHue(color::rgb(255, 0, 0), big)` fails
identically (observed). `color::hsl(0.0 - big, 1.0, 0.5)` goes through the same
line.

Contrast cases that work today, and must keep working:

| Input | Result |
|---|---|
| `rotateHue(#3366cc, 400.0)` = `rotateHue(#3366cc, 40.0)` | ✓ (`color_hsl_rt` golden `wrap400`/`wrap40`) |
| `rotateHue(#3366cc, -320.0)` | ✓ (`wrapNeg320`) |
| `rotateHue(#3366cc, 360.0)` is the identity | ✓ (`wrap360`) |
| `1e36 MOD 360.0` in user code | ✓ prints `304.00` |
| a non-finite hue (`big^9`) | fails **before** reaching `color`, with `Error: 7-705-0015` — correct, and not this bug |

## Root Cause

`src/codegen/builtins/color/helper_hsl.rs:__color_wrapHue`:

```
LET turns AS Float = hue / 360.0
LET wrapped AS Float = hue - toFloat(math::floor(turns)) * 360.0
```

`math::floor(Float)` returns an **`Integer`** and raises `ErrOverflow` when the
floored value does not fit in 64 bits. The helper only wants the fractional
turn, but it routes that through an `Integer` it immediately converts back to
`Float`. So a hue is valid only while `|hue / 360|` fits in an `Integer`
(about 9.2e18 turns, 3.3e21 degrees).

Every hue-building path calls it:
`__color_hslToColor` (used by `color::hsl`, `color::hsla`, `color::rotateHue`,
`color::saturate`, `color::desaturate`) and `__color_colorToHsl` (used by
`color::toHsl`).

The contrast cases are immune because their turn count is small; a non-finite
hue is immune because the language rejects it where it is produced.

## Goal

- `__color_wrapHue` returns a value in `[0.0, 360.0)` for every finite `Float`
  and never raises.
- `hsla(1e36, 1.0, 0.5, 255)`, `hsl(-1e36, 1.0, 0.5)` and
  `rotateHue(rgb(255, 0, 0), 1e36)` each return the colour of the wrapped hue.
- `toHsl(hsl(-360.0, 1.0, 0.5)).hue` is `+0.0` (no `-0.0` leaks out of the wrap).

### Non-goals (must NOT change)

- Every currently-passing output of `tests/rt-behavior/color/color_hsl_rt` —
  primaries, greys, clamps, `wrap*`, alpha lines.
- The sRGB (not linear-light) HSL model, and saturation/lightness clamping.
- The public signatures and the (empty) Errors tables of the three functions.
- **Tempting wrong fix:** declaring `ErrOverflow` on the three descriptors and
  calling the docs sentence the contract. That ratifies the bug; the pages have
  always promised wrapping.
- **Also wrong:** clamping `hue` to some "sane" range before wrapping — that
  changes the colour for large hues instead of wrapping them.

## Blast Radius

Found by `grep -rn 'math::floor\|math::ceil\|math::round' src/codegen/builtins --include='*.rs'`
(2026-09-12), non-comment hits only:

- `color/helper_hsl.rs:__color_wrapHue` (floor of `hue / 360.0`) — **fixed by this bug**.
- `color/helper_hsl.rs:__color_hslToColor` — `math::floor(sixth)` and
  `math::floor(sixth / 2.0)` — **unaffected**: `sixth = wrapped / 60.0` is in
  `[0, 6)` after the wrap.
- `audio/helper_mml_synth.rs:40` — `phase - toFloat(math::floor(phase))`, the
  same "fractional part through an Integer" shape — **latent, NOT observed to
  fail, out of scope**: `phase` would have to pass 9.2e18 cycles, which no
  finite synthesis reaches. Worth converting to `MOD 1.0` if that file is
  touched, and noted here so the pattern is not re-derived.
- `canvas/helper_items.rs:294-295` — `math::floor` of an inverse-transformed
  coordinate, **unaffected**: the result is genuinely wanted as an `Integer`
  pixel coordinate, so an out-of-range value is a real out-of-range result, not
  a lost fractional part.
- `vector/func_*.rs` (`rotate_2d`, `normalize`, `clamp_length`, `lerp`,
  `lerp_unclamped`, `slerp`, `project`, `angle`) — `math::round` producing
  `Integer` vector components — **unaffected**: an `Integer` result is the
  declared return type; overflow there is a true overflow.

## Fix Design

Replace the floor with Float `MOD`, which is exact at every finite magnitude
(measured: `1e36 MOD 360.0` → `304.00`) and keeps the sign of the dividend:

```
FUNC __color_wrapHue(hue AS Float) AS Float
  LET wrapped AS Float = hue MOD 360.0
  IF wrapped < 0.0 THEN
    LET up AS Float = wrapped + 360.0
    IF up >= 360.0 THEN
      RETURN 0.0
    END IF
    RETURN up
  END IF
  IF wrapped = 0.0 THEN
    RETURN 0.0
  END IF
  RETURN wrapped
END FUNC
```

The `up >= 360.0` arm covers a tiny negative remainder that rounds to exactly
`360.0` when lifted; the `wrapped = 0.0` arm normalises `-0.0` (from a negative
whole number of turns) to `+0.0`, which the old form produced and `toHsl` prints.

Rejected: keeping `math::floor` behind an `IF math::abs(turns) < 9.0e18` guard —
it needs a second, wrong answer for the large branch.

**Expected output shift:** `helper_hsl.rs` BODY is byte-significant and is
injected into every program that imports `color` (`RegistryHelper::always`), so
every `.ir` golden that embeds the colour helpers changes by exactly this
function's lines, and the `.ncodesum` of any fixture importing `color` flips.
`grep -rl '__color_wrapHue' tests/` lists them. No runtime output of an existing
fixture should change.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Extend `tests/rt-behavior/color/color_hsl_rt/src/main.mfb` with a hue of
      `1e36`, a hue of `-1e36`, a `rotateHue` by `1e36` (each printed beside the
      same colour built from the wrapped value, `304.0` / `56.0`), and
      `toHsl(hsl(-360.0, 1.0, 0.5)).hue`. Confirm the fixture stops at the first
      of them with `Error: 7-705-0010` on the unfixed binary (observed
      2026-09-12 against a `/tmp` copy).
- [x] Complete the blast-radius audit above (done at filing).

Acceptance: the extended fixture fails for the documented reason; the audit
list is complete with a verdict per site.
Commit: —

### Phase 2 — the fix

- [ ] Apply the Fix Design to `helper_hsl.rs:__color_wrapHue`.
- [ ] Remove the "raises `ErrOverflow`" sentence from the `hue` parameter of
      `func_hsl.rs` and `func_hsla.rs` and from `degrees` in `func_rotate_hue.rs`
      (added by plan-125 to document this bug as current behavior).

Acceptance: the Phase 1 lines print the wrapped colours and `0.00`; every
pre-existing `color_hsl_rt` output line is unchanged.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/sync-goldens.sh target/release/mfb color_hsl_rt`, then diff:
      the only runtime-output delta is the new lines.
- [ ] Regenerate every other golden embedding the helper
      (`grep -rl '__color_wrapHue' tests/`) and confirm each `.ir` diff is only
      `__color_wrapHue`'s lines; regenerate flipped `.ncodesum`s per
      `.ai/testing-gates.md`.
- [ ] `scripts/artifact-gate.sh target/release/mfb all` → `diffs=0`; the
      project's full suite.

Acceptance: full suite green; golden deltas are exactly the helper's lines plus
the new fixture output.
Commit: —

## Validation Plan

- Regression test: `tests/rt-behavior/color/color_hsl_rt` (extended).
- Runtime proof: the Failing Reproduction above prints `#ff00d4ff`, exit 0.
- Doc sync: remove plan-125's `ErrOverflow` sentence from the three parameter
  descriptions (Phase 2). No spec change: `stdlib/18_color.md` already describes
  hue as wrapping.
- Full suite: `scripts/artifact-gate.sh target/release/mfb all`,
  `scripts/test-accept.sh`, `cargo test --release --no-fail-fast`.

## Open Decisions

- None.

## Summary

The fix is one function. The engineering cost is the golden regeneration: the
helper is injected into every program importing `color`, so the `.ir` and
`.ncodesum` churn is wide but mechanical, and each diff must be confirmed to be
only `__color_wrapHue`. The `audio` MML synth carries the same pattern latently
and is left alone.
