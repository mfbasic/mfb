# plan-41-D: Scalar primitive — conversions & strings seam

Last updated: 2026-07-13
Effort: medium
Depends on: plan-41-A, plan-41-B, plan-41-C

Make `Scalar` useful: the conversion built-ins (`toScalar`, `toInt(Scalar)`,
`toString(Scalar)`), the payoff — a `strings::` bridge that iterates a
`String` into a `List OF Scalar` and rebuilds a `String` from scalars — and a set
of scalar classification predicates (`strings::isLetter`, `isDigit`,
`isWhitespace`, `isUpper`, `isLower`). After this sub-plan a program can walk a
string one Unicode scalar at a time, classify/`MATCH`/compare each scalar, and
reconstruct a string — the whole reason the primitive exists.

References (read first):

- plan-41-A/B/C (the typed, serialized, code-emitting `Scalar`).
- `mfb spec language types` §4.1 (conversions) and `mfb man builtins general`
  (`toByte`, `toMoney`, `toInt`, `toString`).
- The `toByte`/`toMoney` conversion built-ins are the dispatch template; the
  `strings::toBytes` seam is the strings-bridge template (native seam + source
  companion, see the "source companion: 3 augmentation chains" pattern).

## 1. Goal

- `toScalar(Integer)` returns a `Scalar`, **fallible**: a codepoint outside
  0..10FFFF or a surrogate (D800–DFFF) fails with `ErrInvalidArgument`
  (77050002).
- `toScalar(String)` returns the single scalar of a one-scalar string, fallible
  (`ErrInvalidArgument` when the string is empty or has more than one scalar).
- `toScalar(Byte)` returns the `Scalar` for that byte value, **infallible** —
  every byte 0..255 is a valid non-surrogate codepoint (surrogates start at
  D800), so widening never fails.
- `toInt(Scalar)` returns the codepoint as `Integer`, **infallible** (0..10FFFF).
- `toByte(Scalar)` returns the byte value of a scalar, **fallible**: a codepoint
  > 255 fails with `ErrInvalidArgument` (77050002). This is the narrowing inverse
  of `toScalar(Byte)`, mirroring the `toByte(Money)`/`toByte(Integer)`
  range-narrowing precedent.
- `toString(Scalar)` returns the one-scalar UTF-8 `String`, infallible.
- `strings::toScalars(String)` returns `List OF Scalar` in scalar order;
  `strings::fromScalars(List OF Scalar)` returns the concatenated `String`.
- Round-trip identity: `strings::fromScalars(strings::toScalars(s)) = s` for any
  valid UTF-8 `String s`.
- Scalar classification predicates, each `strings::isX(Scalar) -> Boolean`,
  **infallible** (total over every valid `Scalar`):
  - `strings::isLetter(Scalar)` — true for Unicode letters (general categories
    L*: Lu, Ll, Lt, Lm, Lo).
  - `strings::isDigit(Scalar)` — true for decimal digits (general category Nd).
  - `strings::isWhitespace(Scalar)` — true for Unicode whitespace (White_Space
    property: space, tab, newline, CR, and the other WS codepoints).
  - `strings::isUpper(Scalar)` — true for uppercase letters (Uppercase property).
  - `strings::isLower(Scalar)` — true for lowercase letters (Lowercase property).

### Non-goals (explicit constraints)

- **No `Scalar` package.** Unlike `Money`, `Scalar` gets no dedicated builtin
  package/runtime — conversions live in the general builtins and the strings
  seam, mirroring `Byte`. Do not add a `scalar::` namespace.
- **No grapheme awareness.** `toScalars` iterates Unicode *scalars*, not grapheme
  clusters (that stays a separate `strings::` concern). This is the whole point
  of the name.
- **No implicit String↔Scalar coercion.** Conversion is always explicit via the
  built-ins above; a `Scalar` is not auto-usable where a `String` is expected.
