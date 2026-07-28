# plan-29-A: Money type — front-end typing, dimensional lattice & literals

Last updated: 2026-07-07
Overall Effort: x-large (1d–3d — the whole plan-29 feature)
Effort: medium (1h–2h)

This sub-plan adds the new scalar type `Money` to the compiler front end: the type
name, its **dimensional** place in the arithmetic lattice, its literal typing, and
every front-end predicate (numeric / comparable / orderable / defaultable / literal
range). No IR-constant lowering, no `.mfp` encoding, and no native codegen — those
land in plan-29-B..F. After this sub-plan a program that declares, binds, adds, and
scales `Money` values **type-checks** and passes `ir::verify`, and every disallowed
mixing (`M + scalar`, `M * M`, `M` compared to a scalar) is **rejected at compile
time**; it cannot yet be lowered to a binary (that is C onward), so acceptance here is
unit-test + syntaxcheck driven.

`Money` is a 64-bit signed integer carrier interpreted as a base-10 fixed-point value
scaled to **5 decimal places** (SCALE = 100000). One unit = 0.00001; `1.00000` is raw
i64 `100000`. Range `-92233720368547.75808` … `92233720368547.75807`. It is **exact
decimal** (the spec §4.1 anticipated "a future exact base-10 financial type … must
specify decimal scale, rounding, and overflow rules separately"). Crucially, `Money`
is not "another number format" — it is a **financial quantity with a currency
dimension**. Its algebra is dimensional: you may add two amounts, scale an amount by a
dimensionless number, and divide two amounts to get a ratio — but you may not add a
bare number to an amount, multiply two amounts, or compare an amount to a bare number.

It complements:

- `./mfb spec language types` (§4.1 primitives, §4.10 defaults, §4.11 comparable/orderable, §4.12 literal-range checks — all gain `Money`; canonical spec `src/docs/spec/language/04_types.md`)
- `./mfb spec language type-inference` (untyped decimal literal → `Money` under expectation / `m` suffix)
- `./mfb spec diagnostics rule-codes` (new `TYPE_MONEY_LITERAL_*` and `TYPE_MONEY_OPERATION_INVALID` rules)

## 1. Goal

- `Money` is a recognized scalar type name everywhere `Fixed` is: it parses in a type
  annotation, resolves as a built-in type, is numeric, defaultable (default
  `0.00000`), comparable, and orderable — **but only against another `Money`**.
- The **dimensional lattice** (below) is enforced: same-dimension add/subtract,
  scalar scaling for `*`/`/`, ratio for `M / M`, and a compile-time error for every
  dimensionally-invalid pairing.
- A decimal literal acquires type `Money` from an expected `Money` type and from an
  `m`/`M` suffix (**decided** — mirrors the `f`=Float / `F`=Fixed suffixes; `1.25m` is
  Money); an out-of-range `Money` literal is rejected statically with
  `TYPE_MONEY_LITERAL_OVERFLOW`/`UNDERFLOW`.

### The Money dimensional algebra (the contract this sub-plan enforces)

Let `k` be any *dimensionless* numeric (`Integer`, `Byte`, `Float`, `Fixed`) and `M` be `Money`:

| Operator | Valid forms → result | Rejected forms |
|---|---|---|
| `+`, `-` | `M , M → M` | `M , k` and `k , M` → error |
| `*` | `M , k → M` ; `k , M → M` | `M , M` → error |
| `/` | `M , k → M` ; `M , M → Float` | `k , M` → error |
| `DIV` | `M , M → Float` ; `M , k → Float` | `k , M` → error |
| `MOD` | `M , M → M` | `M , k` and `k , M` → error |
| `^` | — | any operand `M` → error |
| unary `-` | `M → M` | — |
| `= <> < > <= >=` | `M , M → Boolean` | `M , k` and `k , M` → error |

### Non-goals (explicit constraints)

- **No change to `Fixed`, `Float`, `Integer`, or `Byte` typing.** Existing lattice
  rows and all current goldens stay byte-identical. The Money rules are additive: they
  only fire when at least one operand is `Money`.
- **No implicit conversion / no casting.** MFB has no casts; crossing into or out of
  `Money` is always an explicit `toMoney(...)` / `to*(money)` call (plan-29-G). The
  dimensional errors above are *not* softened by silent promotion.
- **`Money * Money`, `Money ± scalar`, `scalar / Money`, `Money MOD scalar`, `Money ^ n`,
  and `Money`-vs-scalar comparisons are compile errors**, not runtime failures.
- No new language surface beyond the type name + optional literal suffix; no change to
  value/copy/move semantics (Money is a plain 8-byte scalar) or the flat type-name
  encoding (bare base id `Money`).
- No codegen, no IR constant, no `.mfp` change in this sub-plan.

## 2. Current State

Numeric type-name constants: `src/numeric.rs:1-4`; `is_numeric_type` whitelists the
four (`:163`). Literal lattice `LiteralType { Integer, Float, Fixed }` (`:10-14`),
decided by `classify_literal` (`:27`). Promotion result type `binary_result_type`
(`:146`): DIV→Float, else Fixed dominates, then Float, then Byte+Byte, else Integer —
**commutative and operator-coarse today** (same result for `+ - * ^ / MOD`). Money
requires making it operator-aware and order-aware for the Money rows.

Built-in type-name resolution: `src/resolver/mod.rs:5-19` (`BUILTIN_TYPES`). Parseable
type: `src/syntaxcheck/types.rs:63` (`"Fixed" => Type::Fixed`), the `Type::Fixed`
variant + name mappings `src/syntaxcheck/mod.rs:31,932,1897,1912`, and the
literal/numeric/name mapping arms `src/syntaxcheck/helpers.rs:245,289,304,314`, with
the numeric-literal-into-slot coercion at `types.rs:200-202,225,246`. Comparison /
equality acceptance and the numeric-type sets live in `src/ir/verify/mod.rs`
(numeric closures `:1547,:1747`; other numeric enumerations `:146,1210,1849,2254,2640,2963`;
comparable/orderable/`is_defaultable` predicates — grep `Fixed`). Static Fixed
literal-range check `:1592-1597,1623-1628`; the two Fixed literal rules
`src/rules/table.rs:382-393` (`2-203-0017/0018`). `monomorph/lower.rs:1450` maps
`LiteralType::Fixed => "Fixed"`. Lexer `f`/`F` suffix scan `src/lexer.rs:528-540`.

Precedent to mirror for *type identity, storage, and literals*: **`Fixed`**. The
**lattice and comparison rules are new** (Fixed has no dimensional restrictions).

One adjacent surface deliberately **not** touched here: `src/builtins/math.rs` gates
its overloads through a *local* `is_numeric` whitelist (`math.rs:238`,
`Integer|Float|Fixed`) plus per-function acceptance arms, so widening
`numeric::is_numeric_type` in this sub-plan does **not** auto-enable any `math::`
function for Money. The deliberate `math::` Money surface (abs/min/max/clamp/
floor/ceil/round accepted; pow and the transcendentals rejected) is specified in
plan-29-G §4.7.

## 3. Design Overview

Three pieces:

1. **Type identity** — add `TYPE_MONEY = "Money"`; register it in `BUILTIN_TYPES`, the
   `Type` enum + all syntaxcheck mapping arms, `is_numeric_type`, and the
   `is_defaultable` predicate (default `0`). Money is comparable **and** orderable, but
   the comparability/ordering *pairing* rule is restricted to Money-vs-Money (piece 3).

2. **Dimensional lattice** — make `binary_result_type` operator- and order-aware for
   any pairing that includes `Money`, implementing the table in §1 (new helper
   `money_result_type`). A rejected pairing returns `None`, surfaced by syntaxcheck as
   an operand-type error; add a dedicated `TYPE_MONEY_OPERATION_INVALID` rule so the
   message explains *why* (e.g. "cannot multiply two Money values" / "cannot add a
   Money and a non-Money value") rather than a generic mismatch.

3. **Comparison restriction + literal typing** — the `=`/`<>` "any two numerics" rule
   and the ordering rule both gain a Money guard: if either operand is `Money`, both
   must be `Money`, else `TYPE_MONEY_OPERATION_INVALID`. A decimal literal types as
   `Money` under expectation / `m` suffix, and is range-checked exactly via
   `money_raw_from_decimal` (plan-29-B; land the helper here if B has not).

Correctness risk concentrates in pieces 2 and 3 — the *rejection* paths. Getting every
invalid pairing to fail (and every valid one to pass) without disturbing non-Money
numerics is the whole game; it is covered by an exhaustive pairing unit test.

## 4. Detailed Design

### 4.1 Type constants & predicates
- `src/numeric.rs`: `pub(crate) const TYPE_MONEY: &str = "Money";`; add to
  `is_numeric_type`; add `Money` to `LiteralType`.
- `src/resolver/mod.rs:5-19`: `"Money"` in `BUILTIN_TYPES`.
- `src/syntaxcheck`: `types.rs:63` name→`Type::Money`; `mod.rs:31` `Type::Money`
  variant + mappings (`:932,1897,1912`); `helpers.rs:245,289,304,314` literal/numeric/
  name arms; coercion `types.rs:200-202,225,246`.
- `src/ir/verify/mod.rs`: `Money` in the numeric closures (`:1547,:1747`) and the other
  numeric enumerations (`:146,1210,1849,2254,2640,2963`); Money in the comparable and
  orderable predicates; `is_defaultable` → defaultable, default `0`.

### 4.2 Dimensional lattice (`binary_result_type`, `src/numeric.rs:146`)
```
if !is_numeric_type(left) || !is_numeric_type(right) { return None; }
let l = left == TYPE_MONEY; let r = right == TYPE_MONEY;
if l || r { return money_result_type(operator, l, r); }
// ... existing non-money rules unchanged ...

fn money_result_type(op, l_money, r_money) -> Option<&'static str> {
    match op {
        "+" | "-"        => (l_money && r_money).then_some(TYPE_MONEY),
        "*"              => if l_money && r_money { None } else { Some(TYPE_MONEY) },
        "/"              => if l_money && r_money { Some(TYPE_FLOAT) }
                            else if l_money { Some(TYPE_MONEY) } else { None },
        "DIV"            => if l_money { Some(TYPE_FLOAT) } else { None },
        "MOD"            => (l_money && r_money).then_some(TYPE_MONEY),
        _ /* "^" etc. */ => None,
    }
}
```
Unit-test the full §1 table (both operand orders, every operator, `k` ∈ each of the
four scalars).

### 4.3 Comparison restriction
Wherever comparison/equality operand compatibility is decided (syntaxcheck comparison
type-check + the `ir::verify` equality/ordering acceptance around the `:1747` numeric
closure): before applying the "any two numerics compare" rule, guard that
`is_money(left) == is_money(right)` when either is Money — otherwise emit
`TYPE_MONEY_OPERATION_INVALID`. Money-vs-Money uses the ordinary signed-i64 order.

### 4.4 Literal typing & static range
- **Suffix (decided):** mirror `f`=Float / `F`=Fixed with `m`/`M`=Money. Extend the
  lexer suffix scan (`src/lexer.rs:528-540`) to admit `m`/`M` alongside `f`/`F`, and
  `classify_literal` (`src/numeric.rs:27`) to map the `m`/`M` suffix →
  `LiteralType::Money` (stripped value returned parse-ready). Both cases map to Money
  (there is only one money type, so — unlike f/F — case is not load-bearing here; the
  lexer tests at `src/lexer.rs:1403,1418` gain `m`/`M` cases).
- `monomorph/lower.rs:1450`: `LiteralType::Money => "Money"`.
- Expected-type → `Money` coercion: mirror the Fixed path in `ir/lower.rs` (the typing
  half here; constant-raw emission in plan-29-B).
- Static range: beside the Fixed check in `ir/verify` (`:1592-1628`), compute the exact
  raw via `money_raw_from_decimal`; `Err(out-of-range)` → `TYPE_MONEY_LITERAL_OVERFLOW`
  (positive) / `UNDERFLOW` (negated).

### 4.5 Diagnostic rules (`src/rules/table.rs`, after `:393`)
Next free `2-203-00NN` codes:
```
TYPE_MONEY_LITERAL_OVERFLOW   — "numeric literal is outside the Money range"
TYPE_MONEY_LITERAL_UNDERFLOW  — "numeric literal is outside the Money range"
TYPE_MONEY_OPERATION_INVALID  — "operation is not valid for Money operands"
```
Mirror into `src/docs/spec/diagnostics/01_rule-codes.md` in the same commit.

## Layout / ABI Impact

None here (Money will be an 8-byte scalar in plan-29-C; flat type-name encoding
unchanged — bare `Money`). No `.mfp` change (plan-29-B).

## Phases

### Phase 1 — Type identity & predicates
Money is a numeric, defaultable, comparable/orderable (Money-only) scalar that resolves and parses.

- [ ] `src/numeric.rs` (`TYPE_MONEY`, `LiteralType::Money`, `is_numeric_type`);
      `resolver/mod.rs` `BUILTIN_TYPES`.
- [ ] `syntaxcheck` name/variant/mapping arms (`types.rs:63`, `mod.rs:31,932,1897,1912`,
      `helpers.rs:245,289,304,314`, coercion `types.rs:200-202,225,246`).
- [ ] `ir/verify` numeric closures + enumerations + comparable/orderable +
      `is_defaultable` (default `0`).
- [ ] Unit + syntaxcheck fixtures: `AS Money` parses; `MUT m AS Money` defaults;
      `Map OF Money TO String` and `sortBy` over Money accepted.

Acceptance: a fixture binding/defaulting/comparing two Money values type-checks; Money
map-key + sort accepted. Unit tests pass.
Commit: —

### Phase 2 — Dimensional lattice
`binary_result_type` implements the §1 table; invalid pairings are rejected.

- [ ] `src/numeric.rs`: `money_result_type` per §4.2, wired into `binary_result_type`.
- [ ] Exhaustive unit test of the §1 table (both orders, all operators, all scalars).
- [ ] `rules/table.rs` + spec: `TYPE_MONEY_OPERATION_INVALID`.
- [ ] `_invalid` syntaxcheck fixtures: `M + 5`, `M * M`, `5 / M`, `M MOD 3`, `M ^ 2`
      each rejected; `_valid`: `M + M`, `M * 3`, `M * 1.08`, `M * aFixed`, `M / 4`,
      `M / M` (Float), `M MOD M`.

Acceptance: every §1 valid form type-checks with the correct result type; every
rejected form emits `TYPE_MONEY_OPERATION_INVALID`; all existing numeric goldens
unchanged.
Commit: —

### Phase 3 — Comparison restriction & literals
Money compares only with Money; decimal literals type as Money and are range-checked.

- [ ] `syntaxcheck` + `ir/verify` comparison/equality guard (§4.3).
- [ ] `classify_literal` `m`/`M` + lexer suffix; `monomorph/lower.rs:1450`;
      `ir/lower.rs` expected-type typing; static range check (§4.4);
      `TYPE_MONEY_LITERAL_*` rules.
- [ ] Tests: `_valid` (`LET a AS Money = 1.25`, `LET b = 1.25m`, `M = M`) + `_invalid`
      (`M < 5`, out-of-range literal over/underflow).

Acceptance: `1.25m` types as Money; `money = money` ok but `money = 5` rejected; an
out-of-range literal is rejected with the correct `TYPE_MONEY_LITERAL_*` code.
Commit: —

## Validation Plan

- Unit tests: `classify_literal` (m/M), the full `binary_result_type` Money table,
  `money_raw_from_decimal` bounds (if landed here).
- Syntaxcheck fixtures (valid + invalid): typing, every lattice pairing, comparison
  restriction, literal range.
- Doc sync: `src/docs/spec/diagnostics/01_rule-codes.md` (three new rules). The §4.x
  `types` prose lands in plan-29-G.
- Acceptance: `scripts/test-accept.sh …` — no golden drift.

## Open Decisions

- **Literal suffix — DECIDED: `m`/`M` = Money**, mirroring `f`=Float / `F`=Fixed
  (user-confirmed). `1.25m` types as Money. (§4.4)
- **`M / M` result — DECIDED: `Float`** (a dimensionless ratio: margins, growth; use
  `toInt(M / M)` for a count). (§4.2) **Rationale (document in §4.1 prose, plan-29-G):**
  a ratio of two amounts is *dimensionless and inherently non-terminating*
  (`100.00m / 3.00m` has no exact finite value in **any** type, base-10 or base-2), so
  there is no "exact dimensionless type" to return — the only choice is *which* inexact
  type. Float wins because it is what the result feeds into (margins, growth,
  percentages, statistics — all Float math) and it has the dynamic range for large/tiny
  ratios; returning Fixed would falsely signal exactness and clip range. Money's
  exactness exists for *auditable cent accounting*; a ratio is no longer money and is
  not audited to the cent, so the guarantee does not apply. State this in §4.1 rather
  than merely asserting `Float`.
- **`M DIV k → Float` — CONFIRMED (2026-07-11), stays Float.** `DIV` is defined
  language-wide as fractional division that always returns `Float` for *every* valid
  operand pair (`mfb man types numeric`), so the operator itself is the explicit,
  visible escape into Float — writing `DIV` against a Money is a deliberate dimension
  exit, exactly parallel to calling `toFloat`. Rejecting it only for Money would make
  Money the one numeric where `DIV` behaves differently. Document the "`DIV` is the
  explicit Float escape" framing in the §4.1 prose (plan-29-G doc sweep).
- **Comparison strictness — DECIDED: Money-vs-Money only** for both `=`/`<>` and
  ordering (dimensional discipline; `money = 5` is a compile error — use `money =
  toMoney(5)`). (§4.3)
- **Bare-literal typing is context-independent — DECIDED.** A suffixless decimal literal
  is **always `Float`** (`classify_literal`, `src/numeric.rs:34`) regardless of
  surrounding operands — the Money context does **not** re-steer it to Fixed (a
  context-dependent *type* would be a worse long-term footgun than the inexactness it
  avoids). The exactness nudge for `Money * <bare-float-literal>` is therefore a
  **diagnostic-only** warning (it never changes the type), specified as an optional,
  deferrable phase in plan-29-F. (relates to §4.4)

## Summary

Money becomes a first-class *dimensioned* numeric: same-dimension add, scalar scaling,
ratio division, and hard compile-time rejection of every dimensionally-nonsensical
pairing. The engineering risk is entirely in the rejection rules (lattice + comparison);
type identity and literals mirror `Fixed`. Nothing here touches codegen, `.mfp`, or
existing goldens.
