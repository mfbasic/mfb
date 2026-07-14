# plan-41-E: Scalar primitive — docs, spec & acceptance

Last updated: 2026-07-13
Effort: small
Depends on: plan-41-A, plan-41-B, plan-41-C, plan-41-D

Bring the docs, embedded spec, and man pages into sync with the shipped `Scalar`
primitive, and run the full acceptance suite with regenerated goldens. After this
sub-plan `mfb spec language types` documents `Scalar`, `mfb man` covers the new
conversions, and the entire feature is green end to end. This closes plan-41.

References (read first):

- The `.ai/specifications.md` and man-template rules (`.ai/man_template.md`,
  `.ai/man_type_template.md`, `.ai/man_package_template.md`) — follow them
  exactly.
- plan-41-A..D for the exact behavior to document (delimiter, defaults, wire id,
  storage size, conversions, strings seam).

## 1. Goal

- `mfb spec language types` §4.1 lists `Scalar` (32-bit Unicode scalar,
  register-carried, backtick literal) and §4.11 lists it as comparable + orderable
  but not numeric; the defaults table (§4.10) shows default U+0000.
- The lexical-structure and grammar specs document the backtick scalar literal and
  its escapes, and explicitly note it does **not** disturb `'` comments.
- The package spec (`04_type-table.md`, `05_constant-pool.md`) shows
  `10 = Scalar`, ids `11–19` reserved for future primitives,
  `FIRST_TABLE_TYPE_ID = 20`, and the 4-byte const-pool payload; the memory spec
  (`01_scalar-storage.md`) shows `Scalar` = 4 bytes.
- `mfb man` has pages for `toScalar`, `strings::toScalars`, `strings::fromScalars`,
  the five scalar predicates (`strings::isLetter`, `isDigit`, `isWhitespace`,
  `isUpper`, `isLower`), and updated `toString`/`toInt`/`toByte`/`toScalar`
  overload notes (including `toByte(Scalar)` and `toScalar(Byte)`); a new dedicated
  `types/string.md` man page documents **both** `String` and `Scalar` (they are
  the text pair — a `String` is a sequence of Unicode scalars), keeping `Scalar`
  out of the numeric type page.
- The full acceptance/golden suite passes with `Scalar` coverage included.

### Non-goals (explicit constraints)

- **No behavior change.** This sub-plan only documents and validates what
  plan-41-A..D shipped; if a doc can't be written truthfully, the bug is in the
  implementation, not the doc — fix it there, don't paper over it.
- **Numeric tables stay numeric.** Do not add `Scalar` to the promotion/algebra
  tables in the docs (it is non-numeric); document it in the comparable/orderable
  and defaults tables instead. It likely warrants its own short man topic rather
  than an entry in `types/numeric.md`.

## 2. Current State

Spec sources: `src/docs/spec/language/04_types.md` (primitives :13, Money :10/27,
promotion :56-77, defaults :404-416, comparability :420, orderability :424,
summary tables :428-429 — each with `[[src/...]]` citations that must stay
accurate); `02_lexical-structure.md:4` (comments), `:34-36` (suffixes);
`19_grammar.md` (literal grammar); `24_type-name-encoding.md`;
`src/docs/spec/package/04_type-table.md:21-35`, `05_constant-pool.md:30,43`;
`src/docs/spec/memory/01_scalar-storage.md:9,13`.

Man sources: `src/docs/man/types/` currently holds `numeric.md`, `comparisons.md`,
`list.md`, `logical.md`, `map.md`, `package.md`, `pair.md`, `partition.md` — there
is **no** `String` type page today. `Scalar` does NOT belong in `numeric.md`;
instead author a new `types/string.md` covering both `String` and `Scalar`.
Also touch `comparisons.md`/`package.md`;
`src/docs/man/builtins/general/{toByte,toMoney,toFixed,toString}.txt` +
`package.md` (add `toScalar.txt`, update `toByte`/`toString`); strings man dir for
`toScalars`/`fromScalars` and the five `isLetter`/`isDigit`/`isWhitespace`/
`isUpper`/`isLower` pages (plus the strings `package.md` overview). Driver
scripts: `scripts/update_man.sh`,
`scripts/update_man_package.sh` (authoring rules live there).

## 3. Design Overview