- **Predicates classify a scalar, not a string.** `strings::isLetter` &c. take a
  single `Scalar`, not a `String` — a "is this whole string letters" question is
  ambiguous (all scalars? first?) and is left to the caller via `toScalars` + a
  fold. Living in `strings::` (not a `scalar::` namespace) keeps the seam with
  `toScalars`/`fromScalars` while honoring the "no `Scalar` package" constraint.

## 2. Current State

Conversion built-ins: name constants `src/builtins/general.rs:6-11` (`TO_BYTE`,
`TO_MONEY`); membership `:46-52`; return-type/param/arity tables `:86-127`
(`TO_BYTE=>Byte` :90); argument-type dispatch `resolve_call` `:203-272`
(`toString` printable set :214; `toInt`/`toByte`/`toMoney` accept-sets); help
strings `:325-346`. Conversion codegen: `src/target/shared/code/builder_conversions.rs`
(`lower_to_byte` :463, `lower_to_money` :624, `toInt(Money)` :88); inline dispatch
`src/target/shared/code/builder_values.rs:683-689,761,1401-1407`; IR-validate
allowlist `src/target/shared/validate.rs:1309-1313`; static const-fold return
maps `src/target/shared/code/data_objects.rs:1075-1077`,
`builder_value_semantics.rs:681-683`; toString-of-constant folding
`type_utils.rs:190-215` (Byte :202). IR-lower return hint for `toByte` is
`src/ir/lower.rs:2458` (`Some("Byte")`).

Strings seam: `strings::toBytes` is the precedent — a native seam plus a source
companion (`strings` package `.mfb` + the augmentation wiring across
syntaxcheck/resolver/ir-lower described in the "source companion" pattern). The
new `toScalars`/`fromScalars` mirror it: `toScalars` decodes UTF-8 into
codepoints (each a `Scalar`), `fromScalars` UTF-8-encodes each scalar and
concatenates.

`Byte` is the exact conversion template (no package, general-builtin dispatch);
`strings::toBytes` is the exact strings-seam template.

## 3. Design Overview

Two layers:

1. **General conversions** (mirror `toByte`): register `TO_SCALAR`; add
   `toScalar(Integer)` / `toScalar(String)` accept-arms (fallible, with the
   codepoint-validity check reusing the same surrogate/range predicate as the
   plan-41-A literal check) and `toScalar(Byte)` (infallible zero-extend — a byte
   is always a valid scalar); add `toInt(Scalar)` (infallible, zero-extend the
   32-bit codepoint to i64) and `toByte(Scalar)` (fallible narrow: trap when the
   codepoint > 255, reusing the `toByte` range-narrowing arm); add `Scalar` to the
   `toString` printable-arg set with a `lower` arm that UTF-8-encodes one scalar
   into a `String`. Wire the codegen (`lower_to_scalar`, the new `toByte(Scalar)`
   and `toScalar(Byte)` arms, `toInt`/`toString` scalar arms), the inline
   dispatch, the IR-validate allowlist, the static const-fold maps, and the
   `toScalar` return hint in `ir::lower`.
2. **Strings seam** (mirror `toBytes`): `strings::toScalars(String) -> List OF
   Scalar` and `strings::fromScalars(List OF Scalar) -> String`, as a native seam
   + source companion, threaded through the same augmentation chain as `toBytes`.
3. **Classification predicates** (source companion, no native seam):
   `strings::isLetter/isDigit/isWhitespace/isUpper/isLower(Scalar) -> Boolean`,
   authored in the `strings` package `.mfb` companion. They test scalar membership
   against the Unicode property ranges — reuse the regex package's Unicode table
   (see the "regex package impl" range-table pattern) rather than growing a new
   one, and expose the needed ranges (L*, Nd, White_Space, Uppercase, Lowercase)
   so both packages share one source of truth. Because they are pure `Scalar ->
   Boolean` source functions over `toInt(Scalar)`, they need no codegen, wire, or
   IR-validate changes beyond the source-companion wiring.

