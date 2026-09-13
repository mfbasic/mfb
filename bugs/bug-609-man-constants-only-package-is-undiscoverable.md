# bug-609: `mfb man errorCode` lists none of its 52 constants, and its types page says "list its functions"

Last updated: 2026-09-13 (fixed)
Effort: small–medium
Severity: LOW
Class: Documentation (renderer)

Status: FIXED (5eaea6f15)
Regression Test: `src/cli/man.rs` tests — `a_constants_only_package_lists_every_constant_with_value_and_message`, `every_registered_constant_is_listed_on_its_package_page`, `a_record_constant_renders_as_a_qualified_record_construction`, `the_no_types_page_names_what_the_package_has`, `naming_a_constant_as_a_page_points_at_the_package_page`

`errorCode` is a constants-only package: `src/codegen/builtins/errorcode/mod.rs:register`
registers 52 `Err*` constants (`add_constant`), each with a value and a message,
and nothing else. The man renderer shows none of them:

```
mfb man errorCode --all              # the overview only — no constant names or values
mfb man errorCode ErrPathNotFound    # error: unknown errorCode function
mfb man errorCode types              # "The `errorCode` package has no public types.
                                     #  Run `mfb man errorCode` to list its functions."
```

The last line points to a page that, per its own overview, "exports no functions".
The overview routes to `mfb spec diagnostics error-codes` for the mapping, but a
developer at `mfb man` cannot see which names exist. The same gap hides
`vector`'s 42 record constants (`vector::zeroFloat3`, …), whose overview now spells
the naming rule out by hand (plan-125-B Phase 3).

**The single correct behavior a fix produces:** a package's registered constants
are listed on its `mfb man <pkg>` page (name, value, and description where
present), and the no-types fallback does not tell the reader to list functions
the package does not have.

Found by plan-125-B Phase 3's Codex review of `errorCode`
(`planning/plan-125-findings/B-phase3/man-pkg-errorCode.md`, findings 1–2).
**Filed, not fixed**, by user instruction during a documentation-only plan
("file all bugs, make no fixes").

## Reproduction

The three commands above, at `worktree-P-125` HEAD.

## Root cause

- `src/cli/man.rs` renders functions, records, unions, enums and resources, but
  has no section for `RegistryConstant` (`grep -n -i constant src/cli/man.rs`
  finds no rendering site).
- The no-types fallback at `src/cli/man.rs` (the `"has no public types.\n\nRun
  `mfb man {}` to list its functions."` string) is unconditional.

## Non-goals

- Changing constant values or names.
- Hand-maintaining a constants table in overview prose, which would drift from the
  registry.

## Blast-radius audit

- Every package that calls `add_constant`: `errorCode` (52), `vector` (42), and any
  other (`grep -rn add_constant src/codegen/builtins`) gains a section.
- Tests pinning the rendered `errorCode`/`vector` pages will shift.

## Fix

- [x] Phase 1 — renderer tests: `mfb man errorCode` lists `errorCode::ErrPathNotFound`
and its value; the types fallback for a package with no functions does not say
"list its functions" (RED). Commit: 5eaea6f15 (tests and fix landed together;
all five tests verified RED on the pre-fix renderer first; prep refactor 2eb775566)

- [x] Phase 2 — render a Constants section from the registry; word the fallback by what
the package has (GREEN); update proven-wrong pins. Commit: 5eaea6f15 (no pin needed
updating: no test or golden carried the rendered errorCode/vector page)

## STATUS: FIXED (5eaea6f15)

Deviations and additions beyond the doc:

- `mfb man errorCode ErrPathNotFound` still exits 2 (a constant has no page), but
  now says it is a constant and points at `mfb man errorCode`.
- A record constant's Value renders as `pkg::Type[c1, …]`; a probe program using
  `vector::Float3[0.0, 0.0, 0.0]`, `color::Color[0, 0, 0, 255]`,
  `errorCode::ErrPathNotFound = 77030001` and `math::pi` built and printed TRUE ×4.
- Found while fixing: the errorCode overview's example said `ErrPathNotFound` is
  `77020001` — that is `ErrReadFailed`; corrected to the registered `77030001`.
- math's overview carried a hand-maintained constants table (the non-goal's drift
  risk); it is replaced by a sentence naming the constants and pointing at the
  derived table.
- `scripts/man-census.sh --memory-scope` carve-out 2 now also classifies the
  errorCode Constants row that carries `ErrOutOfMemory`'s runtime message
  "Allocation failed."; whole surface: 0 unclassified, carve-out 2 40 → 41.
  `.ai/man-content.md` §0 lists Constants as derived and §4.4 documents this
  carve-out, which §9.2 already referenced.
- Fallout caught by the full suite: `tests/guards/no_type_strings.rs` (plan-111)
  flagged the first version's `constant_type_name(…, type_name: &str)` helper
  (`str_type_params / cli: 1 > budget 0`). The helper now takes the
  `RegistryConstant` (5372f1211); the guard is green with its budget unchanged.
