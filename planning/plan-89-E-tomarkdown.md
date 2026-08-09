# plan-89-E: `astrings::toMarkdown` — the renderer

Last updated: 2026-08-08
Effort: medium (1h–2h)
Depends on: **plan-89-A** (type + text) and **plan-89-B** (attribute overlay + read-time resolution).
Does **not** depend on C or D. If A or B is not complete, E cannot start, full stop.

Adds `astrings::toMarkdown(a AS AttributedString) AS String`, the one real consumer of the attribute
overlay: it flattens overlapping spans into maximal non-overlapping runs and emits a bespoke
markdown-flavored format.

**Single behavioral outcome:** `toMarkdown` renders styled text into the defined marker vocabulary,
with overlapping spans producing correctly-nested per-run flag wrapping and correct forward
font/size state switches, and with delimiter characters in the text/font-names escaped.

References (read first): plan-89-A §2/§3; plan-89-B §3 (higher-start-wins resolution, the model);
`src/builtins/strings.rs:505` `source_file()` (source-companion emit patterns).

## Prerequisites

See plan-89-A. plan-89-A and plan-89-B landed.

## 1. Goal

- `astrings::toMarkdown(a) AS String` emits, for the resolved (higher-start-wins per enum member)
  attribute state across `a`'s scalars:
  - **flags** as per-run wrapping pairs: `**bold**`, `*italic*`, `~~strike~~`, `__underline__`,
    `^^overline^^`;
  - **font/size** as a forward **state switch** `::font;size::` emitted at run boundaries where the
    state changes: a value sets, `-` resets to default, an empty slot leaves unchanged; minimal-delta
    form (`::font::` font-only, `::;size::` size-only, `::-;-::` reset both, etc.);
  - literal delimiter characters escaped.

### Non-goals (explicit constraints)

- **Not CommonMark.** The format is bespoke (`//`? no — italic is `*`; but `__`/`^^`/`::…::` are
  non-CommonMark and `__` collides with CommonMark bold). It is read by the `astrings` toolchain, not
  a standard markdown engine. Do not "fix" it toward CommonMark.
- **No new attributes**; no HTML fallback in v1.
- **`toMarkdown` is read-only** — it never mutates `a`.

## 2. Current State

After B, `AttributedString` carries a span overlay with read-time higher-start-wins resolution
(`getAttributes(a, i)` gives the resolved set at scalar `i`). No renderer exists. Source companions
can hold pure-`.mfb` helpers, but `toMarkdown` needs to walk resolved state scalar-by-scalar — decide
in Phase 1 whether it is expressible in `.mfb` over `getAttributes`/`toString` (preferred, simplest)
or needs a native arm.

### Measured populations

