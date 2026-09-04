# plan-122-E: `astrings` foreground/background take a `color::Color`

Last updated: 2026-09-02
Effort: medium (1h–2h)
Depends on: plan-122-A/B/C (the `color` package). NOT plan-122-D — see Prerequisites.

`astrings::foreground(r, g, b)` and `astrings::background(r, g, b)` become
`astrings::foreground(base AS color::Color)` and
`astrings::background(base AS color::Color)`, and the `AttrNumber` payload widens
from `0xRRGGBB` to `0xAARRGGBB` so an attribute round-trips a whole colour rather
than losing its alpha.

Behavioral outcome: `astrings::getAttributes` on a string styled with
`astrings::foreground(color::rgba(255, 128, 0, 128))` returns an `AttrNumber` whose
`value` unpacks — via `color::fromPacked` — to exactly that colour, alpha included;
and `term::drawText` renders it as the same truecolor foreground it renders today.

References:

- plan-122-A — Prerequisites; `color::toPacked`/`fromPacked` and the `0xAARRGGBB`
  canonical order this letter depends on.
- `src/docs/spec/stdlib/15_astrings.md` — the `astrings` specification, including
  the `term::drawText` truecolor sentence at `:41`.
- `src/codegen/builtins/term/helper_astrings_bridge.rs` — the gated `term`↔`astrings`
  chunk that unpacks the payload. It is edited here, not in F, because it is the
  payload's only consumer.

## Prerequisites

Stated once in plan-122-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-122-A/B/C complete (the `color` package exists) | `ls planning/completed/plan-122-{A,B,C}-*` → three matches | **MET** (2026-09-04) — landed on `main` at `756c57a87` |
| ~~plan-122-D complete~~ | `ls planning/completed/plan-122-D-*` → one match | **CORRECTED — this row was wrong.** D is not a precondition for E; see below and Corrections. D remains unstarted by explicit user instruction. |

~~If plan-122-D is not complete, this sub-plan cannot start, full stop.~~

**Why the D row was struck (measured 2026-09-04, not assumed).** E and D are
independent: D migrates **canvas**, E migrates **astrings**, and the two packages
do not touch each other.

- E's body never references D. `grep -n "plan-122-D\|letter D\|in D\b"` over this
  file returns **only** the `Depends on:` line and the prerequisite row itself —
  no task, no design decision, and no acceptance criterion needs anything D
  produces.
