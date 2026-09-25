# bug-680: a `UNION` template parameter is unusable — no variant may name a template instantiation

Last updated: 2026-09-22
Effort: medium (1h–2h)
Severity: LOW
Class: Footgun

Status: Fixed
Regression Test: `tests/syntax/monomorph/union-template-variant-valid` (new),
plus a rejection case for the genuinely-undeclared spelling

`./mfb spec language templates` §3 states that "Template parameters may appear
only on `TYPE`, `UNION`, `FUNC`, and `SUB` declarations", and the parser honours
the `UNION` half: `UNION Opt OF T ... END UNION` parses and builds today. But a
union *variant* is parsed as a bare (optionally package-qualified) type name
with no ` OF …` arguments, so no variant can name `Some OF T` — or any other
template instantiation, including a fully concrete one like `Box OF Integer`.

The two halves do not meet. `T` is accepted on the declaration and is then
structurally unreachable from every variant, because a variant's payload is the
*only* place a union puts a type. A `UNION … OF T` is therefore always one of
two things: a union whose parameter is dead, or a compile error — never a
working generic union. The compiler says neither; it accepts the dead form
silently, with no `unused template parameter` diagnostic.

**The single correct behavior a fix produces:** a union variant is parsed with
the same type-name grammar every other type position uses, so
`UNION Opt OF T { Some OF T, None }` declares a union whose variant payload
carries `T`, and `Opt OF Facing` monomorphizes to a union over `Some OF Facing`
and `None`. If instead the decision is that unions do not take template
parameters, then `UNION Opt OF T` must be *rejected* at the declaration and §3
corrected — what must not persist is the current state where it is accepted and
inert.

This is a footgun, not a correctness bug: nothing miscomputes, and no shipped
code is affected (no `.mfb` in the tree declares a union template — see Blast
Radius). The cost is that a documented feature cannot be used, and the failure
arrives as a parse error pointing at the variant rather than at the declaration
that is actually unsupported.

References:

- `./mfb spec language templates` §3 — the sentence that admits `UNION` to the
  template-parameter list
- `./mfb spec language types` §4.3 — unions as closed sums over named concrete
  member types
- `./mfb spec architecture monomorphization` — the unification/instantiation
  algorithm the fix must stay inside
- Found while discussing how `examples/dungeon`'s `keyDirection` should spell
  "this key is not a movement key" without a 0-or-1 `List OF Facing`; the
  question of whether a user could write the optional shape themselves is what
  surfaced this.

## Failing Reproduction

```
mkdir -p /tmp/bug680/src
cat > /tmp/bug680/project.json <<'EOF'
{"name":"bug680","version":"0.1.0","mfb":"1.0","kind":"executable",
 "sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],
 "entry":"main","targets":["native"]}
EOF
cat > /tmp/bug680/src/main.mfb <<'EOF'
TYPE Some OF T
  value AS T
END TYPE
TYPE None
END TYPE

UNION Opt OF T
  Some OF T
  None
END UNION

FUNC main AS Integer
  RETURN 0
END FUNC
EOF
./target/release/mfb build /tmp/bug680
```

- Observed: two parse errors on the variant line, neither of which names the
  real limitation:

  ```
  /tmp/bug680/src/main.mfb:8 error[1-102-0005 MFB_PARSE_UNEXPECTED_TOKEN]: parser found an unexpected token
                 Expected end of statement after union member type.
    8 |   Some OF T
      |        ^^
  /tmp/bug680/src/main.mfb:8 error[1-102-0005 MFB_PARSE_UNEXPECTED_TOKEN]: parser found an unexpected token
                 Expected end of statement after union member type.
    8 |   Some OF T
      |           ^
  ```

- Expected: the declaration is accepted and `Opt OF Facing` instantiates to a
  union over `Some OF Facing` and `None`.

Contrast cases, all measured at `5e58663d9`:

| Case | Spelling | Result |
| --- | --- | --- |
| Template param on a `UNION`, variants non-generic | `UNION Opt OF T { A, B }` | builds ✓ (and `T` is silently dead) |
| Template param on a `TYPE`, generic field | `TYPE Stack OF T { items AS List OF T }` | builds ✓ |
| Union variant is a *concrete* instantiation | `UNION Opt { Box OF Integer, B }` | same parse error ✗ |
| Union variant is a plain declared type | `UNION Shape { Circle, Rect }` | builds ✓ |

The third row is the one that localizes the bug: the failure has nothing to do
with the parameter `T` being in scope. The variant grammar rejects ` OF ` even
when every argument is concrete, so this is a missing parser case, not a
monomorphization limit.

## Root Cause

`Parser::parse_union_variant` (`src/ast/items.rs:416`) reads a variant as

```rust
let name = self.parse_qualified_name("Union member type must be a type name.")?;
let name = self.normalize_qualified_type_name(name);
self.consume_statement_end("Expected end of statement after union member type.");
```

`parse_qualified_name` accepts `Ident` and `pkg::Ident` and stops. It is the
shared helper for function and constant references, so it deliberately knows
nothing about ` OF …`. The template-argument grammar lives in
`parse_type_name` (`src/ast/expr.rs:922`), which every other type position
calls — including the sibling record-field parser twenty lines above
(`src/ast/items.rs:373`, `let type_name = self.parse_type_name()?;`). The
variant parser is the one type position that does not, so the `OF` token is
still unconsumed when `consume_statement_end` runs and reports it.

The AST then has nowhere to put the arguments even if they were parsed:
`ast::types::UnionVariant` (`src/ast/types.rs:449`) is `{ name: String, line:
usize }`, while `TypeDecl.template_params` (`src/ast/types.rs:425`) is shared
across `Type`/`Union`/`Enum` and so accepts `OF T` on a `UNION` without
complaint. That asymmetry — parameter stored, argument unstorable — is the bug
in one line.

Two findings that bound the fix:

- **The back end is already prepared.** `Lowerer::lower_type_decl`
  (`src/monomorph/lower.rs:621`) already maps every variant through
  `self.concrete_type(&variant.type_, substitutions)`, i.e. it already
  substitutes template arguments into union variants. `HirUnionVariant` carries
  a `type_`, not a bare name. Nothing downstream needs teaching.
- **Argument inference from a constructor does not cover unions.**
  `src/monomorph/lower.rs:1635` reads `TypeDeclKind::Union => Vec::new()`, so a
  union template gets an empty field list to unify against and can infer nothing
  from constructor arguments. In practice the variant (`Some[x]`) is a `TYPE`
  template and infers normally, with the union type supplied by the expected
  type — but a direct `Opt[...]`-shaped inference has no path. This is where the
  correctness risk sits, and Phase 1 must pin down which inference sites are in
  scope before Phase 2 touches the parser.

## Goal

- `UNION Opt OF T` with a variant `Some OF T` parses, monomorphizes on use, and
  matches with `CASE Some(s)` / `CASE None`.
- A union variant naming a concrete instantiation (`Box OF Integer`) parses on
  the same path.
- The spelling that is genuinely undeclared still fails, with a diagnostic that
  names the undeclared type rather than the `OF` token.
- Either §3's `UNION` claim becomes true, or §3 is corrected and
  `UNION … OF T` is rejected at the declaration line. Doc and implementation
  agree at the end of this bug.

### Non-goals (must NOT change)

- **No `Option`/`Maybe` ships.** This bug makes a documented template feature
  usable; it does not add a built-in optional type, and it does not revisit
  `./mfb spec language types` §4.4 ("There is no built-in Option/Maybe"). That a
  user could now *write* `UNION Opt OF T` in their own package is a consequence
  of §3, not a change of stance, and no such type enters the built-in packages.
- `UNION … INCLUDES …` semantics, member-name-conflict rules, and the rule that
  a variant names an already-declared concrete type.
- Enum ordinals, `toInt`/`toString`, defaultability (§4.10) and comparability
  (§4.11) — untouched.
- The `.mfp` package format and its type table. Templates are monomorphized
  before binary representation (§3), so only concrete instantiations are ever
  encoded; if the fix needs a wire-format change, that is a signal the fix is
  wrong.
