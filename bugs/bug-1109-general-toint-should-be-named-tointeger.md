# bug-1109: the general conversion `toInt` is the only `to*` builtin not named after its result type; it must be `toInteger`

Last updated: 2026-10-21
Effort: large (3h–1d) (mechanical, but large: ~8,100 occurrences, ~330 golden files)
Severity: LOW
Class: Other (language-surface naming inconsistency)

Status: Open
Regression Test: a new runtime fixture `tests/rt-behavior/general/to_integer_conversions/` (every
source type `toInteger` accepts) and a new rt-error fixture pinning that a bare `toInt(...)` call is
refused with `SYMBOL_UNKNOWN_IDENTIFIER`.

Every other general conversion builtin is named after the type it produces, spelled in full:
`toString`, `toFloat`, `toFixed`, `toByte`, `toMoney`, `toScalar`
(`src/codegen/builtins/general/mod.rs`: the `TO_*` constants). The `Integer` conversion is
abbreviated: `toInt` (`TO_INT`). The `big` package already spells the same conversion in full,
`big::toInteger` (`src/codegen/builtins/big/func_to_integer.rs`), so the abbreviation also
disagrees with the stdlib. The spec writes the inconsistency down as a rule:
`src/docs/spec/language/18_builtin-functions.md` says "The conversion name is `toInt` (not
`toInteger`)."

**The single correct behavior a fix produces:** the bare general builtin that converts a value to
an `Integer` is named `toInteger`, with exactly the overloads, error contract, and behavior `toInt`
has today; the name `toInt` is no longer a builtin, so a call to it fails with
`SYMBOL_UNKNOWN_IDENTIFIER` like any other unknown name. **No backwards compatibility**: no alias,
no deprecation period, no "did you mean" special case for `toInt`. (Owner decision, 2026-10-21.)

References:

- `src/docs/spec/language/18_builtin-functions.md` (the "conversion name is `toInt`" rule that flips)
- `src/codegen/builtins/general/func_to_int.rs` (the `mfb man` descriptor prose)
- `src/docs/spec/stdlib/19_big.md` (`big::toInteger`, the stdlib spelling this matches)
- Filed at the owner's request, 2026-10-21.

## Failing Reproduction

```
mfb init /tmp/toint_repro
# src/main.mfb:
IMPORT io

FUNC main() AS Integer
  io::print(toString(toInteger("42") + toInteger(1.9)))
  RETURN 0
END FUNC

mfb build /tmp/toint_repro
```

- Observed (`target/release/mfb build .`, 2026-10-21, macos-aarch64):
  ```
  ./src/main.mfb:4 error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: identifier could not be resolved
                 Callable `toInteger` is not a top-level function.
  ```
  (reported twice, once per call)
- Expected: builds; the program prints `43`.

Contrast (works today): the same program with `toInteger` replaced by `toInt` builds and prints
`43`, exit 0 (same command). After the fix the two cases swap: `toInteger` builds and prints `43`,
and `toInt` is refused with `SYMBOL_UNKNOWN_IDENTIFIER`.

Not platform-dependent: the name is resolved before any backend runs.

## Root Cause

Naming only; there is no defect in the conversion itself. The name is declared once,
`src/codegen/builtins/general/mod.rs:TO_INT` (`"toInt"`), and registered through
`src/codegen/builtins/general/func_to_int.rs:register`. But the string `"toInt"` is also
hard-coded outside the registry, so changing the constant alone is not enough. Sites found with
`grep -rn '"toInt"' src`:

- `src/codegen/builtins/mod.rs` (`if target == "toInt"`)
- `src/target/shared/validate/body.rs` (`|| target == "toInt"`)
- `src/target/shared/validate/mod.rs` (`target: "toInt".to_string()`)
- `src/target/shared/runtime/usage.rs` (`| "toInt"`)
- `src/codegen/memory/data/data_objects/static_names.rs` (`"len" | "toInt" =>`)
- `src/codegen/memory/data/data_objects/mod.rs` (`("toInt", &["String"])`)
- `src/codegen/memory/value/builder_value_semantics/static_eval.rs` (`"find" | "len" | "toInt"`)
- unit tests: `src/codegen/builtins/tests/{general_declared_errors,infallible_proof_coverage,unit}.rs`,
  `src/ir/tests/variant_corpus/mod.rs`, `src/fmt/tests/mod.rs`