**Risk concentrates in the UTF-8 boundary**: `toString(Scalar)`/`fromScalars`
must emit correct UTF-8 for 1–4-byte scalars, and `toScalars` must decode
correctly and never produce a surrogate — the encode/decode must be exact
inverses. The `toScalar` fallible-validity path (surrogate/range → trap) is the
other correctness-sensitive spot; reuse the plan-41-A predicate so literal and
runtime agree.

Rejected alternatives:
- *Make `toString(Scalar)` fallible* — rejected; every valid `Scalar` is a valid
  UTF-8 string, so the conversion cannot fail. Only `toScalar` (the narrowing
  direction) is fallible.
- *Provide `scalarAt(String, i)` instead of `toScalars`* — rejected as the
  primary API; index-based scalar access over UTF-8 is O(n) and error-prone.
  `toScalars` gives an ordered `List OF Scalar` that composes with existing
  collection helpers. (`scalarAt` may be a later convenience, not this plan.)

## Compatibility / Format Impact

Additive: new built-in names (`toScalar`, `strings::toScalars`,
`strings::fromScalars`, `strings::isLetter`/`isDigit`/`isWhitespace`/`isUpper`/
`isLower`) and new dispatch arms on existing `toInt`/`toByte`/`toString`
(`toScalar` also gains a `Byte` arm). No change to existing conversions or to the
wire format. `toString(Scalar)`, `toInt(Scalar)`, `toByte(Scalar)`, and
`toScalar(Byte)` extend overload sets without altering existing overloads. The
predicates are pure source companions — additive `strings::` functions with no
runtime/format impact.

## Phases

### Phase 1 — toScalar / toInt(Scalar) / toString(Scalar)

- [ ] Add `TO_SCALAR` and its tables in `src/builtins/general.rs` (name const
      `:6-11`, membership `:46-52`, return/param/arity `:86-127`, help `:325-346`);
      add the accept-arms in `resolve_call` `:203-272` — `toScalar(Integer)`,
      `toScalar(String)` (fallible) and `toScalar(Byte)` (infallible);
      `toInt(Scalar)` (infallible) and `toByte(Scalar)` (fallible — extend the
      `toByte` accept-set with `Scalar`); and `Scalar` in the `toString` printable
      set (:214).
- [ ] Codegen: `lower_to_scalar` (fallible for Integer/String — surrogate/range
      trap → `ErrInvalidArgument`; infallible zero-extend for `Byte`) +
      `toInt(Scalar)` (zero-extend) + `toByte(Scalar)` (fallible narrow → trap when
      codepoint > 255, in the existing `lower_to_byte` :463 arm) +
      `toString(Scalar)` (UTF-8 encode one scalar) in
      `src/target/shared/code/builder_conversions.rs`; inline dispatch in
      `builder_values.rs:683-689,761,1401-1407`; IR-validate allowlist
      `validate.rs:1309-1313`; static const-fold maps
      (`data_objects.rs:1075-1077`, `builder_value_semantics.rs:681-683`,
      `type_utils.rs:190-215`); `toScalar` return hint in `ir::lower` near
      `:2458`.
