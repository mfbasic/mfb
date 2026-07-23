# bug-354: `typeName(<any `strings::` call>)` fails to compile — the code builder's static-type table knows zero `strings.*` and three `math.*` targets fewer than its twin

Last updated: 2026-07-18
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (compile-time rejection of valid source)

Status: Fixed (2026-07-22)
Regression Test: tests/rt-behavior/general/func_typename_builtin_calls (new) + a `static_type_name` ↔ `builtins::*::resolve_call` parity unit test

`typeName(x)` is lowered by folding `x`'s static type to a string constant at
compile time. The fold is performed by `CodeBuilder::static_type_name`
(`src/target/shared/code/builder_value_semantics.rs:650`), whose builtin-call arm
(`:679-708`) is a hand-written match over call targets. That table contains **no
`strings.*` entry at all**, and omits `math.abs`/`math.min`/`math.max`. When it
returns `None`, `builder_values.rs:709-711` raises a hard error and the build
stops. So every one of the 32 `strings::` builtins, plus three `math::` builtins,
plus `collections::find`/`contains`/`hasKey`, make `typeName` of their result
uncompilable — on valid, well-typed source that the resolver has already
accepted.

A second, near-identical implementation of the same fold —
`static_type_name_with_types` (`src/target/shared/code/data_objects.rs:1066`) —
*does* know 18 `strings.*` targets but knows zero `math.*` targets. The two must
agree (the pre-pass interns the folded string into the literal pool that the
builder then looks up), yet **no comment, test, or type relates them**. They have
drifted in opposite directions. A third implementation, `static_nir_value_type`
(`src/target/shared/code/type_utils.rs:3`), already does the right thing by
delegating to `builtins::general/collections/strings::resolve_call`.

The single correct behavior a fix produces: `typeName(e)` compiles for every
expression `e` whose type the `builtins::*` resolvers can name, and prints that
type — `typeName(strings::upper(s))` prints `String`, `typeName(strings::split(s,
t))` prints `List OF String`, `typeName(math::abs(f))` prints `Float`.

References:

- `src/docs/man/builtins/general/typeName.txt` — `typeName` is defined over any
  expression; no builtin-call carve-out is documented.
- **bug-333** (`bugs/bug-333-string-collection-builder-duplication.md`), item
  **C1** — the cleanup-side write-up of the same three-way duplication. bug-333
  cites this defect as its one non-cleanup finding and requires it be reconciled
  *before* any collapse. **This document is that item's correctness counterpart**;
  bug-333 §C1 owns the eventual collapse, this bug owns the reconciliation.
- **bug-355** (`bugs/bug-355-map-getor-missing-hash-probe.md`) — the sibling
  correctness finding surfaced by the same cleanup review (bug-333 §C4).
- Found while independently verifying bug-333's duplication claims.

## Failing Reproduction

```
$ cat src/main.mfb
IMPORT io
IMPORT strings

SUB main
  LET s AS String = "abc"
  io::print(typeName(strings::upper(s)))
END SUB

$ mfb build
Building tnrepro (executable) for macos-aarch64
error: native code cannot determine typeName argument type while lowering eval call io.print
```

- Observed: hard compile failure, exit non-zero, no artifact produced.
- Expected: builds, and running it prints `String`.

### The exact matrix (enumerated, not sampled)

Every row below was compiled individually against `target/debug/mfb` at base
`b12213d2`, macos-aarch64, with the preamble
`LET s/t AS String`, `LET f AS Float`, `LET i AS Integer`,
`LET l AS List OF String`, `LET m AS Map OF String TO Integer`.

**Fails ✗ — `error: native code cannot determine typeName argument type` (38 calls).**

*All 32 `strings::` builtins*, without exception:

| | | | |
| --- | --- | --- | --- |
| `upper` | `lower` | `caseFold` | `normalizeNfc` |
| `trim` | `trimStart` | `trimEnd` | `trimChars` |
| `join` | `split` | `graphemes` | `graphemesCount` |
| `mid` | `replace` | `find` | `byteLen` |
| `contains` | `startsWith` | `endsWith` | `startsWithAny` |
| `endsWithAny` | `left` | `right` | `repeat` |
| `padLeft` | `padRight` | `count` | `stripPrefix` |
| `stripSuffix` | `graphemeAt` | `toBytes` | `toScalars` |

