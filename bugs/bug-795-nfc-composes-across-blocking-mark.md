# bug-795: Runtime NFC composes Hangul across an intervening combining mark

Last updated: 2026-10-02
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness
Status: Open
Regression Test: proposed `tests/rt-behavior/strings/nfc-blocked-composition-rt`

The emitted normalization routine can compose a starter with a later class-zero
scalar across a combining mark. Constant folding preserves the blocked sequence.
The correct behavior is that runtime and folded NFC both preserve
`U+1100 U+0301 U+1161`; composition must respect intervening combining classes.

## Failing Reproduction

Create an executable project with the following `src/main.mfb`:

```basic
IMPORT io
IMPORT strings
FUNC nfc(s AS String) AS String
  RETURN strings::normalizeNfc(s)
END FUNC
FUNC main AS Integer
  io::print(strings::normalizeNfc("\u{1100}\u{301}\u{1161}"))
  io::print(nfc("\u{1100}\u{301}\u{1161}"))
  RETURN 0
END FUNC
```

`project.json`:

```json
{"name":"unicode_spec_probe","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
```

Run from the repository root:

```sh
./target/release/mfb build /tmp/mfb-unicode-spec-probe
/tmp/mfb-unicode-spec-probe/build/unicode_spec_probe.out
```

Observed on macOS AArch64 at the default optimization level:

- Folded: `U+1100 U+0301 U+1161` (correct).
- Runtime: `U+AC00 U+0301` (incorrect).
- Expected: both lines contain `U+1100 U+0301 U+1161`.

The probe was initially run with each result wrapped in
`toString(strings::toBytes(...))`, which round-trips its UTF-8 bytes and prints
these same strings. Adjacent Hangul composition and ordinary accent composition
are contrast cases to retain in the regression test.

## Root Cause

`src/codegen/builtins/strings/func_normalize_nfc.rs:lower_out` branches directly
to `compose_try` when the **current** combining class (`scratch15`) is zero,
without testing the remembered intervening class (`scratch28`). After U+0301,
that remembered class is 230. U+1161 is class zero, so
`src/codegen/string/unicode_props.rs:emit_hangul_composition_attempt` composes it
with U+1100 despite the intervening mark.

The folded implementation, `src/unicode/backend.rs:normalize_nfc`, delegates to
`UnicodeNormalization::nfc` and does not take that branch.

Search used to check existing reports, including archives:
`rg -l -i 'hangul.{0,50}block|block.{0,50}hangul|compose_try' bugs planning --glob '*.md'`
(no pre-existing report matched).

## Goal and Non-goals

Runtime NFC must apply composition blocking to class-zero candidates as well as
nonzero candidates. Do not disable normalization, force all calls through constant
folding, change Unicode versions, or weaken existing fixtures. Preserve ordinary
Hangul LV/LVT composition, canonical ordering, public signatures and memory layout.

## Blast Radius

Search: `rg -n 'normalize_nfc::lower_out|normalize_nfc::lower|emit_hangul_composition_attempt' src`.

- `func_normalize_nfc.rs:lower` / `lower_out`: affected copying normalization.
- `string_self_update.rs:try_inplace_string_rewrite_assign`: affected non-ASCII
  self-update through the same `lower_out` scratch destination.
- Attributed-string normalization: affected through its String normalization call.
- All native targets consume the same emitter; only macOS AArch64 was executed.
- `unicode/backend.rs:normalize_nfc`: unaffected folded oracle.
- Case mapping and grapheme segmentation: unaffected; no composition pass.

## Fix Design

Attempt composition when the remembered blocking class is zero or strictly less
than the current class. Preserve the starter bookkeeping and table/Hangul paths.
Test both Hangul L+V and LV+T across intervening marks, plus any class-zero
composition pairs in the table. The risky boundary is class-zero composition
versus the selection of a new starter, not allocation or table layout.

## Phases

### Phase 1 — failing test and audit

- [ ] Add dynamic, literal-folded and self-update regression cases; prove the
      dynamic cases fail before the fix.
- [ ] Audit every `lower_out` caller and class-zero composition pair.

Acceptance: regression fails with the observed scalar mismatch.
Commit: —

### Phase 2 — correction

- [ ] Correct composition eligibility in `lower_out`.
- [ ] Verify blocked and unblocked Hangul and non-Hangul contrast cases.

Acceptance: runtime, self-update and folded outputs agree with the normalization oracle.
Commit: —

### Phase 3 — validation and documentation

- [ ] Run the full suite before changing any golden; inspect each expected delta.
- [ ] Run the repository's compiler/artifact/runtime gates on affected targets.
- [ ] Remove the current-behavior limitation from the Unicode spec once corrected.

Acceptance: full validation passes and the reproduction returns identical correct lines.
Commit: —

## Validation Plan

Use `unicode_normalization::UnicodeNormalization::nfc` as the independent runtime
oracle; keep inputs dynamic and verify generated programs execute the emitter.
Run the full cargo test suite and the compiler gates from `.ai/compiler.md` and
`.ai/testing-gates.md`. Update `src/docs/spec/unicode/01_tables-and-algorithms.md`
with the corrected blocking behavior. No implementation or test change is part
of this filing.
