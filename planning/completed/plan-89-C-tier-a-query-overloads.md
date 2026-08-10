# plan-89-C: Tier-A query overloads (`strings::` accepts `AttributedString`)

Last updated: 2026-08-08
Effort: medium (1h–2h)
Depends on: **plan-89-A** (the `AttributedString` type + `toString` extraction seam). Does **not**
depend on B — Tier-A queries read only the visible text, never the attribute overlay. If A is not
complete, C cannot start, full stop.

Makes the read-only, non-transforming `strings::` functions accept an `AttributedString` by operating
on its visible text and returning exactly what the `String` version returns. Each overload is a thin
wrapper that extracts the text slot and reuses the existing `String` code path.

**Single behavioral outcome:** for every Tier-A function `q`, `strings::q(a, …)` equals
`strings::q(toString(a), …)` for any `AttributedString a` — same value, same type, same errors.

References (read first): plan-89-A §2 (`toString` overload precedent, resolver + `lower_to_string`);
`src/builtins/strings.rs:235` `STRINGS_FUNCTIONS`; `src/builtins/mod.rs:385`
`resolve_call_return_type`; dispatch `src/target/shared/code/builder_values.rs:701`.

## Prerequisites

See plan-89-A. plan-89-A landed. (B may or may not be landed — irrelevant to C.)

## 1. Goal

- The Tier-A subset of the 39 `strings::` functions accepts `AttributedString` as its text argument,
  returning the same result type as the `String` overload (`Bool`/`Integer`/`List OF String`/`List OF
  Byte`/`List OF Scalar`/`String`), computed on the visible text.

### Non-goals (explicit constraints)

- **No attribute awareness.** Tier-A results never depend on or carry attributes; they read the text.
- **No transform overloads.** Functions that return a new *string* value (`mid`, `trim`, `replace`,
  …) are Tier-B and belong to plan-89-D — do not overload them here.
- **No implicit coercion.** The overload is explicit dispatch, not a `String` coercion.
- **`String` behavior unchanged.**

## 2. Current State

`strings::` functions resolve return types via each module's resolver
(`src/builtins/strings.rs` `STRINGS_RESOLVER`, aggregated at `mod.rs:385`) and lower via native-direct
dispatch (`builder_values.rs:701`). `toString(AttributedString)` (from A) already extracts the text
slot in `lower_to_string` (`builder_strings.rs:757`) — the same slot-load these overloads reuse.

### Measured populations

| What | Count | Command / basis |
|---|---|---|
| Total `strings::` functions | 39 | `grep -cE '^const [A-Z_]+: &str = "strings\.' src/builtins/strings.rs` → 39 |
| Tier-A candidates (query, from `mfb man strings`) | ~16 | byteLen, contains, count, displayWidth, endsWith, endsWithAny, find, graphemes, graphemesCount, split, startsWith, startsWithAny, toBytes, toScalars, (graphemeAt — see Open Decisions), (indexOf if present) |
| Functions taking no `String` primary arg (N/A) | 7 | fromScalars, join, isDigit, isLetter, isLower, isUpper, isWhitespace |

> The exact Tier-A membership is Phase 1's first task — derived by reading `STRINGS_FUNCTIONS`
> signatures, not assumed. The count above is from the man categorization, to be confirmed.

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| The text-slot load is reusable per overload | CONFIRMED | it is exactly what A's `lower_to_string` arm does |
| A first-arg `AttributedString` overload can share the `String` return type | CONFIRMED | resolver returns type from `arg_types`; add an `"AttributedString"` first-arg case returning the same type as `"String"` |

## 3. Design Overview

For each Tier-A function, add an overload entry accepting `AttributedString` at the text position
(resolver returns the same type as the `String` overload), and in codegen emit a preamble that loads
the text slot into a `String` value, then falls through to the existing `String` lowering. This is
mechanical and low-risk — the risk is only **completeness** (missing a function) and **mis-tiering**
(overloading a transform that should be Tier-B). Phase 1 nails the exact list from the source table.

Rejected alternative: *a single generic "coerce AttributedString→String at any String param"
shim.* Rejected — the user forbade implicit coercion; explicit per-function overloads keep the
surface intentional and let Tier-B diverge (returning `AttributedString`, not `String`).

## 4. Detailed Design