- **Tempting wrong fix, forbidden:** "fixing" this by deleting `UNION` from §3's
  template-parameter list *without also rejecting* `UNION … OF T` at the parser.
  That leaves the silent dead-parameter form — the actual footgun — in place
  while making the document merely describe it.

## Blast Radius

Searched with `grep -rn "UNION .* OF " --include="*.mfb"` across the tree
(excluding `target/`) and by reading every caller of `parse_union_variant`.

- `src/ast/items.rs:parse_union_variant` — the defect; fixed by this bug.
- `src/ast/types.rs:UnionVariant` — needs a home for the arguments (or a
  rendered type string, matching how `TypeField.type_name` already carries
  `List OF T`); fixed by this bug.
- `src/monomorph/lower.rs:lower_type_decl` (line 621) — already substitutes into
  variants; unaffected, and its existing behavior is the reason the fix is
  parser-shaped.
- `src/monomorph/lower.rs:1635` (`TypeDeclKind::Union => Vec::new()`) — latent
  gap in constructor-driven argument inference for union templates. In scope to
  *characterize* in Phase 1; in scope to fix only if the Phase 1 tests show a
  reachable inference site, otherwise recorded here and left.
- `src/codegen/registry/mod.rs:689` (`UnionVariant { name, description }`) — the
  built-in-companion registry's own variant shape, used to render `UNION`
  declarations for built-in packages such as `json`. No built-in companion
  declares a template union; unaffected, but it is a second variant
  representation and must not silently drift from the AST one.
- No `.mfb` anywhere in the tree (examples, tests, packages, benchmark, tools)
  declares a `UNION … OF …`. Zero shipped code depends on either the current
  rejection or the proposed acceptance, so there is no golden churn to expect
  beyond the new tests.

## Fix Design

Make `parse_union_variant` call `parse_type_name` — the same helper the record
field parser calls — and widen `UnionVariant` to carry the rendered type string
instead of a bare name, mirroring `TypeField.type_name` (which already carries
`List OF T` as text and defers to `ParameterType::parse`). Keeping one type
grammar is the existing house rule, stated in the comment at
`src/ast/items.rs:362`: "the one type grammar stays the only parser of it."

The correctness risk is *not* in the parser change — it is in whether resolution
and monomorphization treat a variant type string identically to a field type
string on every path that currently assumes a variant is a bare declared name:
member-name-conflict detection, `INCLUDES` merging, `MATCH` pattern resolution
(`HirMatchPattern::Union { type_, binding }`, `src/monomorph/lower.rs:1298`),
and exhaustiveness. Phase 1 exists to enumerate those paths from the code before
Phase 2 edits the parser.

Rejected alternatives:

- *Teach `parse_qualified_name` about `OF`.* Rejected: it is shared with
  function and constant references, where ` OF ` is not a type argument list.
- *Add a bespoke variant-only template-argument parser.* Rejected: two type
  grammars, which is the thing the existing comment forbids.
- *Delete `UNION` from §3 and reject the parameter.* This is a legitimate
  outcome and stays on the table as an Open Decision — but it is strictly more
  spec churn for strictly less capability, and the back end already supports the
  substitution, so it should be chosen deliberately rather than by default.


### Phase 1 findings (measured on the worktree build, probes under `/tmp/b680probe/`)

All seven new fixtures fail at HEAD with the documented
`MFB_PARSE_UNEXPECTED_TOKEN … after union member type` on the variant line
(`target/release/mfb build tests/syntax/monomorph/<fixture>` with the pre-fix
binary).

Verdict per path that assumed a variant is a bare declared name:

- **`INCLUDES` merging — same defect, fixed.** `parse_union_includes` also used
  `parse_qualified_name`, so `UNION Outcome OF T INCLUDES Opt OF T` failed to
  parse at the header. Now reads `parse_type_name`; `hir::elaborate_type_decl`
  classifies includes (and variants) against the declaration's template params
  exactly as fields are (`parse_type`). `lower_type` already substituted
  includes. Fixture `union-template-includes-valid`.
