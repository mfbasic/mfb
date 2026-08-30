# plan-111-D: type codegen's scalar-semantics cluster

Last updated: 2026-08-29
Effort: large (3h–1d)
Depends on: plan-111-C (the type tables and the registry answer typed queries, so
a converted emitter has something typed to call).

The first of three mechanical codegen letters. This one takes the **scalar
semantics** cluster — arithmetic, conversion, string representation, money, SIMD,
error emission and entry/thunk — where the dominant pattern is a `match` on a
rendered scalar type name choosing an instruction sequence.

122 violation sites across 15 files (§2). Every one is the same edit: the
function takes `&ParameterType` instead of `&str`, and its `match` arms become
variant patterns. There is no design content in this letter.

See plan-111-A for the shared prerequisites, the five sanctioned boundaries, the
tiered gate policy, and the rejected alternatives.

References:

- `src/codegen/builtins/math/gen_math.rs` — 28 spelling arms, the single densest
  file in plan-111 and the cleanest instance of the pattern.
- `src/codegen/engine/convert/builder_conversions.rs` — 22 arms + 3 parses; the
  conversion matrix, where a wrong arm is a wrong *number*, not a crash.
- `src/codegen/engine/operators/builder_numeric.rs` — 19 arms + 2 compares +
  2 parses.
- `.ai/codegen-invariants.md` — register lifetimes and the desugars these
  emitters sit inside; `.ai/arch-abi.md` for the per-arch traps in the SIMD and
  entry files.
- `src/numeric.rs` — the single typed promotion algebra (plan-106-A); the
  numeric emitters should be consulting it, not re-deriving from spellings.

## Prerequisites

See plan-111-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-111-C complete | `rg -n '\b(resolve_call\|call_return_type\|argument_types)\(' src/ --glob '!**/tests*'` → definitions only; `string_keyed_type_maps` budget 0 | **MET** (2026-08-30, `38b69d72d`). Every C box ticked, every acceptance verified, spot-check 0 diffs on 4 builtins. `string_keyed_type_maps` has no budget row at all — the class is 0 tree-wide. The literal `rg` still shows `resolve_call`, but every live hit is the `#[cfg(test)]` spelling shim or a registration assertion (Correction C4); the production population is 0. |
| Scope re-measured at kickoff | the four census commands from plan-111-A §2, restricted to this letter's file list | **MET** (2026-08-30) — and re-measured with the gate's own `test_free_lines` stripper, not `rg`, per Correction C3. See the kickoff table below. |

### Kickoff re-measurement (2026-08-30)

`cargo test --test no_type_strings census_by_file -- --ignored --nocapture` — a
per-FILE dump added to the gate in this letter's first commit, so D/E/F can each
scope themselves without repeating the `rg` over-count that Corrections A3 and
C3 both record.

| File (under `src/codegen/`) | plan §2 | live | delta |
|---|---|---|---|
| `builtins/math/gen_math.rs` | 28 | 28 | — |
| `engine/convert/builder_conversions.rs` | 25 | 25 | — |
| `engine/operators/builder_numeric.rs` | 23 | 23 | — |
| `string/repr/builder_strings.rs` | 12 | 12 | — |
| `builtins/vector/builder_vector_inline.rs` | 8 | 8 | — |
| `builtins/money/gen_money_math.rs` | 6 | 6 | — |
| `builtins/astrings/gen_astrings.rs` | 3 | 3 | — |
| `builtins/vector/builder_simd_math.rs` | 3 | 3 | — |
| `error/emission/builder_error_emission.rs` | 3 | 3 | — |
| `engine/function/entry.rs` | 3 | **4** | **+1** |
| `builtins/vector/builder_simd_float_math.rs` | 2 | 2 | — |
| `builtins/vector/builder_simd_fixed_math.rs` | 2 | 2 | — |
| `link/thunk/link_thunk.rs` | 2 | **0** | **−2** |
| `builtins/math/gen_pow.rs` | 1 | 1 | — |
| `builtins/money/func_get_rounding.rs` | 1 | 1 | — |
| **Total** | **122** | **121** | **−1** |

Two deltas, both explained and both corrected in the phase bodies below
(Correction D1). Unlike letters A and C, the `rg`-based §2 table is very nearly
right here — this letter's classes are match arms and compares, which barely
occur in test modules, so the blind spot that cost A and C a correction each
does not bite. **Thirteen of fifteen files match exactly.**

