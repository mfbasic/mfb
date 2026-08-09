# plan-89-B: the `Attribute` model, storage, and mutation (Tier C)

Last updated: 2026-08-08
Effort: large (3h–1d)
Depends on: **plan-89-A** (the opaque `AttributedString` type must exist and round-trip). If A is not
complete, B cannot start, full stop.

Adds the open, user-constructible `Attribute` model, the attribute-overlay storage inside
`AttributedString`, and the Tier-C mutation/read functions: `addAttribute`, `removeAttribute`,
`clearAttributes` (ranged and whole), and `getAttributes`. After B, styled text can be built and
inspected headlessly; rendering (`toMarkdown`) is still E.

**Single behavioral outcome:** given `a = astrings::fromString("hello world")`,
`addAttribute(a, 0, 4, astrings::bold())` marks scalars 0–4 inclusive bold; a subsequent
`removeAttribute` over a sub-range **splits** the span (leaving the flanks); `getAttributes(a, i)`
returns the attributes active at scalar `i` with per-enum-member **higher-start-wins** resolution.

References (read first):

- **plan-89-A §2/§3** — the machinery map and the opaque-primitive design this extends.
- `src/builtins/strings_package.mfb` + `src/builtins/strings.rs:505` `source_file()` — the embedded
  source-companion precedent the `Attribute` model declarations follow.
- `src/target/shared/code/builder_search.rs:653` `lower_mid` — the scalar-index→byte-offset walk
  pattern (`emit_ascii_scalar_fastforward` + `emit_scalar_skip_continuations`) reused for range args.
- `src/ast/manifest.rs:42` `template(...)` — how concrete always-in-scope types are injected (for
  `Attribute`/wrappers/enums if not placed in the source companion).

## Prerequisites

See plan-89-A. Additionally: plan-89-A landed (`ls planning/completed/plan-89-A-* → present`, or its
phases all show `Commit:` hashes).

## 1. Goal

- The open model exists and is user-constructible: three enums (`AttrTypeFlag`/`AttrTypeText`/
  `AttrTypeNumber`), three wrapper records (`AttrFlag`/`AttrText`/`AttrNumber`), and `UNION Attribute`.
- Convenience constructors: `astrings::bold()`, `italic()`, `underline()`, `strike()`, `overline()`,
  `font(name AS String)`, `fontSize(n AS Integer)` — each returning an `Attribute`.
- `astrings::addAttribute(a, start, end, attr) AS AttributedString` — records `attr` over the
  **inclusive** scalar range `[start, end]`.
- `astrings::removeAttribute(a, start, end, attr) AS AttributedString` — removes matching spans within
  `[start, end]`, **splitting** spans that straddle the range (flanks survive).
- `astrings::clearAttributes(a, start, end) AS AttributedString` and
  `astrings::clearAttributes(a) AS AttributedString` — clear all attributes in a range / everywhere,
  splitting straddlers for the ranged form.
- `astrings::getAttributes(a, index) AS List OF Attribute` — the resolved attribute set at scalar
  `index`, one per enum member, per higher-start-wins.

### Non-goals (explicit constraints)

- **No merging/coalescing.** Overlapping same-member spans are stored as-is; conflicts resolve at read
  time by **higher start index wins** (tie-break: later insertion order). Never trim the loser on
  write.
- **No half-open ranges.** All range args are inclusive `[start, end]`; length = `end − start + 1`;
  `start == end` is a single scalar; there is no empty-range form.
- **No rendering, no `toMarkdown`** (that is E). B's observable is `getAttributes`.
- **No new attribute value types** beyond flag / String / Integer. `Color` is out (superseded design).

## 2. Current State

`AttributedString` (from A) is an opaque heap object with a String slot and an (empty) `List OF
Attribute` slot. `Attribute` does not yet exist. Builtin source companions
(`*_package.mfb`, glued via `include_str!`, `src/builtins/mod.rs:53` macro or the hand-rolled
`strings.rs:505`) are the idiomatic home for concrete exported types + pure-`.mfb` helpers. Native
functions that manipulate opaque layout (like `strings::mid`) are native-direct
(`src/target/shared/runtime/usage.rs:6`) and dispatch in `builder_values.rs:701`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Enums to add | 3 | `AttrTypeFlag`, `AttrTypeText`, `AttrTypeNumber` |
| Flag members (v1) | 5 | Bold, Italic, Underline, Strike, Overline |
| Wrapper records + union | 3 + 1 | `AttrFlag`/`AttrText`/`AttrNumber` + `Attribute` |
| Convenience constructors | 7 | bold, italic, underline, strike, overline, font, fontSize |
| Tier-C functions | 5 | addAttribute, removeAttribute, clearAttributes×2 (arity overload), getAttributes |

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| Scalar-index→byte-offset walk is reusable for range args | CONFIRMED | `lower_mid` (`builder_search.rs:653`) does exactly this walk |
| `Attribute` (a union) is comparable? | **NO** | map §3: unions are not comparable — so `removeAttribute` matches structurally in codegen, not via user `=` |
| Where `Attribute` model lives (source companion vs prelude) | UNVERIFIED — Phase 1 decides | try the source companion first (exported types); fall back to `ast/manifest.rs` template if the resolver needs them pre-source |