Plus: `math::abs(f)`, `math::min(f, f)`, `math::max(f, f)`,
`collections::find(l, t)`, `collections::contains(l, t)`,
`collections::hasKey(m, t)`.

**Works ✓ (contrast cases — these bound the bug and become the regression guards).**

| Expression | Why it resolves |
| --- | --- |
| `math::sqrt(f)`, `exp`, `log`, `log10`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan` | `builder_value_semantics.rs:700-703` |
| `math::pow(f, f)`, `math::atan2(f, f)` | `:704-706` |
| `math::floor(f)`, `math::ceil(f)`, `math::round(f)` | `:699` |
| `collections::get(l, 0)`, `collections::getOr(l, 5, t)` | `:695-698` |
| `toString(i)`, `toInt(s)`, `toFloat(s)`, `toFixed(s)`, `toByte(i)`, `toMoney(s)`, `toScalar(s)`, `isNumeric(s)`, `len(l)`, `typeName(s)` | `:680-688` |
| `s` (a plain local), `i + 1`, `s & t`, `NOT (i = 3)` | non-call arms `:653`, `:709-731` |

**Not this bug — excluded from the matrix.** Bare `get`/`getOr`/`find`/`mid`/
`replace` and `collections::len`/`first`/`last`/`slice` fail earlier with
`error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]`: those names do not exist in that
form. They never reach the code builder and are unrelated to this defect.

### Corrections to the reviewer's report

The prior report (carried into bug-333 §C1) stated the failure affects "all
fifteen `strings::` builtins tested". Enumerating rather than sampling shows:

1. It is **all 32** `strings::` builtins, not 15.
2. It is **not confined to `strings::`** — `math::abs`/`min`/`max` and
   `collections::find`/`contains`/`hasKey` fail identically.
3. **The mechanism is one-sided, not a mutual disagreement.** bug-333 §C1
   describes the failure as the two tables disagreeing, with `strings.padLeft`
   (in *neither* table) implicitly expected to behave differently from
   `strings.upper` (in the data-objects table only). It does not:
   `padLeft` and `upper` fail identically. The predicate for failure is exactly
   **`static_type_name` returns `None`** — `static_type_name_with_types`'s
   contents do not affect *whether* the build fails.
4. **The reverse direction (`native code string literal '<T>' has no data
   object`) does not fire in practice** and is not part of this bug. Tested
   directly: `typeName(collections::get(l, 0))` where `l AS List OF Widget` and
   `Widget` is a user `TYPE` — resolved only by `static_type_name`, absent from
   `static_type_name_with_types` — compiles and prints `Widget`. The pool is
   populated from other sources, so a pre-pass miss is survivable. Only a
   *builder* miss is fatal. A fix must not assume symmetry between the two
   failure modes.

| Environment | Config | Result |
| --- | --- | --- |
| macOS 24.6.0 aarch64 | `target/debug/mfb`, base `b12213d2` | fails ✗ |
| all targets | the tables are backend-independent, above the MIR seam | fails ✗ (by inspection) |

## Root Cause

`src/target/shared/code/builder_values.rs:708-711`:

```rust
if target == "typeName" && args.len() == 1 {
    let type_name = self.static_type_name(&args[0]).ok_or_else(|| {
        "native code cannot determine typeName argument type".to_string()
    })?;
```

`typeName` has no runtime implementation — the type must be folded to a string
constant at compile time, so an unresolved type is a build error rather than a
fallback. The resolver consulted is `CodeBuilder::static_type_name`
(`builder_value_semantics.rs:650`). Its call arm (`:679-708`) is a hand-written
match whose complete target list is:

`replace`, `typeName`, `toString`, `find`, `len`, `toInt`, `mid`, `toFloat`,
`toFixed`, `toByte`, `toMoney`, `toScalar`, `isNumeric`, `get`, `getOr`,
`collections.get`, `collections.getOr`, `math.floor`, `math.ceil`, `math.round`,
`math.sqrt`, `math.exp`, `math.log`, `math.log10`, `math.sin`, `math.cos`,
`math.tan`, `math.asin`, `math.acos`, `math.atan`, `math.pow`, `math.atan2` —
then `_ => None` at `:707`.

Note `replace`/`find`/`mid` are the *bare* forms, which no longer exist as
callable names; the `strings.`-qualified targets the resolver actually produces
are absent. Hence: zero `strings.*` coverage, and the three `math` functions
added later (`abs`/`min`/`max`) were never mirrored here.

The twin, `static_type_name_with_types` (`data_objects.rs:1066`, call arm
`:1087-1116`), covers 18 `strings.*` targets and `collections.find` but no
`math.*` and no `get`/`getOr`. It feeds the literal-pool pre-pass
(`data_objects.rs:1043-1049`), which is why the two are coupled at all. **The
invariant that they must agree is undocumented.** Searched
(`grep -rniE 'in sync|must agree|agree with|same table|mirror' src/target/shared/code/`):
the only hits are `link_thunk.rs:1536`, `os.rs:38`, and `mod.rs:1132`, all about
unrelated tables. Nothing names this coupling — confirming bug-333 §C1's finding
that the reviewer's claimed warning comment does not exist.

The contrast cases are immune for exactly one reason: their target string appears
literally in the `builder_value_semantics.rs:679-708` match. `math::sqrt` works
and `math::abs` does not, though both are `math::` and both are resolved
identically by `builtins::math`, because one is spelled in the table and the
other is not.

`static_nir_value_type` (`type_utils.rs:3`) answers the same question by
delegating to `builtins::general/collections/strings::resolve_call`
(`:32-42`). It is not defective and is the reconciliation target.

## Goal

- `typeName(e)` compiles for every `e` whose type `builtins::*::resolve_call` can
  name, and emits the correct type string. Specifically, all 38 rows in the
  Fails ✗ matrix compile and print the type listed by the `builtins::*` resolver.
- The union of the two tables is written into this file with a per-entry
  justification, before either table is deleted.
- A parity test makes the previously-undocumented invariant executable: for every
  builtin name in the `builtins::*` catalog, `static_type_name` and
  `static_type_name_with_types` return the same answer.
- The contrast-case rows (`math::sqrt`, `collections::get`, `toString`, plain
  locals, binary/unary) still compile and produce byte-identical output.

### Non-goals (must NOT change)

- **The `builtins::*::resolve_call` catalog.** It is read as the source of truth;
  this bug does not add, remove, or retype any builtin.
- **`typeName`'s compile-time-fold contract.** Do NOT "fix" this by adding a
  runtime `typeName` fallback or by emitting a placeholder string such as
  `"Unknown"` when the fold fails. Both hide the defect and change documented
  behavior.
- **Do NOT collapse the two tables in the same change.** bug-333 §C1 requires
  reconciliation to land on its own, with its own test, before any collapse. A
  collapse that silently picks one table as the winner converts this failure into
  a different one.
- **Do NOT narrow the reproduction to the 15 `strings::` builtins the prior
  report sampled.** The test must cover a `math::` and a `collections::` case too,
  since those fail for the same reason.
- Currently-working rows must not shift: the fix is purely additive to the
  builder's table.

## Blast Radius

Searched, not recalled: `grep -rn "static_type_name(" src/target/shared/code/`.
Every call site classified.

**Fixed by this bug — hard failure when the table misses:**

- `builder_values.rs:709` (`typeName` in the eval-call path) — the reproduction.
- `builder_values.rs:958` (`typeName` in the filter-predicate path) — same
  `ok_or_else` error, same table.
- `builder_values.rs:1585` (`typeName` in the helper-call path) — same.

**Latent, same root cause, different symptom — silent fallback, NOT a compile
error:**

- `builder_inplace_assign.rs:62` and `:147` — the in-place `append`/`set` fast
  path commits only when `static_type_name(&args[1])` equals the list's element
  type, else `return Ok(false)` and the general value-semantic path runs (which
  copies the collection; the arm's own comment at `:59-61` cites bug-01). Because
  the table returns `None` for every `strings.*` call,
  `append(list, strings::upper(s))` provably cannot take the fast path. Verified
  by inspection of the gate; **not demonstrated end-to-end** — an attempt to show
  it via `--ncode` was inconclusive because the constructed test case did not
  enter the fast path even in the control. In scope only in that the same table
  fix removes the cause; do not claim a measured speedup without measuring one.
- `builder_collection_queries.rs:1017` (`#collections_slice$` specialization) —
  `let Some(list_type) = ... else { return Ok(None) }`, a silent
  de-specialization on the same miss.
- `builder_math.rs:80`, `builder_numeric.rs:151` — consume the table for numeric
  typing; a miss degrades or falls through rather than erroring.

**Unaffected:**

- `data_objects.rs:1066` `static_type_name_with_types` — must be reconciled to the
  same union (that is this bug's second half), but no observed failure originates
  here; a miss on this side is survivable (see Reproduction, correction 4).
- `type_utils.rs:3` `static_nir_value_type` — already delegates to `builtins::*`.
  It is the model, not a fix target. Callers `module_analysis.rs:350`, `:351`,
  `:390` unaffected.
- All backends (`aarch64`, `x86_64`, `riscv64`) — the tables sit above the MIR
  seam; one fix covers every target.
- `builder_value_semantics.rs:757`, `:773`, `:777`, `:783` (thread runtime return
  types) — use the table's non-call arms only.

## Fix Design

Replace the `_ => None` fallthrough at `builder_value_semantics.rs:707` with
delegation to the authoritative resolvers, mirroring `type_utils.rs:32-42`:
resolve `target` through `builtins::general::resolve_call`,
`builtins::collections::resolve_call`, and `builtins::strings::resolve_call`,
passing the argument types obtained by recursing through `static_type_name`.
Keep the existing hand-written arms *ahead* of the delegation for this change, so
every currently-working row keeps its exact current answer and the diff for those
rows is provably empty. Apply the same delegation to
`data_objects.rs:1115`. Then add the parity test.

This is deliberately additive rather than a rewrite: it makes the 38 failing rows
compile without touching the 30 working ones. Collapsing the now-redundant
hand-written arms is bug-333 §C1 Phase 3 and must not happen here.

The correctness risk concentrates in one place: `get`/`getOr`/`collections.get`/
`collections.getOr` at `:695-698` return the *list element type*, deliberately
resolving only lists and returning `None` for maps (documented `:689-694`, cites
bug-01). If the `builtins::*` delegation resolves the map case where the
hand-written arm returned `None`, the in-place fast path at
`builder_inplace_assign.rs:62` would newly engage for map reads — a behavior
change outside this bug's scope. Keeping the hand-written arms first prevents
this; verify with the artifact gate.

Expected output shift: programs in the Fails ✗ matrix newly compile, and the
string pool gains their type-name entries. Nothing else should move.

**Rejected alternatives:**

- *Add the missing 38 targets to the hand-written table by hand.* Rejected: it
  fixes today's drift and guarantees tomorrow's. The next builtin added to
  `builtins::strings` re-opens the bug. Delegation plus the parity test is what
  makes it non-recurring.
- *Delete `static_type_name` and call `static_nir_value_type` everywhere.*
  Rejected for this change: the two differ in where locals' types come from
  (`self.locals` vs a passed map) and in the bug-01 list-element special case.
  That collapse is bug-333 §C1 Phase 3, after parity is proven.
- *Make `typeName` fall back to a runtime type query.* Rejected: `typeName` is
  specified as a compile-time fold; adding a runtime path is a language change.

## Phases

### Phase 1 — failing test (no behavior change)

- [x] Add an acceptance fixture under `tests/rt-behavior/general/` exercising
      `typeName` over `strings::upper/lower/trim/caseFold/join/split/graphemes/
      byteLen/find/contains/startsWith/padLeft/replace/mid/repeat`, `math::abs/
      min/max/sqrt`, `collections::find/contains/hasKey/get`, and the contrast
      cases `toString`/`len`/a plain local. Confirmed it fails today with
      `native code cannot determine typeName argument type`.

Acceptance: met.
Commit: `—`

### Phase 2 — the fix

- [x] Add `CodeBuilder::static_type_name_for_fold`
      (builder_value_semantics.rs): try the hand-written `static_type_name`, and
      on a builtin-call miss delegate to `builtins::resolve_call_return_type`,
      recursing through itself for nested calls. Route the three `typeName` fold
      sites (`builder_values.rs:721, 970, 1575`) through it.
- [x] Add the pre-pass twin `static_type_name_for_fold_with_types`
      (data_objects.rs), same shape, and route the two pre-pass `typeName` fold
      sites (`data_objects.rs`, `type_utils.rs:148`) through it — so the interned
      string the builder looks up is folded identically.
- [x] Parity unit test `data_objects::tests::typename_fold_agrees_with_the_
      authoritative_resolver`: for a catalog spanning every failing-row class, the
      pre-pass fold equals `builtins::resolve_call_return_type`. Bisected: fails if
      the fold delegation is removed.

**Deviation from the doc's fix design, with reasoning.** The doc proposed
widening `static_type_name` (and `static_type_name_with_types`) directly, keeping
the hand-written arms ahead. I did NOT do that, because those two functions are
consumed by far more than the `typeName` fold — `static_type_name` also gates the
in-place-append fast path (`builder_inplace_assign.rs:62,148`), numeric-result
typing (`builder_math.rs:80`, `builder_numeric.rs:151`), and the slice
specialization (`builder_collection_queries.rs`). Widening it there shifted the
codegen of **9 goldens** (cover-crypto/cover-audio/cover-net × 3 targets) whose
programs use *no* `typeName` at all — the fast path newly engaged inside their
inlined package bodies (which call `strings::`/`collections::` internally). That
contradicts the doc's own "Nothing else should move" and produced a
36 000-line-diff that is not honestly auditable as "the intended shift." Scoping
the resolver fallback to a dedicated fold wrapper used ONLY at the five `typeName`
sites leaves `static_type_name`/`static_type_name_with_types` byte-identical for
every other consumer, so the fast path is untouched. The doc's own get/getOr map
concern is likewise moot — those arms are unchanged.

Also: I delegate to the unified `builtins::resolve_call_return_type` (all 22
packages) rather than the doc's `general/collections/strings` trio, because the
trio does not cover `math::abs/min/max` — three of the documented Fails ✗ rows.

Acceptance: met — all matrix rows compile and print the resolver's type
(`strings::upper`→String, `strings::split`→List OF String, `math::abs`→Float,
`collections::find`→Integer, `collections::get`→String, …); contrast rows
unchanged; parity test passes.
Commit: `—`

### Phase 3 — validation

- [x] `scripts/artifact-gate.sh target/debug/mfb` → **0 diff(s)** across 1064
      tests / 1318 goldens. Because the fold is isolated, the golden delta is
      confined to programs that actually use `typeName` — and the one such program
      is the new fixture, whose goldens are freshly seeded, so nothing pre-existing
      moved. (The broad-widening approach's 9-golden shift is gone.)
- [x] Acceptance for the new fixture:
      `scripts/test-accept.sh … 'func_typename_builtin_calls'` → passed. Runtime
      output is 26 correct type strings.
- [x] Full matrix re-run: every `strings::` (sampled 15), `math::abs/min/max/
      sqrt`, and `collections::find/contains/hasKey/get` compiles and prints the
      resolver's type.
- [x] Unit suites: `target::shared::code` (114), `builtins::` (346) green.

Acceptance: met — zero pre-existing golden churn; matrix compiles; parity pinned.
Commit: `—`

## Validation Plan

- Regression test: the new `tests/rt-behavior/general/` fixture (fails today,
  passes after Phase 2) plus the resolver-parity unit test — the missing
  invariant, made executable.
- Runtime proof: `typeName(strings::upper(s))` builds and prints `String`;
  `typeName(strings::split(s, t))` prints `List OF String`;
  `typeName(math::abs(f))` prints `Float`.
- Full suite: `scripts/artifact-gate.sh`, then `scripts/test-accept.sh`.
- Doc sync: none expected — this restores behavior `typeName`'s man page already
  implies. If the union reveals a builtin whose documented return type disagrees
  with `builtins::*`, that is a separate bug; file it, do not fix it here.

## Open Decisions

- **Should the hand-written arms be kept ahead of the delegation, or removed in
  the same commit?** Recommended: keep them, for a provably empty diff on the 30
  working rows. Removing them is bug-333 §C1 Phase 3, gated on the parity test.

## Summary

A valid program does not compile. `typeName` folds its argument's type at compile
time through `CodeBuilder::static_type_name`, whose hand-written builtin table
covers zero of the 32 `strings::` builtins and three fewer `math::` builtins than
its undocumented twin in `data_objects.rs` — so all 32 `strings::` calls, plus
`math::abs`/`min`/`max` and `collections::find`/`contains`/`hasKey`, are
uncompilable inside `typeName`. Filed HIGH: it is a hard, unconditional
compile-time rejection of valid source across an entire package's public surface,
with no workaround short of not calling `typeName`.

The engineering risk is not the delegation — it is resisting the temptation to
collapse the three resolvers in the same change. Reconcile, prove parity with a
test, and leave the collapse to bug-333 §C1.