## 1. Goal

- Every file in §2's list takes and matches `ParameterType`. Zero `&str` type
  parameters, zero spelling match arms, zero spelling compares, zero
  `ParameterType::parse` calls in any of them.
- The gate budgets for these 15 files read 0 across all six needle classes.

### Non-goals (explicit constraints)

- See plan-111-A §1 Non-goals — all apply. In particular: **no behavior change.**
  A converted arm must select the identical instruction sequence for the
  identical input type.
- Do not consolidate emitters, merge arms, or "simplify" a dispatch while
  converting it. If two arms look redundant, leave them redundant; a merged arm
  that behaves differently on one type is the exact failure this plan cannot
  have.
- Do not change `src/numeric.rs`'s promotion algebra. If an emitter re-derives
  promotion instead of calling it, record that in Corrections as a finding — do
  not fix it here.
- Do not touch the collection or memory files (letters E and F).

## 2. Current State

These emitters receive a type as a rendered name — historically from
`TypeModel`/registry lookups, which letter C has now made typed — and dispatch on
it with a string `match`. After C, the callers hold a `ParameterType` and render
it purely to satisfy these signatures.

### Measured populations

At HEAD (`fd09ea809`), 2026-08-29, tests excluded. `a` = spelling match arms,
`e` = spelling `==`/`!=`, `p` = `&str` type parameters, `parse` =
`ParameterType::parse` sites. Command: the four patterns from plan-111-A §2 run
per file.

| File (under `src/codegen/`) | a | e | p | parse | total |
|---|---|---|---|---|---|
| `builtins/math/gen_math.rs` | 28 | 0 | 0 | 0 | 28 |
| `engine/convert/builder_conversions.rs` | 22 | 0 | 0 | 3 | 25 |
| `engine/operators/builder_numeric.rs` | 19 | 2 | 0 | 2 | 23 |
| `string/repr/builder_strings.rs` | 11 | 0 | 1 | 0 | 12 |
| `builtins/vector/builder_vector_inline.rs` | 0 | 3 | 4 | 1 | 8 |
| `builtins/money/gen_money_math.rs` | 6 | 0 | 0 | 0 | 6 |
| `builtins/astrings/gen_astrings.rs` | 0 | 0 | 0 | 3 | 3 |
| `builtins/vector/builder_simd_math.rs` | 0 | 0 | 0 | 3 | 3 |
| `error/emission/builder_error_emission.rs` | 0 | 3 | 0 | 0 | 3 |
| `engine/function/entry.rs` | 0 | 3 | 0 | 0 | 3 |
| `builtins/vector/builder_simd_float_math.rs` | 0 | 0 | 0 | 2 | 2 |
| `builtins/vector/builder_simd_fixed_math.rs` | 0 | 0 | 0 | 2 | 2 |
| `link/thunk/link_thunk.rs` | 0 | 2 | 0 | 0 | 2 |
| `builtins/math/gen_pow.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/money/func_get_rounding.rs` | 0 | 0 | 0 | 1 | 1 |
| **Total** | **86** | **13** | **5** | **18** | **122** |

### Verified properties

- **These 15 files are disjoint from letters E and F.** The per-file census in
  plan-111-A's working notes covers all 71 codegen files with any violation;
  D (122) + E (161) + F (175) = 458 = codegen's measured total
  (147 arms + 59 compares + 143 params + 109 parses). No file appears twice and
  none is unassigned.
- **UNVERIFIED: how many of the 18 parses survive letter C.** Several are
  registry-adjacent and C may remove them as a side effect. Re-measure at
  kickoff; a lower number is C working, not a scope error.

## 3. Design Overview

One phase per coherent sub-cluster, ordered smallest-blast-radius first. There is
no design uncertainty in this letter — the uncertainty was spent in A and C.

Where correctness risk sits, in order:

1. **`builder_conversions.rs` (22 arms).** The conversion matrix decides which
   numeric conversion instruction runs. A misrouted arm produces a *wrong value*,
   not a crash — invisible to a compile-only check and invisible to byte-identity
   only if the wrong arm happens to emit the same bytes. The rt-behavior fixtures
   are the real guard here.
2. **The SIMD files.** Per-arch, and `.ai/arch-abi.md`'s traps apply; an x86-only
   miscompile is invisible on a Mac host except through `.ncodesum` — which this
   plan does not check until letter G. Phase 3 compensates by converting these
   files without touching their arch dispatch at all.
