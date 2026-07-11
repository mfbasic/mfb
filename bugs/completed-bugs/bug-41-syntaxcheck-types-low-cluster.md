# bug-41: `syntaxcheck/types.rs` LOW cluster — bare-name User-type unification, `Map OF K TO V` split on first ` TO `, and Byte-literal RECOVER range check can't parse radix literals

Last updated: 2026-07-08
Effort: small (<1h)

Three LOW-severity type-checking defects in `src/syntaxcheck/types.rs`. Batched
(same file, all LOW). Distinct root causes noted separately.

**(1) `compatible` unifies any two `User` types sharing a bare name (latent
soundness gap).** `compatible` (`:146-168`) reduces both type names with
`rsplit('.').next()` and returns `true` on `expected_bare == actual_bare` (`:156`).
The intent is to equate an imported type's qualified reference (`binding.Db`) with
its bare registration (`Db`), but the condition is **not** gated on a shared origin,
so it also unifies genuinely distinct types that merely share a final path segment —
e.g. an imported `geo::Point` (`User("geo.Point")`) and a local `TYPE Point`
(`User("Point")`) with different fields. The checker then accepts assigning one into
the other's slot despite different layouts.

**(2) `parse_type` splits `Map OF K TO V` on the first ` TO ` (latent mis-parse).**
`parse_type` (`:49-56`) does `rest.split_once(" TO ")` — leftmost separator, no
nesting awareness — so a key type that itself contains ` TO ` (e.g.
`Map OF Map OF String TO Integer TO Boolean`) parses to key
`"Map OF Map OF String"` / value `"Integer TO Boolean"` instead of key
`Map OF String TO Integer` / value `Boolean`. Same paren/nesting-unaware
type-string-parsing class as bug-35 and bug-26. (`type_name`, `mod.rs:~1916`, also
serializes `Map` values without parenthesizing.)

**(3) Byte-literal RECOVER range check parses the raw lexeme with `u16` (spurious
rejection).** `expression_compatible` (`:196-199`) validates a `Byte`-typed
`Expression::Number` with `value.parse::<u16>()` on the **un-canonicalized** lexeme.
For a radix/separator Byte literal in an inline-TRAP `RECOVER` — `RECOVER 0xFF` or
`RECOVER 2_00` against a `Byte` success type (the surviving consumer,
`checking.rs:320`) — `str::parse::<u16>()` can't handle `"0xFF"`/`"2_00"`, returns
`Err`, and `TYPE_RECOVER_TYPE_MISMATCH` fires even though `255`/`200` is an in-range
Byte.

The single correct behavior a fix produces: (1) only the *same* underlying type
declaration unifies across qualified/bare forms; (2) nested `TO`-bearing key/value
types parse correctly; (3) radix/separator Byte literals are canonicalized before
the `<= u8::MAX` range test.

Severity LOW for all three (two latent, one a narrow spurious rejection).

References:

- `src/syntaxcheck/types.rs:146-168` (`compatible`, bare-name equality at `:156`),
  `:49-56` (`parse_type` `Map OF` split), `:196-199` (`expression_compatible` Byte
  arm, `value.parse::<u16>()`).
- `src/syntaxcheck/checking.rs:320` (the surviving `expression_compatible` consumer,
  Byte-typed `RECOVER`).
- `src/syntaxcheck/mod.rs:~1916` (`type_name` serializes `Map` values unparenthesized).
- Same type-string-parsing class: bug-35 (`monomorph/helpers.rs`), bug-26
  (`builtins/general.rs`).
- KNOWN (not re-filed): FE-05 (bare Float literal → inf) in checking.rs.
- Found during goal-01 review of `src/syntaxcheck/{mod,checking,types}.rs`.

## Failing Reproduction

(1) Import `geo` (exports `TYPE Point{lat,lng}`) and declare a local `TYPE Point{x}`;
assign a local `Point` into a `geo::Point` slot → accepted (should be a type error).
(2) A type `Map OF Map OF String TO Integer TO Boolean` → structurally wrong `Type`.
(3) `RECOVER 0xFF` against a `Byte` success type → spurious
`TYPE_RECOVER_TYPE_MISMATCH`.

- Observed: (1) unrelated types unify; (2) key/value boundary mis-split; (3) valid
  radix Byte literal rejected.
- Expected: (1) rejected unless same declaration; (2) correct nesting; (3) accepted.

Contrast: (1) different simple names are rejected correctly; (2) nested *value*
`Map OF String TO Map OF …` parses correctly; (3) `RECOVER 200` (decimal) works.

## Root Cause

(1) Bare-name equality not gated on shared origin. (2) Leftmost ` TO ` split with no
nesting awareness. (3) Range check parses the raw lexeme instead of
`numeric::classify_literal`'s canonical decimal.

## Goal

- (1) Only the same registered `TypeInfo` unifies across qualified/bare forms.
- (2) `parse_type` splits `Map` on the top-level ` TO ` (paren-aware) and `type_name`
  parenthesizes `TO`-bearing sub-types.
- (3) `expression_compatible` canonicalizes via `numeric::classify_literal` before
  the `<= u8::MAX` check.

### Non-goals (must NOT change)

- The legitimate qualified==bare equivalence for the *same* imported type.
- Decimal Byte-literal RECOVER (works today); nested-value `Map` parsing (works).

## Blast Radius

- `compatible`, `parse_type`, `expression_compatible` (+ `type_name` serialization).
  Reconcile (2) with the shared type-string-parsing fix (bug-35/bug-26).

## Fix Design

(1) Resolve both names through `type_infos`/`canonical_import_name` to one key and
only unify when they map to the same declaration. (2) Parenthesize `TO`-bearing
key/value sub-types in `type_name` (as `thread_type_argument_name` does) and do a
paren-aware split in `parse_type`. (3) `numeric::classify_literal(value)` → canonical
decimal → `parse::<u16>()` bound check.