Any of these left behind would quietly turn off a lowering, validation, or static-eval path for the
renamed builtin. That is the main risk in this change.

## Goal

- `toInteger(x)` resolves for every source type `toInt` accepts today (`String` with optional
  `base`, `Float`, `Fixed`, `Money`, `Byte`, `Scalar`, `Color`, any `ENUM`), with identical results
  and identical declared errors (`ErrInvalidFormat`, `ErrOverflow` per overload).
- `toInt(x)` is refused with `SYMBOL_UNKNOWN_IDENTIFIER`.
- A user `FUNC toInteger(m AS Cash)` overrides the builtin exactly as `FUNC toInt(m AS Cash)` does
  today (`tests/rt-behavior/functions/func_override_toint_user`,
  `func_override_toint_enum_precedence`); a user `FUNC toInt` becomes an ordinary user function.
- `grep -rnw toInt src packages examples tests benchmark tools` finds nothing except historical
  `bugs/` and `planning/` docs and the rt-error fixture that pins the refusal.
- `mfb man toInteger` renders the descriptor; `mfb man toInt` no longer finds a builtin; the spec rule
  reads "The conversion name is `toInteger`".

### Non-goals (must NOT change)

- Conversion semantics: truncation toward zero, `base` 2–36, enum ordinal, overflow and format
  errors, Money whole units. Generated code apart from symbol and name strings must be byte-for-byte
  the same.
- `big::toInteger` and every other `to*` builtin.
- No alias, shim, deprecation warning, or fallback lookup for `toInt` (owner decision).
- Internal Rust identifiers (`func_to_int`, `TO_INT`, `emit_*_to_int`, runtime helper symbols) may be
  renamed for consistency, but a rename must not change emitted code except where the symbol name
  appears in it.
- Don't re-baseline goldens wholesale to get green. Every golden delta must be explained by the
  rename alone (`toInt` → `toInteger` in IR/AST text, and any checksum that hashes those names).
  An unexplained delta is a bug-hunt trigger (AGENTS.md).

## Blast Radius

Counts from `grep -rIlw toInt <dir>` / `grep -rIow toInt <dir> | wc -l`, excluding `target`,
`.claude`, `.git`:

| Area | files | occurrences | Verdict |
| --- | ---: | ---: | --- |
| `src/` | 229 | 817 | fixed: registry, hard-coded sites above, descriptor prose (most hits are example programs embedded in other builtins' `mfb man` descriptors, e.g. `crypto`, `json`, `encoding`, `canvas`, `color`), spec + man guide topics |
| `packages/` | 39 | 135 | fixed (shipped package source) |
| `examples/` | 25 | 68 | fixed |
| `benchmark/` | 15 | 37 | fixed; benchmark `baseline/` outputs should not change (verify) |
| `tools/` | 16 | 26 | fixed |
| `tests/` | 460 | 7,324 | fixed: fixture sources + goldens. 332 golden files mention `toInt` (`grep -rIlw toInt tests \| grep golden`): 265 `.ir`, 55 `.ast`, 6 `.log`, 2 `.nir`, 2 `.nplan`, 1 `.testrun`. `.ncodesum` files may also shift if names reach the hash (guess; confirm with the artifact gate) |
| `repository/` | 0 | 0 | unaffected |
| `bugs/`, `planning/` (incl. `completed/`) | 198 | 611 | unaffected: historical record, left as written |

- `big::toInteger` — unaffected: package-qualified, so it can't collide with the bare builtin
  (`tests/acceptance/src/big.mfb` and `benchmark/language/mfb/src/bigint.mfb` already use it).
- No `.mfb` file in the tree calls a bare `toInteger` today (`grep -rInE
  '(^|[^:a-zA-Z_])toInteger\b' --include='*.mfb'`, excluding `big::`; the only hit is a TCASE label
  string), so the new name shadows no existing user function.
- `src/resolver/resolution/body.rs` reaches general builtins through `is_general_call`, so it follows
  the registry automatically. Verify it, don't edit it.

## Fix Design

1. Rename the builtin at its single declaration (`TO_INT` → `TO_INTEGER = "toInteger"`, file
   `func_to_int.rs` → `func_to_integer.rs`) and rewrite its descriptor prose.
2. Replace every hard-coded `"toInt"` string literal in `src/` (list above) with the constant where
   it's in scope, otherwise with `"toInteger"`. Putting these sites on the constant is the durable
   fix, so the next rename can't miss one.
3. Mechanical whole-word rewrite `\btoInt\b` → `toInteger` across `.mfb` sources, embedded example
   programs, docs, and tests. Exclude `bugs/`, `planning/`, and the new refusal fixture. Rename the
   two `func_override_toint_*` fixture directories to `func_override_tointeger_*`.
4. Regenerate goldens through the project's golden tooling, then diff-audit them: the only allowed
   textual change is the identifier.

Rejected: keeping `toInt` as an alias. The owner ruled out backwards compatibility, and two names for
one builtin is the inconsistency this bug removes.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [ ] Add `tests/rt-behavior/general/to_integer_conversions/` covering every overload. It fails
      today with `SYMBOL_UNKNOWN_IDENTIFIER`.
- [ ] Add an rt-error fixture asserting bare `toInt("1")` is refused with
      `SYMBOL_UNKNOWN_IDENTIFIER`. It fails today because the call builds.
- [ ] Re-run the `"toInt"` literal sweep and record any site missing from Root Cause.

Acceptance: both new tests fail for the documented reason; the literal list is complete.
Commit: —

### Phase 2 — the rename

- [ ] Registry constant, descriptor file and prose, hard-coded literals (Fix Design 1–2).
- [ ] Source rewrite across `src`, `packages`, `examples`, `benchmark`, `tools`, `tests` fixtures
      (Fix Design 3); rename the override fixtures.
- [ ] Spec: `18_builtin-functions.md` rule flips to `toInteger`; update other spec/man topics that
      mention `toInt` (`src/docs/spec/**`, `src/docs/man/**`).

Acceptance: Phase 1 tests pass; `cargo test` unit tests green; Goal grep finds nothing.
Commit: —

### Phase 3 — regenerate goldens + validation

- [ ] Regenerate `.ir` / `.ast` / `.nir` / `.nplan` / `.log` / `.testrun` (and any `.ncodesum`) goldens.
      Diff-audit: every hunk is the identifier rename only.
- [ ] Run `scripts/artifact-gate.sh <mfb> all` and a full `scripts/test-accept.sh` (AGENTS.md:
      required before re-baselining).
- [ ] Re-run the Failing Reproduction: `toInteger` prints `43`; `toInt` is refused.
- [ ] `mfb man toInteger` renders; `mfb spec` topic renders the flipped rule.

Acceptance: all gates green; golden deltas are exactly the rename; reproduction passes.
Commit: —

## Validation Plan

- Regression tests: the two Phase 1 fixtures.
- Runtime proof: the Failing Reproduction project builds and prints `43`.
- Doc sync: `src/docs/spec/language/18_builtin-functions.md`, `func_to_integer.rs` descriptor, man
  guide topics under `src/docs/man/**` that use `toInt`.
- Landing gate: the project landing tier, plus `scripts/artifact-gate.sh <mfb> all` and
  `scripts/test-accept.sh`, because hundreds of goldens shift.
- Full tier: the scheduled background run. Cross-target goldens (`linux-*`, `windows-*`) are
  covered by the artifact gate.

## Open Decisions

- Rename internal Rust identifiers (`func_to_int`, `TO_INT`, helper names) too? Recommended: yes for
  the registry file and constant (they name the builtin), no for unrelated helper symbols whose
  rename would churn native-code goldens for no user-visible gain.

## Summary

Risk is in two places: missing one of the hard-coded `"toInt"` strings outside the registry (which
would silently drop a lowering or validation path), and the ~330-file golden regeneration, which has
to be audited down to identifier-only hunks. Conversion semantics and `big::toInteger` stay as they
are.