- **`MATCH` union-pattern resolution — broken, fixed.** `CASE Some(s)` lowered
  its pattern through `concrete_type(Named("Some"))`, which stays the bare
  template name, and the post-monomorph resolve reported `SYMBOL_UNKNOWN_TYPE:
  Type `Some``. New `Monomorphizer::union_pattern_type` types the scrutinee and
  takes the one instantiation of that template the concrete union (and its
  `INCLUDES`) carries.
- **Member-name conflicts — new rule.** Because the CASE names a member by its
  template, a union carrying two instantiations of one template could never be
  matched. Declared directly: `TYPE_DUPLICATE_VARIANT` in
  `resolve_type_decl` (fixture `union-template-duplicate-instantiation-invalid`).
  Reached through `INCLUDES`: `TYPE_MATCH_PATTERN_MISMATCH` at the CASE
  (`union-template-case-ambiguous-invalid`). A CASE naming a template the union
  does not carry: `TYPE_MATCH_PATTERN_MISMATCH` in source spellings
  (`union-template-case-not-member-invalid`), instead of the unknown-type
  misreport.
- **Exhaustiveness — correct as-is.** Runs on the concrete union
  (`TYPE_MATCH_NOT_EXHAUSTIVE … does not cover None` for a missing arm). Its
  message quotes the mangled `Opt$Integer`; that is a pre-existing,
  compiler-wide property of every post-monomorph diagnostic (a plain
  `TYPE Box OF T` misuse reports `Box$Integer` at HEAD), not introduced here.
- **`DOC` `PROP` member names — broken, fixed.** `validate_doc_block` built the
  union's member set from the rendered type (`Some OF T`), so `PROP Some` was
  `DOC_PROP_UNKNOWN`. A template member is now named by its template.
  Covered in `union-template-variant-valid`.
- **`codegen/registry` `UnionVariant` — unaffected.** Built-in companions only;
  none declares a template union.

Inference gap (`TypeDeclKind::Union => Vec::new()`): **not reachable as
inference** — a union is never constructed by its own name (`Shape[42]` is
`TYPE_CONSTRUCTOR_REQUIRES_RECORD`), so there is nothing to infer. But the
template spelling `Opt[42]` reached the post-monomorph resolve as the bare
template and was misreported as `SYMBOL_UNKNOWN_TYPE`; it now reports
`TYPE_CONSTRUCTOR_REQUIRES_RECORD` at that site
(`union-template-constructor-invalid`).

## STATUS: FIXED (30c52e601)

Landed as the recommended "fix" outcome: §3's `UNION` claim is now true.
Deviations from the plan, all found by the Phase 1 audit and fixed in the
same commit:

- `INCLUDES` had the same parser defect and now uses `parse_type_name` too.
- `MATCH` needed a monomorph step (`Monomorphizer::union_pattern_type`), not
  just the parser change: a bare `CASE Some(s)` otherwise reached the
  post-monomorph resolve as an unknown type.
- New rule: a union may carry at most one instantiation of a template
  (`TYPE_DUPLICATE_VARIANT` at the declaration; `TYPE_MATCH_PATTERN_MISMATCH`
  at a CASE when it arises through `INCLUDES`).
- `DOC` `PROP` member names and the `Opt[x]` constructor diagnostic fixed.
- Eight fixtures instead of two (`tests/syntax/monomorph/union-*`); the golden
  delta is exactly those eight plus one unrelated line: the full gate was red
  at HEAD on `app-window-surface`'s linux-x86_64 app ncodesum, which no
  committed compiler ever produced (35ab73a30; main landed the identical
  correction independently).

Not fixed here (pre-existing, compiler-wide): post-monomorph diagnostics quote
mangled names (`Opt$Integer`, and `Box$Integer` for a plain `TYPE Box OF T`).