Pure documentation + validation. Update each spec/man source to match the shipped
behavior, keeping every `[[src/...]]` citation pointing at the real symbol.
Author the new man pages from the templates. Then run the full acceptance suite
and sync goldens.

**Risk is drift, not logic**: a doc that describes intended-but-unshipped behavior
is worse than none. Every claim here must be checked against a plan-41-A..D test
or a live `mfb`/`mfb spec`/`mfb man` invocation.

## Compatibility / Format Impact

None (docs only). The spec numbers (`Scalar = 10`, reserved band `11–19`,
`FIRST_TABLE_TYPE_ID = 20`, 4-byte storage) merely record what plan-41-B/C
already shipped.

## Phases

### Phase 1 — Spec sync

- [ ] Update `src/docs/spec/language/04_types.md`: add `Scalar` to the §4.1
      primitives table, §4.10 defaults (U+0000), and §4.11 comparable +
      orderable tables (and prose noting it is orderable-but-not-numeric); do NOT
      add it to the promotion tables. Keep the `[[src/...]]` citations accurate.
- [ ] Update `02_lexical-structure.md` (backtick scalar literal, its escapes, and
      the note that `'` comments are unchanged), `19_grammar.md` (literal
      grammar), and `24_type-name-encoding.md` (the `Scalar` spelling).
- [ ] Update `src/docs/spec/package/04_type-table.md` (`10 = Scalar`, ids `11–19`
      reserved, `FIRST_TABLE_TYPE_ID = 20`), `05_constant-pool.md` (4-byte
      payload), and `src/docs/spec/memory/01_scalar-storage.md` (`Scalar` = 4
      bytes).
- [ ] Verify: `mfb spec language types` and `mfb spec package` render the updated
      content with no broken citation.

Acceptance: `mfb spec` renders `Scalar` correctly across the types, lexical,
grammar, package, and memory topics; citations resolve.
Commit: —

### Phase 2 — Man pages + full acceptance (highest-risk last)

- [ ] Author `src/docs/man/builtins/general/toScalar.txt` and update
      `toString.txt`/`toInt`/`toByte` overload notes — including `toByte(Scalar)`
      and `toScalar(Byte)` (via `scripts/update_man.sh`); author a new
      `src/docs/man/types/string.md` covering **both** `String` and `Scalar` (not
      under `numeric.md`); add `strings::toScalars`/`fromScalars` and the five
      `strings::isLetter`/`isDigit`/`isWhitespace`/`isUpper`/`isLower` man pages;
      update the relevant `package.md` overviews (via
      `scripts/update_man_package.sh`).
- [ ] Run the full acceptance suite; sync/regenerate goldens (including any from
      plan-41-B's wire renumber and plan-41-C/D's new programs).
- [ ] Verify: `mfb man builtins general toScalar`, `mfb man` for the strings
      functions (`toScalars`/`fromScalars` and the five predicates), and the
      `Scalar` type page render per template.

Acceptance: `mfb man` renders every new/updated page per the templates; the full
acceptance/golden suite is green tree-wide with `Scalar` coverage.
Commit: —

## Validation Plan

- Tests: the full repo acceptance suite (`scripts/test-accept.sh` or the project's
  standard command) plus the plan-41-A..D unit/runtime tests, all green.
- Runtime proof: `mfb spec` and `mfb man` invocations render the new content;
  the plan-41-D string-walk program remains green under the full suite.
- Doc sync: this sub-plan *is* the doc sync — spec (language/package/memory) and
  man pages all updated and rendering.
- Acceptance: full acceptance/golden suite green; man/spec render checks pass;
  `scripts/artifact-gate.sh` clean.

## Open Decisions

_Resolved 2026-07-13 (user)._

- **Scalar man home — DECIDED: a new dedicated `types/string.md` page documenting
  both `String` and `Scalar`.** (Was: `types/scalar.md` vs. an entry in
  `types/numeric.md`.) `Scalar` is explicitly non-numeric, so grouping it with the
  numeric primitives would mislead; pairing it with `String` on one page reflects
  that a `String` is a sequence of Unicode scalars. There is no `String` type page
  today, so this also fills that gap. (§2)

## Summary

No logic — only truthful documentation of the shipped `Scalar` and a full-suite
green. The only risk is doc drift, mitigated by verifying every claim against a
test or a live `mfb` invocation. When this lands, archive all of plan-41-A..E to
`planning/old-plans/`.