## 3. Design Overview

**Storage.** The overlay is a `List` of stored spans `(start: Integer, end: Integer, attr:
Attribute, seq: Integer)` — inclusive scalar bounds, plus an insertion sequence for the tie-break.
`addAttribute` appends; `removeAttribute`/`clearAttributes` rewrite the list, splitting straddlers.
Represent the stored span as an internal record the native functions build; it need not be
user-visible.

**Resolution (read time).** `getAttributes(a, i)` scans spans covering `i`; for each enum *member*
present, the covering span with the **highest start** (tie: highest `seq`) supplies the value; result
is at most one `Attribute` per member. Flags: presence of any covering span ⇒ member on.

**Range mapping.** `start`/`end` are visible scalar indices → converted to byte offsets via the
`lower_mid` walk when the native functions need to touch the text; but since the overlay is separate,
add/remove/clear operate purely on the integer span list — **no text mutation**, so the only scalar
work is bounds validation against the visible scalar length.

**Split (the removeAttribute/clearAttributes core).** For a stored span `[s,e]` and removal range
`[rs,re]` that overlaps: emit `[s, rs−1]` if `s ≤ rs−1`, and `[re+1, e]` if `re+1 ≤ e`; drop the
overlap. `removeAttribute` applies this only to spans whose `attr` structurally matches; ranged
`clearAttributes` to all spans; whole `clearAttributes(a)` empties the list.

**Where correctness risk concentrates (schedule last):** the split arithmetic with inclusive bounds
(off-by-one at `rs−1`/`re+1`, single-scalar spans, `rs==re`) and the resolution tie-break. Each gets
a test written against the naive implementation, including the user's worked case (bold `5–25`,
remove `10–20` → `5–9` + `21–25`).

**Rejected alternative:** *resolve/coalesce at write time.* Rejected — trimming losers on write makes
`removeAttribute` of the winner reveal nothing; read-time resolution keeps the loser recoverable, and
the user chose "no merging."

## 4. Detailed Design

### 4.1 The model (source companion `astrings_package.mfb`)
Declare the 3 enums, 3 wrapper records, `UNION Attribute`, and the 7 convenience constructors as
exported `.mfb`. Wire the source companion into the `astrings` module (`BuiltinSource`,
`WhenImported`). Keep declarations **line-neutral** on later edits to bound importer-golden churn.

### 4.2 Bounds + errors
`start`/`end` validated against the visible scalar count (reuse the count loop
`private/unicode.rs:89`): `start < 0 || end < start` → `ErrInvalidArgument (77050002)`; `end ≥
scalarLen` → `ErrIndexOutOfRange (77050001)`. An op on an empty `AttributedString` always errors.

### 4.3 Native functions
`addAttribute`/`removeAttribute`/`clearAttributes`/`getAttributes` as native-direct codegen
(`usage.rs:6`, dispatch `builder_values.rs:701`) manipulating the overlay list slot. `getAttributes`
returns a fresh `List OF Attribute`. The two `clearAttributes` arities are one callable with an
overload table entry (arity-based), mirroring `strings::find`'s optional-arg handling.

## Compatibility / Format Impact

- **New:** the `Attribute` model types (per-package wire ids ≥20), 7 constructors, 5 Tier-C
  functions, the `astrings` source companion.
- **Unchanged:** `AttributedString`'s wire id/layout from A (the overlay slot already exists);
  `String`; existing packages.

## Phases

### Phase 1 — the `Attribute` model + constructors (no storage yet)

- [x] Add `astrings_package.mfb` with the 3 enums (`AttrTypeFlag`/`AttrTypeText`/`AttrTypeNumber`),
      3 records (`AttrFlag`/`AttrText`/`AttrNumber`, field `kind`), `UNION Attribute`, and 7
      constructors; wire the `WhenImported` source companion into the `astrings` module (injected in
      resolver/syntaxcheck/ir-lower before `strings`). The public `astrings::bold()`…`fontSize()`
      map to `__astrings_*` companion bodies via `Implementation::Rewrite`.
- [x] Tests: `tests/rt-behavior/astrings/attribute-model-rt/` builds every constructor and `MATCH`es
      the union to confirm member/value (`ctors=Bold Italic Underline Strike Overline Font:Serif
      Size:14`); `tests/syntax/astrings/attribute-model-invalid/` proves the internal bridge is not
      user-callable.

Acceptance: MET. Every constructor builds and matches its expected member/value under `MATCH`.
Commit: ecb8b9503

### Phase 2 — storage + `addAttribute` + `getAttributes` + resolution

- [x] Overlay span list + `addAttribute` (source-companion body over the native `readSpans`/
      `writeSpans` bridge), inclusive bounds validation via native `scalarLen` (§4.2).