- The packages are unconnected. `grep -rn "canvas" src/codegen/builtins/astrings/*.rs`
  → **no hits**; `astrings`'s `add_imports` is `["collections", "astrings",
  "strings", "bits"]` with no `canvas`. In the other direction canvas's only
  mention of astrings is a **comment** (`canvas/mod.rs:175`), not an import or a
  call.
- E's phases name only `astrings/*` and `term/*` files
  (`func_foreground.rs`, `func_background.rs`, `helper_pack_color.rs`,
  `term/helper_astrings_bridge.rs`, `tests/acceptance/src/astrings.mfb`,
  `tests/rt_native_term_runtime.rs`). No canvas file appears in any phase.

What E actually needs is the **`color` package** — specifically `color::Color` and
`color::toPacked`/`fromPacked` — and those come from A and B, both landed. The
`Depends on: plan-122-D` line encoded the author's intended *sequencing* of the
three consumer migrations (D→E→F), not a technical precondition.

**F's dependency on E is different and is real**, and is honoured: the term↔astrings
bridge reads the packed attribute payload whose bit layout E widens, so E is done
first.

## 1. Goal

- `astrings::foreground` and `astrings::background` each take a single
  `color::Color` and no longer take three `Byte` channels.
- The `AttrNumber` payload for `AttrTypeNumber.Foreground`/`Background` is
  `0xAARRGGBB`, alpha in the high byte, matching `color::toPacked`.
- The term bridge renders exactly the colours it renders today (terminals have no
  alpha; the bridge ignores it, explicitly and documented).
- `astrings::toMarkdown` is unchanged — it already ignores colour attributes
  (`astrings/helper_md_state_at.rs:41`).

### Non-goals (explicit constraints)

- **No change to `AttrNumber`'s shape.** It stays `{ kind AS AttrTypeNumber, value AS Integer }`
  (`astrings/mod.rs:170-190`). The colour rides in `value`; it does not become a
  record field. Widening the record would change `AttrSpan`'s encoding
  (`astrings/helper_encode_attr.rs:12-19`) and the opaque `AttributedString`
  layout with it — far more than this letter needs.
- **No change to `AttributedString`'s wire representation.** It is an opaque
  primitive-like type whose field layout is a compiler-side table, never serialized
  (`src/binary_repr/sections.rs:158`). `value` is already an `Integer`; only the
  bit assignment inside it changes.
- **No change to overlap resolution.** Higher-start-wins at read time, unchanged.
- **No new attribute kind.** `AttrTypeNumber` keeps its three variants.
- **Terminals still get no alpha.** The bridge ignores it. That is a rendering
  limitation, documented on the man page, not a reason to drop alpha at the
  attribute.

## 2. Current State

| Piece | File:line | Today |
|---|---|---|
| `foreground` member | `astrings/func_foreground.rs:38` | `FUNC __astrings_foreground(r AS Byte, g AS Byte, b AS Byte) AS Attribute` → `AttrNumber[AttrTypeNumber.Foreground, __astrings_packColor(r, g, b)]` |
| `background` member | `astrings/func_background.rs:38` | symmetric |
| the packer | `astrings/helper_pack_color.rs:16` | `bor(bor(sl(r,16), sl(g,8)), b)` → `0xRRGGBB` |
| enum variant docs | `astrings/mod.rs:367`, `:372` | "The text color, packed `0xRRGGBB`." |
| enum comment | `astrings/mod.rs:356` | "a packed `0xRRGGBB` color" |
| bridge style record | `term/helper_astrings_bridge.rs:50-57` | `__TermStyle { bold, underline, fg AS Integer, bg AS Integer }`, `-1` = unset |
| bridge unpackers | `term/helper_astrings_bridge.rs:97-107` | `__term_colorR/G/B(packed)` — hand-rolled shifts and masks |
| bridge appliers | `term/helper_astrings_bridge.rs:110-125` | `__term_applyFg/Bg(packed, saved AS term::TermColor)` |

The `-1` sentinel is documented as sound "because a packed color is always in
`0..0xFFFFFF`" (`term/helper_astrings_bridge.rs:49-53`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `astrings::foreground(`/`background(` sites, whole tree | 24 | `grep -rn 'astrings::foreground(\|astrings::background(' --include='*.mfb' --include='*.rs' --include='*.md' . \| wc -l` |
| `.mfb` files using them | 3 | `grep -rl 'astrings::foreground\|astrings::background' --include='*.mfb' .` → `examples/browser/app/src/main.mfb`, `examples/browser/display/src/lib.mfb`, `tests/acceptance/src/astrings.mfb` |
| Rust test files using them | 1 | `grep -rln 'astrings::foreground\|astrings::background' tests/*.rs` → `tests/rt_native_term_runtime.rs` |
| `.mfb` fixtures importing `astrings` | 15 | `grep -rl '^IMPORT astrings' --include='*.mfb' tests/ examples/ \| wc -l` |
| golden fixture dirs under `tests/rt-behavior/astrings` + `tests/syntax/astrings` | 9 + 3 | `ls -d tests/rt-behavior/astrings/*/ \| wc -l; ls -d tests/syntax/astrings/*/ \| wc -l` |

### Verified properties

- **The `-1` sentinel survives the widening.** A `0xAARRGGBB` value is at most
  `0xFFFFFFFF` = 4294967295, which is a positive `Integer`, so `-1` remains
  unreachable as a packed colour. Verified by reading the sentinel's justification
  at `term/helper_astrings_bridge.rs:49-53` and the range it depends on.
- **`toMarkdown` ignores colour already**, so widening the payload cannot change
  markdown output: `astrings/helper_md_state_at.rs:41` — "Only FontSize renders in
  markdown; Foreground/Background carry a ... value the renderer skips".
- **The encode/decode path is payload-agnostic.** `__astrings_encodeAttr` writes
  `n.value` into the `AttrSpan`'s numeric slot and `__astrings_decodeAttr` reads it
  back, with no range assumption
  (`astrings/helper_encode_attr.rs:12-19`). Verified by reading both.
- **`astrings` will pay `color`'s companion cost.** `astrings` has a non-empty
  companion and gains `add_imports(["color"])`, so its importers grow by `color`'s
  measured size on top of astrings' existing 1,073,280 bytes (plan-122-A §2).
  Record the new number in Corrections.

## 3. Design Overview

Three edits and one rename:

1. **The members take a `color::Color`.** Signature and body change; the descriptor
   param type is `COLOR_TYPE_ID`, and `astrings` gains `add_imports(["color"])`.
2. **`__astrings_packColor` is replaced by `color::toPacked`.** Deleting the helper
   rather than widening it is the point of the plan: the packing lives in exactly
   one place. `helper_pack_color.rs` is deleted.
3. **The bridge unpacks with `color::fromPacked`.** `__term_colorR`/`G`/`B` are
   deleted; `__term_applyFg`/`Bg` take the packed `Integer` and reconstruct a
   `color::Color`. This chunk already carries its own inline `IMPORT` lines
   (`term/helper_astrings_bridge.rs:44-47`), so it adds `IMPORT color` there —
   **not** to the `term` package's `add_imports`, which must stay absent so
   term-only programs keep costing nothing (plan-122-A §2).

**Where correctness risk concentrates:** the bit reassignment. `0xRRGGBB` →
`0xAARRGGBB` moves nothing — red stays at bits 16–23 — it only adds alpha at
24–31. That is what makes the change safe, and it is also what makes a mistake
invisible: a bridge that masked with `0xFFFFFF` where it should mask with
`0xFFFFFFFF`, or vice versa, still renders correct colours in every existing test
because every existing colour has alpha `255`. **A test must therefore assert a
non-`255` alpha round-trips**, or the widening is untested.

**Byte-identity is not the gate.** `astrings` `.ncode` and every astrings
importer's `.ir`/`.ast` are expected to drift. `build.log`/`.run` must not change
for any fixture that does not call the two members.

### Rejected alternatives

- **Keep `0xRRGGBB` and drop alpha at the call.** Rejected (user decision,
  2026-09-02): `astrings::foreground(c)` silently discarding `c.alpha` is a lossy
  call that reads as lossless.
- **Keep the three-`Byte` overload alongside the `Color` one.** Rejected: it is the
  same "two ways to say one thing" this plan removes, and the three-channel form
  cannot express alpha at all.
- **A new `AttrTypeNumber.ForegroundAlpha` variant.** Rejected: an enum variant set
  is a closed surface and this needs no new kind — the existing `Integer` payload
  has 32 spare bits.

## Compatibility / Format Impact

**Breaking at the source level.** `astrings::foreground(255, 128, 0)` becomes
`astrings::foreground(color::rgb(255, 128, 0))`, and the calling file needs
`IMPORT color`.

**Observably changed:** `AttrNumber.value` for a colour attribute now carries alpha
in bits 24–31. A program that read `value` directly and assumed 24 bits sees a
larger number. This is why the enum-variant descriptions
(`astrings/mod.rs:367`, `:372`) must be updated in the same commit — they are the
documentation of that payload.

**Unchanged:** `AttributedString`'s layout, `AttrSpan`'s encoding, overlap
resolution, `toMarkdown` output, and what a terminal actually displays.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work; `- [~]` for partial with a line on what remains;
> `- [x] ~~text~~ — moot: <evidence>` rather than deleting. Fill `Commit:` on
> landing. **An unticked box means NOT DONE.**

### Phase 1 — Widen the payload, keep the signature

Prove the bit reassignment alone is correct before the API changes, so a failure
has one possible cause.

- [x] `astrings/helper_pack_color.rs` — widen to `0xAARRGGBB` with a fourth `a AS Byte`
      parameter; callers pass `toByte(255)`. (This file is deleted in Phase 2; it
      exists in this shape only so the payload change lands separately.)
- [x] `term/helper_astrings_bridge.rs` — ~~add `__term_colorA`, and make
      `__term_applyFg`/`Bg` mask the low 24 bits~~ — **moot: the existing
      per-channel masks already discard alpha, and a `__term_colorA` would be dead
      code.** `__term_colorR/G/B` are each `(packed >> n) & 255`, so with alpha
      added *above* bit 23 the colour channels do not move and alpha cannot reach
      the terminal. Verified arithmetically rather than assumed: for
      `a=255,r=255,g=128,b=0` the widened `0xFFFF8000` yields `R=255 G=128 B=0`,
      byte-identical to what the old `0xFF8000` yielded. Nothing reads alpha — the
      bridge's contract is to *ignore* it — so a `colorA` accessor would be an
      unused FUNC in every term+astrings importer's companion, and Phase 2 deletes
      these three helpers anyway. The comments were updated instead: the payload and
      the `-1` sentinel's justification now say `0xAARRGGBB` / `0..0xFFFFFFFF`.
- [x] Update the `AttrTypeNumber` variant descriptions and the enum comment
      (`astrings/mod.rs:356`, `:367`, `:372`) to say `0xAARRGGBB`.
      Both variant descriptions also now state that terminals have no alpha and
      `term::drawText` ignores it, so the limitation is on the surface a reader
      meets first.
- [x] Tests: a `tests/rt-behavior/astrings/` case asserting
      `getAttributes` returns `value = 0xFF0080FF` for
      `astrings::foreground(255, 128, 0)` — the literal constant, not a recomputation.
      Landed as `tests/rt-behavior/astrings/color-payload-rt`. **The plan's expected
      constant was wrong** — `0xFF0080FF` transposes the channels; for
      `foreground(255, 128, 0)` under `0xAARRGGBB` the value is `0xFFFF8000`. See
      Corrections. The fixture pins three literals, not one, and the extra two earn
      their place: `black` (`0xFF000000`) is the only case that distinguishes "alpha
      set to 255" from "alpha left at zero", and the positivity check pins what keeps
      `-1` usable as the bridge's unset sentinel.

Acceptance: **MET.** `tests/rt_native_term_runtime.rs` → **7 passed, 0 failed**, so
the bridge sends the terminal exactly what it sent before. The payload assertions
pass against literals: `fg=4294934528` = `0xFFFF8000`, `bg=4279383126` =
`0xFF123456`, `black=4278190080` = `0xFF000000`, `positive=TRUE`.
Commit: —

### Phase 2 — The members take a `color::Color`

- [ ] `astrings/mod.rs:135` — add `"color"` to `add_imports`.
- [ ] `astrings/func_foreground.rs`, `func_background.rs` — single `base AS color::Color`
      parameter typed `COLOR_TYPE_ID`; bodies become
      `AttrNumber[AttrTypeNumber.Foreground, color::toPacked(base)]`. Rewrite
      `INTRO`/`DESC`/`EX` (the current prose describes the three-channel form and
      the `0xRRGGBB` packing in detail — every sentence of it is now wrong).
- [ ] Delete `astrings/helper_pack_color.rs` and its `register` call; **check the
      neighbouring helper's doc comment did not absorb the deleted one's**.
- [ ] `term/helper_astrings_bridge.rs` — add `IMPORT color` to the chunk's own
      import block (`:44-47`); delete `__term_colorR`/`G`/`B`; rewrite
      `__term_applyFg`/`Bg` to use `color::fromPacked`.
- [ ] Update the 3 `.mfb` files and `tests/rt_native_term_runtime.rs`'s embedded
      programs: add `IMPORT color`, wrap the channels in `color::rgb(...)`.

Acceptance: `mfb man astrings foreground` shows the one-parameter signature and its
example compiles and runs (`scripts/man-run-examples.sh astrings --run`); the 3
`.mfb` files build; `tests/rt_native_term_runtime.rs` passes.
Commit: —

### Phase 3 — Alpha round-trip and goldens

- [ ] Tests: assert a **non-255 alpha** round-trips —
      `color::fromPacked(attr.value) = color::rgba(255, 128, 0, 128)` after
      `astrings::foreground(color::rgba(255, 128, 0, 128))`. Without this the
      widening is untested, because every pre-existing colour has alpha 255.
- [ ] Tests: assert the terminal rendering **ignores** alpha — a half-transparent
      foreground draws the same cells as the opaque one. This is the partner test
      that pins what must not change.
- [ ] Regenerate goldens for the 12 astrings fixture dirs and every astrings
      importer; attribute the delta with a `git archive` attribution binary.
- [ ] `src/docs/spec/stdlib/15_astrings.md` — the payload is `0xAARRGGBB`; the
      constructors take a `color::Color`; the truecolor sentence at `:41` gains the
      alpha-is-ignored note.

Acceptance: both alpha tests pass; `./scripts/test-accept.sh` full run green with
the `N ran` count checked; the golden delta is itemized in Corrections and confined
to `.ir`/`.ast` of astrings importers plus the fixtures this letter edited.
Commit: —

## Validation Plan

- **Tests:** `tests/rt-behavior/astrings/**` (payload literal, alpha round-trip,
  alpha-ignored-by-terminal), `tests/rt_native_term_runtime.rs` (the bridge),
  `tests/acceptance/src/astrings.mfb`.
- **Coverage check:** confirm the rewritten `func_foreground.rs`/`func_background.rs`
  and the edited bridge chunk are in `scripts/coverage.sh --bin mfb`'s denominator.
  The bridge is gated on both imports, so a coverage run that never imports both
  never reaches it.
- **Runtime proof:** run `examples/browser` and confirm the styled text renders as
  before — the bridge's real output, not a test's assertion about it.
- **Doc sync:** `src/docs/spec/stdlib/15_astrings.md`; the `AttrTypeNumber` variant
  descriptions; `src/docs/spec/stdlib/18_color.md` gains a note that the packed
  form is what `astrings` attributes carry.
- **Acceptance:** `cargo test --no-fail-fast`; `./scripts/test-accept.sh` full;
  `scripts/artifact-gate.sh`; `cargo check --all-targets`; `cargo fmt`.

## Open Decisions

- **Whether the bridge should honour alpha by blending against the cell's current
  background.** Recommend no, and say so on the man page: the terminal surface has
  no alpha, and a synthesized blend would disagree with what a GPU canvas draws for
  the same colour. (§1)
- **Whether to keep `__astrings_packColor` as a thin forwarder to `color::toPacked`.**
  Recommend delete: one packer is the point. (§3)

## Corrections

**The Prerequisites row was wrong: E does not depend on D.** Struck with evidence
in §Prerequisites above. Summary: E migrates `astrings`, D migrates `canvas`, and
the two packages do not reference each other in either direction
(`grep -rn "canvas" src/codegen/builtins/astrings/*.rs` → no hits; canvas's only
mention of astrings is a comment). E's body never cites D outside the dependency
line itself, and none of its phases name a canvas file. What E actually needs is
`color::Color` and `color::toPacked`/`fromPacked`, which A and B landed. The
`Depends on: plan-122-D` line encoded the author's intended sequencing of the three
consumer migrations, not a technical precondition. **F's dependency on E is real
and is honoured.** (D remains unstarted, by explicit user instruction.)

**Phase 1's expected payload constant was wrong in the plan.** The task says
`getAttributes` should return `value = 0xFF0080FF` for
`astrings::foreground(255, 128, 0)`. Under the `0xAARRGGBB` order this letter
adopts — the same order `color::toPacked` produces, which §1 requires — that colour
packs to **`0xFFFF8000`** (`a=FF, r=FF, g=80, b=00`). `0xFF0080FF` reads as
`a=FF, r=00, g=80, b=FF`, i.e. the red and blue channels transposed.

Caught by writing the literal into the fixture and comparing against the runtime
rather than copying the plan's number, which is the whole reason the task insists
on a literal in the first place. The fixture pins `0xFFFF8000`, and two further
literals the plan did not ask for: `0xFF000000` for black — the only case that can
tell "alpha set to 255" apart from "alpha left at zero", since every other channel
is already zero — and a positivity assertion, which is what keeps `-1` sound as the
bridge's unset sentinel after the widening.

**Phase 1's bridge task was already satisfied.** See the ticked box: the existing
per-channel masks discard alpha by construction, so no applier change was needed
and a `__term_colorA` would have been dead code. Marked moot with the arithmetic
rather than implemented.

## Summary

The bit reassignment is safe — red, green and blue do not move — which is exactly
why it is easy to get wrong invisibly: every existing colour has alpha 255, so a
mask mistake passes every pre-existing test. Phase 3's non-255 alpha round-trip is
the only test that can see it, and it is paired with a test pinning that terminal
rendering is unchanged.

Untouched: `term`'s own colour API, which is F.