3. **`entry.rs` and `link_thunk.rs`.** ABI-adjacent; `mfb_return` ≡ `c_return` on
   ARM but differs on x86 (the audit-ABI-by-emission-path memory), so a mistake
   here is again host-invisible.

Per plan-111-A §3, the per-phase gate is `cargo test --no-fail-fast -- --skip artifact_gate_all` —
the `--skip` keeps the full cross-target artifact sweep out of the loop, since
`tests/golden.rs`'s only test shells out to `artifact-gate.sh all`. Goldens,
`test-accept.sh` and the artifact gate are swept **once, in letter G**.

For this letter specifically the `rt_*` runtime tests are the whole safety net —
they are what catches a wrong conversion arm producing a wrong *number*. Drop
`--no-fail-fast` and they are silently skipped, because they sort after
`golden.rs`. That would leave this letter with no correctness gate at all.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial with one line on what remains; mark
> moot tasks `- [x] ~~text~~ — moot: <evidence>`; fill `Commit:` when a phase
> lands. **An unticked box means NOT DONE.**

### Phase 0 — strengthen the gate before scoping anything against it (Correction D1)

Not in the original plan. Added because the kickoff re-measurement agreed with
§2 almost exactly, and reading one file against its own census showed why: the
gate could not see a tuple arm. Scoping D, E and F against a gate with a blind
spot would have shipped the blind spot into all three.

- [x] `spelling_match_arms`: scan the whole arm PATTERN, not just its head, so a
      tuple arm `("sin", "Float") =>` is a hit. Stop the pattern at ` if ` — a
      guard decides by comparing, which is class 4's needle.
- [x] `spelling_compares`: catch the wrapped form `== Some("Integer")`.
- [x] `TYPE_PARAM_NAMES`: sweep `src/` for `*type*: &str` and add every
      type-system name the hand-seeded list missed (10). Record the deliberate
      exclusions (`ctype`/`socktype`/`abi_return_ctype`, `type_code`) in the
      constant so they are not re-litigated.
- [x] Pin every new behavior in `scanners_fire_on_their_own_needles`, in BOTH
      directions — a tuple arm counts, a spelling right of the arrow does not, a
      guard compare counts once and in class 4, a constructed `Some("Integer")`
      is not a compare.
- [x] Add `census_by_file` — an `#[ignore]`d per-FILE census, plus
      `MFB_CENSUS_DETAIL=<substring>` to dump offending lines. This is the tool
      that avoids BOTH failure directions seen so far: `rg`'s over-count
      (Corrections A3, C3) and the gate's under-count (D1).
- [x] Re-measure `BUDGETS` wholesale against the strengthened scanners; tight in
      both directions.
- [x] Fix `src/ir/shape.rs:3005` `constructor_typed` — a letter-B site the old
      scanner could not see. `is_named` calls, identical decision, render dropped.
- [x] Add the three `optimizer` sites to letter G (they read the NIR `mov_imm`
      operand-class attribute whose producer is already a counted `target` row).
- [x] Note in letters E and F that their §2 tables need the same re-scope.

Acceptance: **MET.** `cargo test --test no_type_strings` → 4 passed, budgets
tight both ways. The census total moved 525 → 584: **59 sites that were live the
whole time and invisible to three letters of gating.**
Commit: c52478579

### Phase 1 — arithmetic and money (51 sites, no `&str` params)

Pure arm conversion; these two files take no type as `&str`, so the change is
local to their `match` bodies.

- [x] `builtins/math/gen_math.rs` — convert ~~28~~ **45** spelling arms to
      `ParameterType` variant patterns. Correction D1: the extra 17 are
      `(function, spelling)` tuple arms in the SIMD kernel selectors, which the
      pre-D1 scanner could not see. Two helpers stop rendering as well:
      `list_element_type` returns a `ParameterType` instead of a `String`, and
      `numeric_element_type_code` takes one — which is what let all 45 arms
      become variant patterns rather than pushing the render one frame out.
      Every error message is unchanged: `Display for ParameterType` is
      `write!(f, "{}", self.name())`, so `{other}` / `{element}` render exactly
      as before.