- [x] `getAttributes` with higher-start-wins + tie-break (higher `seq`) resolution, in `.mfb`.
- [x] Tests: `attribute-model-rt` covers the `FontSize:10 @[10,20]` vs `FontSize:20 @[15,25]` → `20`
      at 15 case (`size@12=Size:10`, `size@15=Size:20`), cross-member coexistence + flags OR
      (`coexist@7=Bold Italic Font:Serif`); `tests/rt-error/astrings/attribute-bounds-rt/` fires
      `ErrIndexOutOfRange` (`7-705-0001`, exit 255) for an out-of-range end.

Acceptance: MET. Resolution matches the spec on overlaps, ties, and cross-member coexistence; the
out-of-range bound fires `7-705-0001`.
Commit: ecb8b9503

### Phase 3 — `removeAttribute` + `clearAttributes` (split; correctness risk last)

- [x] `removeAttribute` (structural match on class+member+payload, inclusive-bound split), ranged
      `clearAttributes` (split all straddlers), whole `clearAttributes(a)` (empty the overlay). The
      whole vs ranged forms select distinct `.mfb` bodies by arity (`implementation_name`).
- [x] Tests: `tests/rt-behavior/astrings/mutate-split-rt/` — the worked case (bold `5–25`, remove
      `10–20` → `9=bold 15=- 21=bold`); remove-winner-reveals-loser (`before@12=20 after@12=10`);
      single-scalar span (`3=- 4=bold 5=-`); ranged clear (`1=bold 4=- 8=bold`) vs whole clear (`-`).

Acceptance: MET. Every split/reveal case passes, including the inclusive-bound edges, verified
through `getAttributes`.
Commit: ecb8b9503

## Validation Plan

- Tests: rt-behavior (construct, add/get/resolve, remove/clear split) + syntax sanity + Rust unit
  tests for the split arithmetic.
- Coverage check: rt-behavior exercises the native codegen arms.
- Runtime proof: a fixture that builds styled text and prints `getAttributes` results.
- Doc sync: `astrings` man pages for the model + Tier-C functions; spec section (finalized in E).
- Acceptance: `cargo test --bin mfb`; `artifact-gate.sh <exe> all` (importer-golden shifts expected —
  regenerate, confirm delta is only `astrings`).

## Open Decisions

1. **Model home: source companion vs `ast/manifest.rs` prelude.** Recommended source companion
   (exported `.mfb`) — matches `strings`/`json`; fall back to a prelude template only if the resolver
   needs the types before source injection.
   Decision: source companion
2. **`getAttributes` in v1.** Recommended **yes** (a reader) — it is B's only observable and unblocks
   user-authored renderers; the earlier "no reader" note is overridden by the need for a headless
   test seam.
   Decision: Yes

## Corrections

- **Tier-C is `.mfb`-over-a-native-bridge, not fully native codegen (§4.3).** The plan specified
  native-direct `addAttribute`/`removeAttribute`/`clearAttributes`/`getAttributes`. Instead they are
  source-companion `.mfb` bodies over **three** internal-only native primitives:
  `astrings::readSpans(a) AS List OF AttrSpan` (deep-copies the opaque overlay out),
  `astrings::writeSpans(a, spans) AS AttributedString` (rebuilds with a new overlay), and
  `astrings::scalarLen(a) AS Integer` (scalar count for bounds). Rationale: the correctness risk the
  plan flags — inclusive-bound split arithmetic and higher-start-wins resolution — is far safer in
  `.mfb` than in register-level native code (the plan's own letter E prefers `.mfb` for the same
  reason). No acceptance criterion is weakened; the behavior and tests are identical. The bridge is
  `internal_only` + the companion file is `internal`, so users cannot call it (proven by
  `attribute-model-invalid`).
- **The stored span is a flat internal `AttrSpan`, NOT `attr: Attribute` (§3).** Embedding the
  `Attribute` union in the overlay would tie `AttributedString`'s always-in-scope layout to the
  companion (undefined without `IMPORT astrings`). Instead `AttrSpan` flat-encodes the attribute
  (`class` 0/1/2, enum-member ordinal, String + Integer payloads) and is registered unconditionally in
  `validation.rs`; the companion declares a field-identical `AttrSpan` and converts to/from `Attribute`
  in `.mfb`.
- **Reserved-keyword field/parameter renames.** `end` is a reserved keyword (cannot be a member after
  `.`, nor a parameter/field identifier) and `type` is reserved as a field name — so the `AttrSpan`
  end-of-range field is `last`, the public range parameter is `endIndex`, and the wrapper records' type
  field is `kind`. The `.mfb` compiler surfaced these; not knowable from the Rust side.
- **Companion injection order.** `astrings::augmented_project` is injected (resolver/syntaxcheck/
  ir-lower) right after `json`; it imports only `collections` (native members) and `astrings` itself
  (the bridge), so it has no companion-ordering dependency, and the native `scalarLen` removes what
  would otherwise be a `strings`/`encoding` companion dependency.
- **Man pages** for the model + Tier-C members are added under `src/docs/man/builtins/astrings/`; the
  full spec section is E's (this plan's Validation Plan notes it is "finalized in E").

## Summary

The risk is inclusive-bound split arithmetic and read-time resolution, both fenced by tests against
the naive implementation and the user's worked example. The model is cheap (source companion);
storage and split are the real work. Untouched: A's type layout, `String`, existing packages.
