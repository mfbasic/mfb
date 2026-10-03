# bug-790: distinct compiler diagnostic rules share numeric codes

Last updated: 2026-10-02
Effort: medium (estimate)
Severity: MEDIUM
Class: Correctness
Status: Open
Regression Test: proposed `src/rules/mod.rs:tests::rule_codes_are_unique`

Every registered compiler diagnostic must have a unique numeric code. Currently,
separate rules share codes, so tooling that identifies diagnostics by code cannot
distinguish them. Name-based lookup does not repair the ambiguous public identity.
This report records the defect; it does not renumber rules or change compiler code.

References:

- `src/docs/spec/diagnostics/01_rule-codes.md`, Code Collisions.
- User clarification during the diagnostics specification review: the same code
  assigned to different errors is a bug.
- `bugs/completed/bug-40-import-package-invalid-undefined-rule.md` calls the
  metadata/DOC collisions harmless. That historical assessment is incorrect for
  code-based consumers and must not be used to exempt them from uniqueness.

## Failing Reproduction

Run from the repository root:

```sh
python3 - <<'PY'
from collections import defaultdict
from pathlib import Path
import re
codes = defaultdict(list)
for code, name in re.findall(r'code: "([^"]+)",\s*name: "([^"]+)"', Path('src/rules/table.rs').read_text()):
    codes[code].append(name)
duplicates = {code: names for code, names in codes.items() if len(names) > 1}
assert not duplicates, duplicates
PY
```

Observed: exit 1, with this assertion payload:

```text
{'1-102-0010': ['MFB_PARSE_BLOCK_TOO_DEEP', 'MFB_PARSE_TESTING_EXPECTED_TGROUP'], '2-205-0001': ['PACKAGE_VERSION_UNSUPPORTED', 'DOC_UNRESOLVED'], '2-205-0002': ['NATIVE_MANIFEST_INVALID', 'DOC_NAME_MISMATCH']}
```

Expected: exit 0 and no duplicate codes. This reproduces directly from the
platform-independent registry; no target-specific compilation is necessary.
The same duplicate labels are visible in `./target/release/mfb spec diagnostics
rule-codes`. Unique rule names are a working contrast, already checked by
`src/rules/mod.rs:tests::rule_names_are_unique`.

## Root Cause

`src/rules/table.rs:RULES` assigns identical `Rule.code` values to distinct names.
`src/rules/mod.rs:rule_for` searches by name, so lookup does not detect those
collisions. `show_diagnostic` and `show_general_diagnostic` print the stored code
without checking uniqueness. `tests::rule_names_are_unique` checks names only;
`tests::every_rule_is_documented_in_the_spec` checks documentation presence, not
code uniqueness. These checks allow the ambiguous registry to pass.

## Goal

- Every entry in `RULES` has a unique name and a unique code, including entries
  without a live emitter.
- A regression test rejects any future duplicate code without an allowlist.
- Rendered diagnostics and the spec agree on the corrected assignments.

### Non-goals

Do not change diagnostic triggers, severities, messages, symbolic names, or the
runtime `errorCode` registry. Do not hide a collision by dropping a valid rule,
exempting dormant entries, or changing consumers to require a name alongside
an ambiguous numeric code.

## Blast Radius

Audit command:

```sh
rg -n 'MFB_PARSE_BLOCK_TOO_DEEP|MFB_PARSE_TESTING_EXPECTED_TGROUP|PACKAGE_VERSION_UNSUPPORTED|DOC_UNRESOLVED|NATIVE_MANIFEST_INVALID|DOC_NAME_MISMATCH' src tests
```

- `src/rules/table.rs:RULES`: all reproduced pairs are in scope.
- `src/rules/mod.rs:rule_for`, `show_diagnostic`, `show_general_diagnostic`:
  consume the registry; lookup and formatting algorithms need not change.
- `src/ast/parser.rs`, `src/ast/stmt.rs`, `src/ast/testing.rs`,
  `src/ast/doc_items.rs`, `src/resolver/resolution.rs`, `src/cli/fmt.rs`:
  name-based emitters; emitted numeric labels follow the changed registry.
- `PACKAGE_VERSION_UNSUPPORTED` and `NATIVE_MANIFEST_INVALID`: the audit finds
  their Rust names only in the registry. They still participate in uniqueness;
  absence of a current emitter does not authorize code reuse.
- `tests/syntax/testing/testing-parse-invalid/golden/build.log`,
  `tests/syntax/doc/doc-block-invalid/golden/build.log`, and the parser depth
  fixture logs: inspect for changes after selecting which assignments to retain.
- `tests/cli/cli_fmt_nesting_depth.rs`: inspect code/name expectations.
- `src/docs/spec/diagnostics/01_rule-codes.md`: update codes and remove the
  current-collision description when the implementation is corrected.
- `src/docs/spec/language/22_test-framework.md`,
  `src/docs/spec/tooling/05_fmt.md`, and man references returned by the audit:
  inspect for numeric-code dependencies; symbolic-name-only mentions are unaffected.
- `src/codegen/builtins/errorcode/mod.rs:register`: separate runtime registry,
  unaffected by compiler-rule renumbering.

## Fix Design

Add a registry-wide uniqueness test before editing assignments. For each pair,
use `git log -S` and blame to establish allocation order and consumers, retain
one existing assignment, and give the other rule a verified unused code in the
appropriate subsystem. Audit all six names together; fixing only the parser
pair leaves the same defect in DOC/package metadata.

Numeric-label changes are intentional compatibility changes. Preserve the
symbolic diagnostic identity and document the exact reassignment in the fix
commit. Do not select replacement numbers without checking the current table.

## Phases

### Phase 1 — failing test and compatibility audit

- [ ] Add `rule_codes_are_unique` beside `rule_names_are_unique`; prove it fails
      on the reproduced collisions.
- [ ] Establish allocation history and all live numeric-code consumers for each
      pair; select the assignments to retain and unused replacements.

Acceptance: the test fails on the current registry and the compatibility impact
of each reassignment is documented.
Commit: —

### Phase 2 — registry correction

- [ ] Correct every colliding assignment and update the owning spec table.
- [ ] Verify name lookup, severity, message, and emit conditions remain unchanged.

Acceptance: the uniqueness test passes with no exemptions; rendered diagnostics
use the corrected labels.
Commit: —

### Phase 3 — expected-output proof and validation

- [ ] Before editing any golden, establish its history, protected behavior,
      consumers, and independent proof of the wrong code; run the full suite
      before changing expected output. Change only the disproved label lines.
- [ ] Run `cargo test`, `cargo build`, and
      `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- [ ] Build release, rerun the reproduction, render diagnostics, and run
      `scripts/spec-census.sh --citations diagnostics`, `--links diagnostics`,
      and `--render diagnostics`.

Acceptance: the original reproduction passes, registry and rendered output agree,
and full-suite/acceptance results pass with only intended numeric-label changes.
Commit: —

## Validation Plan

Use the registry-wide test to cover dormant as well as emitted rules. Use the
existing parser/testing/DOC fixtures and CLI formatting test to verify observable
headers. The correctness condition is unique codes, not merely successful lookup
by symbolic name or a green documentation-presence test.