| What | Count | basis |
|---|---|---|
| Flag markers | 5 | `**` `*` `~~` `__` `^^` |
| State-switch slots | 2 | font, size (`::font;size::`) |
| Characters needing escape in text | 6 | `*` `_` `~` `^` `:` (+ `\` the escape itself) |

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| `toMarkdown` can be built over `getAttributes` + `toString` | UNVERIFIED — Phase 1 | if yes, `toMarkdown` is a pure-`.mfb` source-companion function; if the per-scalar walk is too costly, add a native arm |
| Overlap requires per-run re-nesting | CONFIRMED | `Bold[0,10]`+`Italic[5,15]` cannot be written with improperly-nested pairs; each run must independently wrap its active flags |

## 3. Design Overview

**Run flattening.** Walk scalars `0..len`; a **run boundary** occurs wherever the resolved
(flags set, font, size) differs from the previous scalar. This yields maximal runs, each with a stable
resolved style.

**Emit per run:**
- **Font/size** first as a state delta vs the running `(font, size)`: emit a `::…::` marker only for
  the slots that changed (value / `-` reset / omitted-if-unchanged); nothing if neither changed.
- **Flags** as wrapping pairs around the run's visible text, in a **canonical marker order** (fixed,
  e.g. bold, italic, strike, underline, overline) so output is deterministic.
- The run's visible text, with `*_~^:` and `\` escaped.

At end-of-string, no explicit font/size reset is required (the string ends).

**Where correctness risk concentrates:** run-boundary detection (must include font/size changes, not
just flags), canonical ordering (determinism), the minimal-delta font/size logic, and escaping
(especially `:` adjacent to a real `::…::` marker). Each gets a golden.

**Rejected alternative:** *emit spans directly as nested pairs without per-run flattening.* Rejected —
overlaps produce improperly-nested markers; per-run wrapping is the only always-valid form.

## 4. Detailed Design

### 4.1 Resolution + boundaries
Reuse B's higher-start-wins resolution to compute each scalar's `(flags, font, size)`; boundary =
change in that tuple. (If native, share the resolver code with `getAttributes`.)

### 4.2 Emit
Per run: font/size delta marker (minimal form: `::F;S::` with `F`/`S` ∈ {value, `-`, empty};
single-slot `::V::` when only font changes) → open flags in canonical order → escaped text → close
flags in reverse order.

### 4.3 Escaping
Escape `\ * _ ~ ^ :` in visible text and in font names (a font name equal to `-` or containing `;`
or `::` must be escaped so it can't be read as a reset/delimiter).

## Compatibility / Format Impact

- **New:** `astrings::toMarkdown`.
- **Unchanged:** everything else; `toMarkdown` is a pure read.

## Phases

### Phase 1 — implementation-shape decision + flags-only rendering

- [x] Decided **`.mfb` over `getAttributes`/`toString`** (Open Decision 1): `__astrings_toMarkdown` is
      a pure source-companion body — smallest surface, no new codegen, and the run/marker arithmetic is
      far safer in `.mfb`. The per-scalar `getAttributes` walk is acceptable for v1.
- [x] Run flattening (a run boundary is a change in the resolved `MdState` record, compared with `=`) +
      per-run flag wrapping in canonical enum order (open bold/italic/underline/strike/overline, close
      in reverse) + delimiter escaping (`\ * _ ~ ^ :`). Font/size handled in the same commit (Phase 2).
- [x] Tests: `tests/rt-behavior/astrings/tomarkdown-flags-rt/` — single flag (`**hello**`), overlapping
      flags with per-run re-nesting (`**hell*****o wo****rld*`), all five flags canonical+reverse
      (`***__~~^^x^^~~__***`), escaping (`a\*b\_c\~d\^e\:f\\g`), plain text unchanged.

Acceptance: MET. Overlapping flag spans render as correctly-nested per-run pairs; delimiters escaped;
deterministic canonical order.
Commit: 22ba58bfd

### Phase 2 — font/size state switches

- [x] Minimal-delta `::font;size::` emission at run boundaries vs the running state: `::font::`
      font-only, `::;size::` size-only, `::font;size::` both, `-` for a reset slot; nothing when
      neither changed. Font-name escaping (the text set plus `;` and a literal `-`).
- [x] Tests: `tests/rt-behavior/astrings/tomarkdown-fontsize-rt/` — font-only + size-only + reset in
      one string (`::Serif::ab::;12::cde::-::fg`), both-at-once (`::Mono;10::hi`), unchanged-across-run
      (single `::;9::abc`), size reset (`::;8::ab::;-::cd`), font name needing escape (`::a\:b\;c::z`).

Acceptance: MET. Font/size switches match the set/reset/unchanged rule and the running-state
minimal-delta form; no spurious markers.
Commit: 22ba58bfd

## Validation Plan

- Tests: rt-behavior goldens for flags (overlap, escape) and font/size (all slot combinations, reset,
  unchanged); a combined styled sample.
- Coverage check: fixtures exercise the emit path.
- Runtime proof: a combined fixture printing `toMarkdown` of a multiply-styled string.
- Doc sync: the `astrings` spec section documenting the full marker vocabulary (this is the feature's
  spec home) + the `toMarkdown` man page; note explicitly "not CommonMark".
- Acceptance: `cargo test --bin mfb`; `artifact-gate.sh <exe> all`.

## Open Decisions

1. **`.mfb` vs native.** Recommended **`.mfb` over `getAttributes`/`toString`** if performance is
   acceptable — smallest surface, no new codegen; fall back to native only if the per-scalar walk is
   too costly.
   Decision: mfb
2. **Canonical flag order.** Recommended enum-declaration order (bold, italic, underline, strike,
   overline) — deterministic and self-documenting.
   Decision: follow recommended

## Corrections

- **Run-boundary detection uses a comparable `MdState` record, not a hand-built key.** The resolved
  per-scalar state (5 flags + font + size) is a companion-internal `MdState` record; a run boundary is
  simply `stateAt(j) <> stateAt(i)` (records with all-comparable fields are comparable). This is
  O(n²) in `getAttributes` calls but adequate for v1 (Open Decision 1 accepts the per-scalar walk).
- **`toMarkdown` is a `.mfb` `Implementation::Rewrite` member** (`astrings.toMarkdown` →
  `__astrings_toMarkdown`), consistent with the rest of the companion surface; it reuses the internal
  `astrings::scalarLen` for the scalar count.
- **Spec home:** the marker vocabulary + the whole `astrings` semantic model live in a new stdlib spec
  section, `./mfb spec stdlib astrings` (`src/docs/spec/stdlib/15_astrings.md`, auto-discovered), plus
  the `toMarkdown` man page; both state explicitly that the format is NOT CommonMark.

## Summary

E is contained: a run-flattener plus a marker emitter, both fenced by goldens on overlap, escaping,
and the font/size delta rule. The bespoke format is intentional and documented as non-CommonMark.
This is the letter that makes the whole feature observably useful. Untouched: `a` itself (pure read)
and everything in A–D.