- [x] `builtins/money/gen_money_math.rs` — convert 6 arms.
- [x] `builtins/math/gen_pow.rs`, `builtins/money/func_get_rounding.rs` — delete
      the 1 parse each. `parse("List OF Float")` →
      `ListOf(Box::new(Float))`; `parse("Rounding")` → `named("Rounding")` (a
      literal builtin nominal, which Correction C1 explicitly keeps as `named`
      rather than `declared` — no lookup key is built from it).
- [x] Lower the gate budgets for these four files to 0. `codegen`
      `spelling_match_arms` 172 → 121, `parse_sites` 92 → 90.
- [x] Tests: no new tests — ~~the existing `rt_*` math/money fixtures cover
      these~~. **Correction D2**: they do not. There is exactly one `rt_*` file
      in the math/money area (`rt_native_numeric_pow_div_runtime.rs`); the real
      coverage is `tests/acceptance/src/math.mfb` and `money.mfb`, and the
      plan's own policy defers the acceptance corpus to letter G. Taken
      literally, Phase 1 would have had **no behavioral gate at all** — only
      "it compiles". So the acceptance project was run scoped, here:
      `scripts/test-accept.sh target/release/mfb /tmp/accept-111d acceptance`
      → **passed, 1 test ran**. That is read-only comparison, not regeneration,
      so it does not touch the deferred-goldens policy. Coverage confirmed by
      enumeration rather than assumed: every converted member appears —
      `abs acos asin atan atan2 ceil clamp cos e exp floor log log10 max min pow
      rand round seed sin sqrt tan` — across scalar, int-array, fixed-array and
      float-array cases.

Acceptance: **MET.** The four files read 0 on all seven needle classes (absent
from `census_by_file` entirely). `cargo test --no-fail-fast -- --skip
artifact_gate_all` → exit 0, 0 failures, 67 result lines, every `rt_*` included.
Scoped acceptance green.
Commit: 167e6b11c

### Phase 2 — conversion, numeric operators, string representation (60 sites)

The highest-value files in this letter, and the one real correctness risk.