### 4.1 Tier assignment (Phase 1)
Read `STRINGS_FUNCTIONS` (`strings.rs:235`); classify each by the **hard rule**: *if a function
**modifies** the string (its result re-expresses the input's text — narrowed, extended, or rewritten),
it MUST return `AttributedString`; if it **interrogates** the string (returns a measurement, a
position, or a decomposition into a collection), it may return whatever the `String` overload
returns.* Operational test: does the result re-express the input's text, or answer a question about it?
- **Tier-B** (defer to D, → `AttributedString`) = *modifiers*: `mid`/`left`/`right` (window),
  `trim*`/`stripPrefix`/`stripSuffix` (trim), `padLeft`/`padRight` (extend), `repeat`, `replace`, and
  `upper`/`lower`/`caseFold`/`normalizeNfc` (recase). The case/normalize four return `AttributedString`
  to satisfy the rule but carry an **empty** overlay — the rule governs the return *type*, not span
  preservation (they cannot map spans; see D §3).
- **Tier-A** (→ plain) = *interrogators*: returns non-string (`byteLen`, `contains`, `find`, `count`,
  `displayWidth`, `endsWith*`, `startsWith*`, …) OR a decomposition into a collection (`split`,
  `graphemes`, `toBytes`, `toScalars`). An indexed pluck (`graphemeAt`) is an interrogation
  (array-index semantics), plain in v1 — see Open Decisions.
- **N/A** = no `String` primary arg.
Record the final table in this plan's Corrections/Detailed Design as the authority D also reads.

### 4.2 Overload wiring per Tier-A function
Add the `AttributedString` first-arg overload to the resolver (same return type as `String`) and a
codegen preamble that loads the text slot, then reuses the `String` arm. No new runtime helpers.

## Compatibility / Format Impact

- **New:** `AttributedString` overloads of the Tier-A `strings::` functions.
- **Unchanged:** all `String` overloads; return types; error codes.

## Phases

### Phase 1 — freeze the Tier-A/B/N-A partition

- [x] Classified all 39 `STRINGS_FUNCTIONS` by reading their `STRINGS_FUNCTIONS`/OV signatures. The
      frozen partition (the authority plan-89-D consumes; codified as
      `builtins::strings::is_tier_a_query`):

      **Tier-A (15) — interrogators → plain result on visible text:** `byteLen`, `contains`, `count`,
      `displayWidth`, `endsWith`, `endsWithAny`, `find`, `graphemes`, `graphemesCount`, `split`,
      `startsWith`, `startsWithAny`, `toBytes`, `toScalars`, `graphemeAt` (indexed pluck = array-index
      interrogation → plain `String`, per Open Decision 1).

      **Tier-B (17) — modifiers → `AttributedString` (plan-89-D):** `left`, `right`, `mid`, `trim`,
      `trimStart`, `trimEnd`, `trimChars`, `stripPrefix`, `stripSuffix`, `padLeft`, `padRight`,
      `repeat`, `replace`, `upper`, `lower`, `caseFold`, `normalizeNfc` (the last four drop attributes,
      D §3).

      **N-A (7) — no `String` primary arg:** `fromScalars` (`List OF Scalar`→String), `join`
      (`List OF String`), `isLetter`, `isDigit`, `isWhitespace`, `isUpper`, `isLower` (all take a
      `Scalar`).

      15 + 17 + 7 = 39. Tier-A and Tier-B use disjoint OV tables (interrogators return non-String or a
      collection; modifiers return `String`), so C touches no Tier-B overload.
- [x] Tests: none (analysis) — the table above is the acceptance artifact D depends on.

Acceptance: MET. Complete per-function tier table covering all 39, committed here and codified as
`is_tier_a_query`.
Commit: 370130660

### Phase 2 — implement Tier-A overloads

- [x] Accept an `AttributedString` at the text position for each Tier-A function. Implemented as a
      resolver override (`StringsResolver::resolve_return_type` substitutes `String` for a leading
      `AttributedString` and reuses the `String` resolution) plus a single IR-lowering rewrite
      (`ir/lower.rs`: wrap the leading arg in `toString(a)` for a Tier-A query) — so BOTH the native
      arms (`contains`/`byteLen`/…) AND the source-companion rewrite arms (`toScalars`) receive a
      `String`. (Corrections: the plan's "codegen preamble" is realized at IR lowering, which is the
      only point before the native-vs-rewrite split; a codegen-only preamble missed `toScalars`.)
- [x] Tests: `tests/rt-behavior/astrings/tier-a-queries-rt/` — every Tier-A function on a styled
      `AttributedString` equals its `strings::q(toString(a))` counterpart (all `X/X`), plus `find`
      not-found error parity (`trap:77050004/77050004`).

Acceptance: MET. Every Tier-A overload equals the `String`-of-plaintext result and matches error
behavior.
Commit: 370130660

## Validation Plan

- Tests: rt-behavior equality suite across the Tier-A set + error propagation.
- Coverage check: the fixture calls each overload (the codegen preamble is in the denominator).
- Runtime proof: the equality fixture.
- Doc sync: note `AttributedString` acceptance in the affected `strings::` man pages.
- Acceptance: `cargo test --bin mfb`; `artifact-gate.sh <exe> all`.

## Open Decisions

1. **`graphemeAt`.** Returns a single grapheme as `String`. Tier-A (plain `String` result) vs Tier-B
   (return an `AttributedString` carrying that grapheme's attributes). Under §4.1's hard rule this is
   the one boundary case: an indexed pluck reads like `mid` but is really array-index semantics = an
   interrogation, so it stays plain.
   Decision: Tier-A (v1; revisit if a caller needs the styled grapheme)
2. **`graphemes`/`split`/`toScalars` (list results).** Recommended Tier-A returning plain `List OF
   String`/… — attribute-preserving splitting is out of scope for v1.
   Decision: Tier-A

## Corrections

- **Overload wiring is a resolver override + IR-lowering rewrite, not per-function descriptor
  overloads + a codegen preamble (§4.2).** A codegen-only preamble (loading the text slot) would have
  missed `toScalars`, whose Tier-A member is a source-companion rewrite (`__strings_toScalars`)
  validated against a `String` param at the IR level — before codegen. So the leading argument is
  wrapped in `toString(a)` once, at IR lowering (`ir/lower.rs`), which is the single point before the
  native-vs-companion split; every Tier-A member (native and rewrite) then receives a `String`. The
  `StringsResolver` return-type override makes the frontend accept the `AttributedString` argument
  (substitutes `String`, reuses the existing resolution) so no `String`-overload golden churns.
- **Tier-A membership matched the plan's estimate (15, not "~16").** The stray "indexOf if present"
  candidate does not exist in `STRINGS_FUNCTIONS`; `graphemeAt` stays Tier-A per Open Decision 1.
- **Doc sync** notes `AttributedString` acceptance in the `strings` package overview (one edit listing
  the Tier-A set) rather than editing all 15 per-function pages.

## Summary

Low-risk and mechanical; the only real work is Phase 1's exact partition, which is also the contract
D consumes. Untouched: every `String` overload and all Tier-B/transform functions.