## Phases

### Phase 1 — failing test + audit

- [x] Tests: (1) distinct same-final-segment `User` types are rejected; (2)
      `TO`-bearing map key round-trips; (3) `RECOVER 0xFF` on a Byte type is accepted.
      Confirmed (1) and (2) fail before the fix; (3) already passes (see Resolution).
- [x] Code confirmation complete (above).

### Phase 2 — the fix

- [x] Apply the fixes: (1) `compatible` bare-name gate on same registered
      `TypeInfo`; (2) nesting-aware `Map OF K TO V` split; (3) no code change needed
      (already correct — see Resolution).

### Phase 3 — validation

- [x] Full unit suite green (`cargo test --bin mfb`, 2430 passed); new tests fail
      before / pass after; common + nested-value map and decimal/radix Byte
      programs build unchanged. `scripts/test-accept.sh` deferred to the
      orchestrator (no golden shift expected — see Resolution).

## Validation Plan

- Regression test(s): the three tests above.
- Full suite: `scripts/test-accept.sh`.

## Summary

Three small type-checker defects in `types.rs`: an over-broad bare-name User-type
unification, a nesting-unaware `Map` split (bug-35/26 class), and a raw-lexeme Byte
range check; each fix is local and preserves valid-program behavior.

## Resolution

Fixed in `src/syntaxcheck/types.rs` (+ tests in the same file). All work is
contained to that one file; no diagnostic rule was added/renamed, so the error-code
registry is untouched.

**(1) Bare-name `User` unification — FIXED (soundness gate).** The `compatible`
`User/User` arm no longer unifies on `expected_bare == actual_bare` unconditionally.
It now returns `true` for a shared bare name only when both names resolve to the
*same* registered `TypeInfo` (`std::ptr::eq`), or when either side is unregistered
(a built-in `User` type such as `net.Url`, or a template parameter) where the shared
bare name is authoritative. The union-variant arm is preserved. This matches Goal (1)
verbatim ("only the same registered `TypeInfo` unifies across qualified/bare forms").
Note: in the *current* data model `type_infos` is keyed only by bare names (local
`TYPE` decls and imported exports alike), so two distinct names never resolve to two
distinct entries in the natural pipeline — the gate is therefore a no-op for programs
that compile today and closes the gap the moment any dotted key can exist. The
deeper "imported types are bare-keyed, colliding with a local same-name type"
limitation is a separate, pre-existing data-model issue out of this file's scope.
Covered by the white-box unit test `bare_name_user_types_need_same_declaration`
(fails before, passes after).

**(2) `Map OF K TO V` split — FIXED.** `parse_type` now splits the map body with a
new `split_map_body` helper that scans at paren depth 0 and skips the ` TO ` owned by
each nested `Map`/`MapEntry`/`Thread`/`ThreadWorker` sub-type (each owns exactly one),
so a `TO`-bearing key (`Map OF Map OF String TO Integer TO Boolean`) parses to
`Map(Map(String, Integer), Boolean)` instead of the leftmost mis-split. Handles
unparenthesized nested-map keys (what the parser actually emits — it adds no parens),
parenthesized/grouped keys, `FUNC(...) AS R` keys, and `RES`-marked values; returns
`None` on a body with no top-level ` TO ` (caller falls through, matching the old
behavior). `type_name` was deliberately **not** changed to parenthesize (the balanced
split needs no parens, avoiding golden churn and a broader blast radius). Covered by
`split_map_body_handles_nested_key_and_value` and
`parse_type_nested_map_key_structure` (both fail before, pass after).

- End-to-end note: in a full `mfb build` the *resolver* (`src/resolver/resolution.rs`
  ~:1240) rejects a nested-`TO` map key first with its own leftmost `split_once(" TO ")`
  — the same bug-35/bug-26 string-parsing class in a file outside this bug's scope.
  This fix corrects the syntaxcheck layer (`check_src`, which does not run the
  resolver); making nested-`TO` map keys compile end-to-end additionally needs the
  resolver and monomorph splitters fixed (bug-35). No observable end-to-end contract
  changed here (such keys are still rejected, now by the resolver only).

**(3) Byte-literal `RECOVER` range check — NOT A BUG (no code change).** The premise
(that `expression_compatible` sees the "un-canonicalized lexeme") does not hold: the
**lexer** already decodes radix prefixes to decimal and strips `_` separators, so
`Expression::Number` carries `"255"` for `0xFF` and `"200"` for `2_00` before it
reaches the `<= u8::MAX` check — `value.parse::<u16>()` succeeds. Verified with `mfb
build`: `RECOVER 0xFF`, `RECOVER 0b1111_1111`, `RECOVER 2_00` are accepted, and
`RECOVER 0x100`(256)/`RECOVER 300` are correctly rejected. Moreover
`numeric::classify_literal` does **not** decode radix (it only strips an `f`/`F`
suffix and detects `.`/`e`), so the proposed canonicalization would not fix a
hypothetical raw-hex lexeme anyway. Added regression guards
`byte_recover_accepts_radix_and_separator_literals` and
`byte_recover_rejects_out_of_range_radix_literal` to lock in the correct behavior
against a future lexer change.

### Tests / commands

- `cargo build` — clean.
- `cargo test --bin mfb syntaxcheck::types` — 32 passed (5 new).
- `cargo test --bin mfb` — 2430 passed, 0 failed, 1 ignored.
- Revert-probe: with the fixes backed out, `parse_type_nested_map_key_structure` and
  `bare_name_user_types_need_same_declaration` FAIL (confirming fail-before).