- [x] `engine/convert/builder_conversions.rs` — convert 22 arms and delete 3
      parses. Convert **one arm per commit or in small named groups**; a 22-arm
      rewrite in one commit cannot be reviewed against the conversion matrix.
      Done as three named-group commits, each listing its matrix rows:
      **2a** `toInt` + `toScalar`, **2b** `toFloat` + `toFixed`,
      **2c** `toMoney` + the sign predicates. The three
      `parse("Scalar")` result types become `named("Scalar")` (a bare nominal —
      `ir/verify/values.rs` asks `is_named("Scalar")` — and no lookup key is
      built from it, so Correction C1's `declared` rule does not apply).
      Two near-duplicate arm pairs were deliberately NOT merged, per §Non-goals:
      `toMoney` Integer-vs-Byte (only Integer is overflow-checked) and the two
      Money rows of `toFloat`/`toFixed` (different helpers, different range
      check).
- [x] `engine/operators/builder_numeric.rs` — convert ~~19~~ **20** arms, 2
      compares, delete 2 parses (Correction D1: +1 tuple arm). Where an arm re-derives numeric promotion, call
      `src/numeric.rs`'s typed algebra instead of reimplementing it — but only
      where it is already equivalent; a behavior change here is out of scope and
      belongs in Corrections. **Both parses were a render/reparse round trip**:
      `promoted_binary_type(..).name().into_owned()` at the top and
      `ParameterType::parse(&result_type)` at the bottom, with a string match
      between them. `promoted_binary_type` has returned a `ParameterType` since
      plan-106-E, so both are simply deleted. The one arm that re-derives
      promotion is **not** equivalent to the algebra — see **Correction D3**;
      per this task's own condition it is reported, not changed.
- [x] `string/repr/builder_strings.rs` — convert 11 arms and 1 `&str` param.
      The parameter (`lower_list_replace`'s `element_type`) came from
      `typed_list_element_type(&value.type_).map(|t| t.name().into_owned())` and
      was then compared by spelling three times — so typing it **removed four
      renders and added one**, at the boundary with `builder_collection_layout`'s
      still-untyped `list_entry_stride` / `kind2_payload_size` /
      `inline_collection_payload_size`. Those three are letter E's (with
      `list_entry_stride`'s ~80 call sites); the single remaining render is
      annotated in place and E deletes it. The `toString` matrix also converts
      two arms the gate never counted — `"List OF Byte"` →
      `ListOf(element) if **element == Byte`, and `"AttributedString"` →
      `is_named(..)` — because neither spelling is in `SPELLINGS_MATCH`.
- [x] Lower the gate budgets for these three files to 0. `codegen`:
      `spelling_match_arms` 121 → 68, `parse_sites` 90 → 85,
      `spelling_compares` 60 → 58, `str_type_params` 166 → 165.
- [x] Tests: add an rt-behavior fixture for any conversion pair in the matrix
      that the existing corpus does not exercise — determine which by reading the
      matrix against the fixture list, not by assuming coverage.
      **Done by enumeration; the answer is that none is missing**, so no fixture
      was added. All 30 pairs are exercised in `tests/acceptance/src/`:

      | Conversion | Source types converted | Where exercised |
      |---|---|---|
      | `toInt` | Fixed, Float, Money, String | `primitives.mfb` "from text, base 10", "from text with an explicit radix", "from Byte, Float, Fixed, Money, Scalar" |
      | `toScalar` | Byte, Integer, String | `primitives.mfb` "from a code point, a one-scalar String, and a Byte" |
      | `toFloat` | Integer, Fixed, Money, String | `primitives.mfb` "from text, Integer, Fixed, Money" |
      | `toFixed` | Integer, Float, Money, String | `primitives.mfb` "from text, Integer, Float, Money" |
      | `toMoney` | Integer, Byte, Fixed, Float, String | `primitives.mfb` "from Integer, Float, Fixed, Byte, exact text" |
      | `toString` | String, Boolean, Byte, Scalar, Integer, Fixed, Float, Money | `primitives.mfb` "renders each primitive" + "Float/Fixed/Money default to 2 places" |
      | `toString` | **List OF Byte** | `general.mfb:189` "byte list decodes to text" |
      | `toString` | **AttributedString** | `astrings.mfb:141` `toString(astrings::fromString("hello"))` |

      The last two rows are the ones an assumption would have missed: they are
      the two `toString` arms the gate never counted (neither spelling is a
      `SPELLINGS_MATCH` needle), and neither is in the `primitives.mfb` group
      where the other eight live.

Acceptance: **MET.** All three files read 0 on all seven needle classes (absent
from `census_by_file`). `cargo test --no-fail-fast -- --skip artifact_gate_all`
→ exit 0, 0 failures. Scoped acceptance green (the corpus that actually
exercises the matrix — see Correction D2).
Commit: 9a2b8bec5, 2506bfad1, 5d8e8a1f8, 9bf446af3

### Phase 3 — SIMD, error emission, entry and thunk (28 sites, host-invisible risk)

Scheduled last: these are the files where a mistake does not show up on the host.

- [x] `builtins/vector/builder_vector_inline.rs` — 3 compares, 4 `&str` params,
      1 parse. The `VECTOR_SHAPES` table's **element** column becomes a
      `ParameterType` (only unit variants, so it stays `const`), which turns all
      three compares into variant compares. `vector_field_count` and
      `make_vector_native` take the type — deleting **five renders** at their
      callers in `function_lowering.rs` and `builder_values.rs`. The `cross`
      lane test stops reading the last character of a nominal's spelling
      (`type_name.ends_with('3')`) and reads `fields.len() == 3` instead; the two
      agree on all nine table rows by construction, since both columns come from
      the same row.
- [x] `builtins/vector/builder_simd_math.rs` (3 parses),
      `builder_simd_float_math.rs` (2), `builder_simd_fixed_math.rs` (2). All
      three SIMD entry points (`lower_simd_unary`/`binary`/`clamp`) took the
      result type as a spelling and re-parsed it at the end; their only callers
      are `gen_math.rs`, typed in Phase 1, so both sides convert together.
- [x] `builtins/astrings/gen_astrings.rs` — 3 parses.
- [x] `error/emission/builder_error_emission.rs` — 3 compares, **and the
      `block_type: &str` parameter** Correction D1 made visible.
- [x] `engine/function/entry.rs` — ~~3~~ **4** compares (kickoff re-measurement).
      Converted end to end rather than at the compare: `CodegenSpec`'s
      `language_entry_returns` field is now a `&ParameterType`, which removes the
      two `&entry.returns.name()` renders in `builder/mod.rs`.
- [x] ~~`link/thunk/link_thunk.rs` — 2 compares.~~ — moot: 0 sites remain.
      Letter C converted `record_native_resources` to a `ParameterType` key,
      which removed both compares as a side effect. Confirmed by
      `census_by_file`: the file does not appear in the live table at all.
- [x] **Do not touch arch dispatch in any of these files.** The conversion is to
      the type argument only; if a change appears to require touching an
      arch branch, stop and record why in Corrections. **Honoured, with one
      one-line exception that is not arch dispatch**: `linux_common/code.rs:1445`
      sets `language_entry_returns: "Integer"` as a struct literal and had to
      become `&ParameterType::Integer` when the field's type changed. No arch
      branch, no emitted instruction, and it is in `src/target` (letter G's) only
      because that is where the struct is filled in.
- [x] Lower the gate budgets for these seven files to 0. `codegen`:
      `parse_sites` 85 → 74, `str_type_params` 165 → 157,
      `spelling_compares` 58 → 48.
- [x] Tests: run the vector/SIMD `rt_*` suite explicitly and record the count.
      **The count is zero — see Correction D2, which predicted exactly this.**
      `ls tests/ | grep -iE 'vector|simd'` returns nothing. The real coverage is
      `tests/acceptance/src/vector.mfb`: **71 `TCASE`s** over all nine shapes and
      every op `vector_op_inlinable` gates — `scale`, `dot`, `cross`,
      `lerp`, `lerp_unclamped` (5 uses), `length`, `distance`, `normalize`,
      including the zero-vector trap for every element type and dimension. Run
      scoped, as in Phase 1: **passed, 1 test ran**.

Acceptance: **MET.** All 15 files in §2 read 0 on all seven needle classes —
verified by their joint absence from `census_by_file`, not by reading the
budget table. `cargo test --no-fail-fast -- --skip artifact_gate_all` → exit 0,
0 failures. The letter's end gate below passes.
Commit: —

### End-of-letter spot-check (scoped, read-only)

Before closing this letter, run the scoped artifact gate on the builtins it
touched — **`math`, `money`, `vector`, `strings`** (this letter's own cluster: arithmetic, money, SIMD, string representation):

```
scripts/artifact-gate.sh target/release/mfb math
scripts/artifact-gate.sh target/release/mfb money
scripts/artifact-gate.sh target/release/mfb vector
scripts/artifact-gate.sh target/release/mfb strings
```

Measured cost: ~31s per builtin (one builtin = 1 test, 6 builds, 7 goldens).
This is **read-only diffing**: it regenerates nothing and updates no golden. It
is multi-target — per-target goldens (`*.linux-aarch64.ncode` and friends) are
discovered by filename and rebuilt with `-target`, so cross-arch drift is caught
on a macOS host, which no other per-letter check can see.

Expect **0 diffs**. A diff here is this letter's, which is the entire point of
running it now instead of discovering it in G behind six letters of churn —
root-cause it with objdump on one fixture and fix the conversion. **Do not
regenerate a golden here.** All regeneration happens once, in letter G, after
attribution (plan-111-A §3).

**Result: 0 diffs on all four, MET.**

```
artifact-gate [math]:    1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
artifact-gate [money]:   1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
artifact-gate [vector]:  1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
artifact-gate [strings]: 1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
```

This is the check that matters most in this letter, because §3 names the SIMD,
`entry.rs` and thunk files as **host-invisible**: a mistake in them shows up on
no macOS test. The gate rebuilds each fixture for every target by filename
(`*.linux-aarch64.ncode` and friends), so the x86-64/Win64 paths through the
converted `entry.rs` return-type branch and the SIMD kernels are byte-compared
here. Nothing regenerated.

## Validation Plan

- Tests: `cargo test --no-fail-fast` — **never plain `cargo test`**. The `rt_*`
  tests sort after `golden.rs` and are silently skipped without `--no-fail-fast`;
  with the golden sweep deferred to G, they are the only tests in this letter
  that catch a wrong conversion arm.
- Gate: `cargo test --test no_type_strings` — all 15 files at 0, budgets tight.
- Coverage check: the conversion matrix in `builder_conversions.rs` is the one
  place to verify coverage rather than assume it — enumerate its arms against the
  rt fixture list and record any arm with no fixture.
- Runtime proof: **deferred to letter G.** No `test-accept.sh` run in this
  letter — the acceptance corpus and its goldens are swept once, at the end
  (plan-111-A §3). The per-phase `rt_*` runtime tests are this letter's
  behavioral signal.

- Artifact gate: **scoped spot-check only** — the builtins above, ~31s each,
  read-only. The full `artifact-gate.sh all`, `tests/golden.rs`,
  `test-accept.sh` and every golden regeneration run once, in letter G.
- Diagnostics: **not run in this letter** — this letter touches codegen, which
  emits no source diagnostics (plan-111-A §3). G re-checks it.
- Doc sync: `.ai/codegen-invariants.md` if any emitter's documented contract
  mentions a `&str` type argument. **Done** — see the entry added there for the
  three gate blind spots (Correction D1) and the boundary-render rule this
  letter established.
- Formatting: `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

None. This letter's design was settled in plan-111-A and plan-111-C; if a
decision appears necessary, it is a sign the conversion is changing behavior —
stop and record it in Corrections.

## Corrections

**D1 — the ratchet gate had three blind spots, and they hid 59 sites.** The
kickoff re-measurement (§Prerequisites) came back almost exactly matching the
plan, which is the opposite of letters A and C. That was worth being suspicious
of, and the suspicion paid: the plan and the gate agreed because they share the
same needles, not because the needles are complete.

Reading `gen_math.rs` against its own census found arms the gate does not count:

```rust
("sin", "Float") => FloatKernel::Sin,          // invisible
("min", "Integer" | "Fixed") => Kernel::MinSigned,  // invisible
"Float" => FloatKernel::Exp,                   // counted
```

`spelling_match_arms` required the arm to *begin* with a quoted spelling
(`quoted_name_end(t, 0, …)` on the trimmed line), so a tuple pattern —
dispatching on a spelling every bit as much — was not a hit. Three blind spots
in total, each fixed by strengthening the scanner and each pinned by a new
fixture in `scanners_fire_on_their_own_needles`:

| Blind spot | Fix | Sites revealed |
|---|---|---|
| a tuple/nested arm pattern | scan the whole pattern, not just its head | +25 arms |
| `== Some("Integer")` — a compare behind an `Option` shell | class 4 learns the wrapped form | +6 compares |
| ten `*type*: &str` parameter names absent from the hand-seeded `TYPE_PARAM_NAMES` | swept `src/` for `*type*: &str` and added every type-system one | +27 params |
| | **total** | **+59** |

Splitting the first two correctly mattered: `_ if t == "Integer" =>` is a
**guard**, which decides by comparing, so the arm scanner now stops the pattern
at ` if ` and hands it to `spelling_compares`. Without that split the same site
would be counted twice and filed under the wrong class. Both directions are
pinned.

The ten added parameter names are `record_type`, `result_type`, `payload_type`,
`success_type`, `ret_type`, `resource_type`, `function_type`, `stride_type`,
`block_type`, `type_str`. Deliberately **not** added, with the reason recorded
in the constant: `ctype` / `socktype` / `abi_return_ctype` (the C FFI type
vocabulary — `CInt8`, `CBool` — a different grammar, LINK's rather than
MFBASIC's) and `type_code` (a numeric collection type-code rendered as a string
immediate, not a type name).

Consequences, all of them real scope:

1. **`gen_math.rs` is 28 arms, not 45.** Phase 1 grew by 17 — every one a
   `(function, spelling)` tuple arm in the SIMD kernel selectors.
2. **A letter-B site was still live.** `src/ir/shape.rs:3005`
   `constructor_typed` matched `type_.name().as_ref()` against six nominals.
   Letter B reported `ir` clean because the scanner could not see it. Fixed here
   rather than deferred — it is `is_named` calls now, identical decision on
   every input (each of the six is a bare `Named`; `parse("Result")` has no
   special arm, the ` OF ` guard at `src/types.rs:515` is the template path),
   with the render dropped. This is the skill's "go do that letter's relevant
   track now, then return".
3. **`optimizer` is a new bucket, and it belongs to letter G.** Its three sites
   (`constant_folding.rs:101`, `lvn.rs:143`, `gvn.rs:267`) all read
   `instruction.get("type").as_deref() == Some("Integer")` — the NIR `mov_imm`
   **operand-class** attribute. That is not a separate vocabulary to be waved
   off: its producer is `target/shared/abi.rs`'s
   `move_immediate(type_: &str, …)`, which the gate *already* counts in the
   `target` bucket that letter G owns. Producer and consumers convert together,
   so the three rows are added to G's scope, not D's. Recorded as a task in G.
4. **Letters E and F must re-scope too.** Their §2 tables were built from the
   same weak needles. `builder_collection_layout.rs` gains params, `general/mod.rs`
   gains 6 tuple arms, and three files appear that no letter lists
   (`fs/gen_path_builder.rs`, `strings/func_join.rs`, `strings/gen_with_any.rs`,
   1 compare each). Re-run `census_by_file` at each kickoff.

The budget table is re-measured wholesale against the strengthened scanners and
is tight in both directions. The measuring instrument itself is now committed —
`cargo test --test no_type_strings census_by_file -- --ignored --nocapture`
prints the live per-file table, and `MFB_CENSUS_DETAIL=<substring>` adds every
offending line — so no later letter has to reconstruct a census with `rg` and
inherit its over-count (Corrections A3 and C3) or the gate's under-count (this
one). **Both failure directions now have one tool that avoids them.**

**D2 — Phase 1 had no behavioral gate, and the plan said it did.** The phase
reads "no new tests — the existing `rt_*` math/money fixtures cover these." They
do not exist: `ls tests/ | grep -iE 'rt_.*(math|money|simd|vector)'` returns one
file, `rt_native_numeric_pow_div_runtime.rs`. The actual coverage for
`math::abs/sqrt/clamp/min/max/floor/ceil/round/sin/cos/tan/exp/log/log10/pow/atan2`
lives in `tests/acceptance/src/math.mfb` and `money.mfb` — and plan-111-A §3
defers the whole acceptance corpus to letter G.

Taken literally, then, converting 51 arithmetic dispatch arms would have been
gated by "it compiles" and nothing else, in the letter whose own §3 says a
wrong arm here is *a wrong number, not a crash* — precisely the failure a
compile check cannot see.

Resolved by running the acceptance project **scoped**, in this phase:

```
scripts/test-accept.sh target/release/mfb /tmp/accept-111d acceptance
→ acceptance tests passed (1 test(s) ran)
```

This does not violate the deferred-goldens policy, which defers *regeneration*
and the *full* sweep; a scoped read-only comparison is the same category as the
scoped artifact gate every letter already runs. Letters E and F should do the
same for their own clusters rather than inherit the phrase "the rt fixtures
cover it" — **check that the fixtures exist before believing the sentence.**

Coverage was confirmed by enumerating the fixture rather than assuming it: all
22 `math::` members appear, across scalar, int-array, fixed-array and
float-array cases.

**D3 — one arm re-derives numeric promotion, and it is right to.** Phase 2 says
to call `src/numeric.rs`'s typed algebra wherever an emitter reimplements
promotion, *"but only where it is already equivalent."* The numeric comparison
dispatcher (`builder_numeric.rs`, the `promoted` binding) does reimplement it,
and it is **not** equivalent:

| Pairing | `numeric::typed_binary_result_type("+", …)` | the comparison dispatcher |
|---|---|---|
| `Float` vs `Fixed` | **`Fixed`** (`:448` tests Fixed *before* Float) | **`Float`** (tests Float first) |
| `Money` vs `Float` | `typed_money_result_type("+", true, false)` → `None` → `unwrap_or(Integer)` | **`Float`** |

Both divergences are correct, and for the same reason: comparison needs a common
**representation** to feed one compare instruction, not an arithmetic **result
type**. `Fixed = Float` widens both sides to f64 and compares there; adding them
produces a `Fixed`. Routing the comparison through the algebra would silently
change which compare instruction a `Fixed = Float` emits — a wrong answer, not a
wrong type name.

So it is left exactly as it was, converted from `String` to `ParameterType`
without touching the rule. Recorded here because "an emitter re-derives
promotion" reads like a defect and is not one — the next reader should not
"fix" it either.

The honest summary of this correction: the plan's stated goal is "delete every
type string after the AST", and for three letters the gate certifying that goal
could not see a `("sin", "Float")` arm. Letters A–C are not wrong about what
they converted; they were wrong about what was left.

## Summary

Risk is `builder_conversions.rs` (a wrong arm is a wrong number, caught only by
rt fixtures) and the SIMD/entry/thunk files (host-invisible, caught only by
letter G's cross-target sweep). Everything else is arm-for-arm substitution.

Untouched: collections and layout (letter E), memory/engine/resource/registry
(letter F), and the five sanctioned boundaries.