- [ ] Tests: unit + runtime — `toScalar(65) = `` `A` ``, `toScalar(0xD800)`
      traps, `toScalar("A") = `` `A` ``, `toScalar("ab")` traps,
      `toScalar(toByte(65)) = `` `A` ``, `toInt(`` `A` ``) = 65`,
      `toByte(`` `A` ``) = 65` (Byte), `toByte(`` `\u{1F600}` ``)` traps,
      `toString(`` `\u{1F600}` ``)` = the emoji string.

Acceptance: a program exercising all six conversions (including the three failing
cases via `TRAP`: `toScalar(surrogate)`, `toScalar(multi-scalar String)`, and
`toByte(codepoint > 255)`) prints the expected values/codes on all three
backends.
Commit: —

### Phase 2 — strings::toScalars / fromScalars + round-trip (highest-risk last)

- [ ] Add `strings::toScalars(String) -> List OF Scalar` and
      `strings::fromScalars(List OF Scalar) -> String`, mirroring the
      `strings::toBytes` native seam + source companion; thread through the
      syntaxcheck/resolver/ir-lower augmentation chain.
- [ ] Tests: runtime round-trip over ASCII, multibyte (é, 中), and astral
      (emoji) strings — assert `fromScalars(toScalars(s)) = s`, and that
      `toScalars` yields the expected codepoint list.

Acceptance: `strings::fromScalars(strings::toScalars(s)) = s` for ASCII,
multibyte, and astral inputs on all three backends; iterating a string into
scalars, comparing/`MATCH`ing them, and rebuilding produces the original string.
Commit: —

### Phase 3 — scalar classification predicates

- [ ] Add `strings::isLetter/isDigit/isWhitespace/isUpper/isLower(Scalar) ->
      Boolean` to the `strings` source companion, testing `toInt(Scalar)` against
      the shared Unicode property ranges (reuse/extend the regex package's range
      table for L*, Nd, White_Space, Uppercase, Lowercase). Thread the five names
      through the same syntaxcheck/resolver/ir-lower augmentation chain as
      `toScalars`/`fromScalars`; no codegen/wire/IR-validate change needed.
- [ ] Tests: runtime truth-table over representative scalars per predicate —
      ASCII (`` `A` ``, `` `z` ``, `` `5` ``, `` ` ` `` space, `` `\t` ``), a
      punctuation/symbol scalar (all-false), a non-ASCII letter (`` `é` ``,
      `` `中` ``), a non-ASCII digit, and the boundary cases (empty-of-category).
      Confirm each predicate is total (no trap) over every input.

Acceptance: each predicate returns the correct Boolean for ASCII, non-ASCII, and
non-letter/non-digit scalars on all three backends, never trapping; a "walk a
string, keep only letters/digits, rebuild" program composes them with
`toScalars`/`fromScalars` and produces the expected filtered string.
Commit: —

## Validation Plan

- Tests: general-builtin unit tests + runtime conversion tests (Phase 1); strings
  round-trip runtime tests over ASCII/multibyte/astral (Phase 2); predicate
  truth-table runtime tests over ASCII/non-ASCII/non-alnum scalars (Phase 3), all
  under the repo's acceptance/rt-behavior folders with goldens.
- Runtime proof: a "walk a string, uppercase ASCII scalars via compare, rebuild"
  program that runs on all three backends and produces the transformed string —
  demonstrating the primitive is actually useful.
- Doc sync: new `toScalar` man page + `toString`/`toInt` overload updates +
  `strings::toScalars`/`fromScalars` man pages — authored in plan-41-E.
- Acceptance: `cargo test` green; cross-arch runtime green; no leaks in the
  string-walk program (the `List OF Scalar` and rebuilt `String` must drop
  cleanly).

## Open Decisions

_Resolved 2026-07-13 (user)._

- **`toByte(Scalar)` / `toScalar(Byte)` — DECIDED: include both.** (Recommendation
  had been to omit for now.) The pair is symmetric with the other conversions:
  `toScalar(Byte)` is an infallible widen (every byte is a valid scalar) and
  `toByte(Scalar)` is a fallible narrow (traps on codepoint > 255), matching the
  `toByte(Integer)`/`toByte(Money)` range-narrowing precedent. Folded into
  Phase 1. (§1)

## Summary

Risk is the UTF-8 encode/decode boundary (`toString`/`fromScalars` ↔
`toScalars`) and the fallible `toScalar` validity check — both pinned by
round-trip and trap tests across ASCII/multibyte/astral on three backends. The
five classification predicates (`isLetter`/`isDigit`/`isWhitespace`/`isUpper`/
`isLower`) ride the source companion over shared Unicode ranges, adding no
codegen risk. This is the sub-plan that makes `Scalar` worth having.
