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

- [ ] Classify all 39 `STRINGS_FUNCTIONS` into Tier-A / Tier-B / N-A by reading their signatures;
      record the table here.
- [ ] Tests: none (analysis) — but the table is the acceptance artifact D depends on.

Acceptance: a complete, per-function tier table covering all 39, committed in this plan.
Commit: —

### Phase 2 — implement Tier-A overloads

- [ ] Add the `AttributedString` overload + text-slot codegen preamble for each Tier-A function.
- [ ] Tests: `tests/rt-behavior/astrings/tier-a-queries-rt/` — for a styled `AttributedString`,
      assert each Tier-A function equals its `strings::q(toString(a))` counterpart (same value/type),
      including an error case (e.g. `find` not-found propagates identically).

Acceptance: every Tier-A overload equals the `String`-of-plaintext result and matches error behavior.
Commit: —

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

<!-- Filled in during execution — especially any function that moves tier after reading its signature. -->

## Summary

Low-risk and mechanical; the only real work is Phase 1's exact partition, which is also the contract
D consumes. Untouched: every `String` overload and all Tier-B/transform functions.