Verification: `cargo test --no-fail-fast` — 227 `test result: ok`, 0 failed;
the reproduction above builds.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/syntax/monomorph/union-template-variant-valid` following the
      layout of `tests/syntax/monomorph/monomorph_instantiation_fanout_bounded`
      (`project.json`, `src/`, `golden/`): a `UNION Opt OF T` over `Some OF T`
      and `None`, instantiated at two distinct argument types, matched with
      `CASE Some(s)` / `CASE None`. Confirm it fails today with the two
      `MFB_PARSE_UNEXPECTED_TOKEN` errors quoted above.
- [x] Add the concrete-argument case (`UNION Opt { Box OF Integer, B }`) as a
      second fixture — it fails today for the same reason and proves the fix is
      not parameter-specific.
- [x] Audit and write the verdict into this file for each path that assumes a
      variant is a bare declared name: member-name-conflict detection,
      `INCLUDES` merging, `MATCH` union-pattern resolution, exhaustiveness, and
      `src/codegen/registry/mod.rs:UnionVariant`.
- [x] Characterize the `TypeDeclKind::Union => Vec::new()` inference gap
      (`src/monomorph/lower.rs:1635`): write the program that would need it and
      record whether it is reachable.

Acceptance: both new fixtures fail for the documented parse reason; every
audited path has a written verdict; the inference gap is reachable-or-not with
the probe recorded.
Commit: 30c52e601 (fixtures; audit recorded in this doc)

### Phase 2 — the fix

- [x] Widen `ast::types::UnionVariant` to carry a rendered type string
      (`src/ast/types.rs:449`).
- [x] Point `parse_union_variant` at `parse_type_name`
      (`src/ast/items.rs:416`), keeping `normalize_qualified_type_name` on the
      head of the path.
- [x] Fix any in-scope site the Phase 1 audit flagged.
- [x] Confirm the undeclared-type diagnostic now names the type, not the `OF`
      token.

Acceptance: Phase 1 fixtures pass; the four contrast rows in the reproduction
table still behave as documented; nothing in Non-goals changed.
Commit: 30c52e601

### Phase 3 — spec sync + full validation

- [x] Update `./mfb spec language types` §4.3 if the accepted variant grammar is
      now wider than "names concrete `TYPE` declarations that already exist".
- [x] Regenerate only the goldens the two new fixtures introduce; confirm no
      pre-existing golden moves (no tree `.mfb` declares a union template, so
      the expected delta is exactly the new fixtures).
- [x] Run the full suite.
- [x] Re-run the reproduction above end to end.

Acceptance: full suite green; golden delta is exactly the two new fixtures;
spec and implementation agree.
Commit: d92260aca (spec), 35ab73a30 (unrelated stale golden found by the full gate)

## Validation Plan

- Regression tests: `tests/syntax/monomorph/union-template-variant-valid` and
  the concrete-argument sibling.
- Runtime proof: the reproduction program, extended to construct both
  instantiations and `MATCH` them, printing a distinguishable line per variant.
- Doc sync: `./mfb spec language types` §4.3 (variant grammar); §3 stays as
  written if the fix lands, and is corrected instead if the Open Decision goes
  the other way.
- Full suite: the project's standard acceptance command.

## Open Decisions

- **Fix or reject?** Recommended: fix — make §3 true, since
  `lower_type_decl` already substitutes into variants and the change is confined
  to the parser plus one AST field. Alternative: reject `UNION … OF T` at the
  declaration and strike `UNION` from §3's list. Either resolves the bug; the
  current silent-dead-parameter state resolves nothing. (§3, §4.3)
- **Variant representation.** Recommended: a rendered type string, matching
  `TypeField.type_name`. Alternative: a structured `{ name, args }`, which is
  cleaner but diverges from how every other type position is stored.

## Summary

The engineering risk is not the parser edit — it is the Phase 1 audit of the
paths that currently assume a union variant is a bare declared name (`INCLUDES`
merging, member-name conflicts, `MATCH` pattern resolution, exhaustiveness). The
monomorphizer already substitutes into variants, and no `.mfb` in the tree
declares a union template, so the blast radius is a parser case and one AST
field, with zero expected movement in existing goldens. Enum semantics, the
`.mfp` format, and the language's no-built-in-`Option` stance are untouched.
