# bug-857: language spec §9–§15 drift — stale claims, misplaced/dangling citations, nonexistent §15.4

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Other (spec accuracy)

Status: Open
Regression Test: none (doc-only; verify by rendering `mfb spec language <topic>` and grep-resolving citations)

Found by the spec hardening review (batch B03) against HEAD `031f64353`. Each finding
below was confirmed by a compiled program or grep. Where a finding says the compiler
lacks a promised diagnostic, the owner must decide whether the spec or the code moves;
otherwise the spec is corrected to match the code.


---

## Finding: language spec §9-§11 drift — dangling fmod citation, union arm binding, bare enum arm, FOR EACH message, pipeline `_` continuation

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (spec/doc accuracy)

Status: Open
Regression Test: none (doc); `mfb spec` citation sweep

Several statements in `src/docs/spec/language/09_pattern-matching.md`,
`10_control-flow.md` and `11_operators.md` do not match the code. Each item is
classified; the correct behavior is "the spec states what the compiler does" (or
the owner decides which side moves for UNCLEAR items).

References: `.ai/spec-content.md`, `.ai/specifications.md`; found in hardening batch B03.

### Items

1. **SPEC — dangling citation.** `11_operators.md` cites
   `[[src/codegen/engine/operators/builder_numeric.rs:emit_float_fmod]]`. No
   `fn emit_float_fmod` exists in that file; it is defined in
   `src/codegen/builtins/math/gen_fmod.rs` (`grep -rn "fn emit_float_fmod" src`),
   and `builder_numeric.rs` only calls it (`self.emit_float_fmod(&a_bits, &b_bits)`).
   `architecture/18_math-kernels.md` already cites the right file. (The stale
   comment in `src/target/shared/plan/symbols.rs` says `builder_numeric::emit_float_fmod`
   too.) Behaviour itself verified: 20 Float MOD pairs (incl. 1e300 MOD 0.1,
   subnormals, ±1e18 randoms) bit-equal to Python `math.fmod`; `x MOD 0.0` and
   `x MOD -0.0` → 77050012.

2. **SPEC/UNCLEAR — union arm without a binding is accepted.** §9: "each non-`ELSE`
   union case must bind one local: `CASE MemberType(binding)`." Observed:
   ```
   MATCH s
     CASE Circle : RETURN 1
     CASE Rect : RETURN 2
     CASE Point : RETURN 3
   END MATCH
   ```
   builds with no diagnostic, counts toward exhaustiveness, and dispatches
   correctly (prints `123` for Circle/Rect/Point). The grammar (`19_grammar.md`:
   `pattern = unionPattern | expr`) admits it as an `expr`, and
   `src/ir/verify/matching.rs:check_match_patterns` treats a bare declared-type
   `Local` as a union arm. Either the spec should say the binding is optional, or a
   diagnostic is missing (owner decision).

   **Owner decision (2026-10-05): the binding is optional.** `CASE Member` without a
   binding is legal when the arm doesn't use the value. §9 and the grammar topic say so.
   Code unchanged.

3. **SPEC — bare enum arm is a hard error, not merely non-counting.** §9: "the
   `Type.Member` qualifier is required for the arm to count toward exhaustiveness —
   a bare `CASE Red` does not." Observed: `CASE Green` over a `Color` scrutinee is
   `SYMBOL_UNKNOWN_IDENTIFIER` ("identifier could not be resolved"); the arm never
   compiles at all. Wording implies it compiles and only fails to count.

4. **UNCLEAR — Boolean MATCH with `CASE TRUE` + `CASE FALSE` is not exhaustive.**
   Observed `TYPE_MATCH_NOT_EXHAUSTIVE` ("MATCH on open type Boolean requires an
   unguarded CASE ELSE") plus `TYPE_FUNC_MISSING_RETURN`. Consistent with the
   coverage rule ("Coverage is computed only from ... union ... enum ... CASE ELSE"),
   but §9 lists only "Open types (`Integer`, `String`, etc.)"; say explicitly that
   `Boolean` is open for exhaustiveness (or make TRUE+FALSE exhaustive).

   **Owner decision (2026-10-05): `CASE TRUE` + `CASE FALSE` is exhaustive (CODE change).**
   Boolean is a closed type for coverage, like an enum. `check_match_patterns`/coverage
   counts TRUE and FALSE, and the program above compiles without `CASE ELSE` and with no
   `TYPE_FUNC_MISSING_RETURN`. §9's coverage rule lists Boolean as closed. Add a
   compile-and-run fixture.

5. **CODE (message) — `TYPE_FOR_EACH_REQUIRES_COLLECTION` text omits Set.** §10:
   "`FOR EACH` accepts `List OF T`, `Set OF T`, and `Map OF K TO V`". The headline
   for `FOR EACH c IN "abc"` reads "FOR EACH source must be a List or Map".

6. **SPEC (footgun) — a pipeline RHS ending in `_` at end of line is a line
   continuation.** `LET c = 3 |> _ + _` followed by a newline joins the next line
   (lexical §2 rule: trailing `_` + whitespace + newline). Observed:
   `LET c = inc(2) |> _ * _` / `io::print(toString(c))` reports
   `SYMBOL_UNKNOWN_IDENTIFIER: Identifier c is not declared` on the LET line, and
   `LET c = 3 |> _ + _` alone reports "`+` requires numeric operands, got Integer
   and Nothing". Adding a trailing comment (`... _ + _ ' c`) or parentheses
   (`1 + (_)`) makes it work (prints `ok 6`, `4`). §11's pipeline notes say "The
   placeholder is the literal identifier `_`" with no mention of this; add a
   sentence (or make the lexer not treat `_` after an operator as a continuation).

### Validation Plan

- Fix the citation; rerun the spec citation sweep (`.ai/testing-gates.md`).
- Render `mfb spec language pattern-matching|control-flow|operators` after edits.

---

## Finding: collections spec claims `[2.0, 1]` infers `List OF Float` and accepts the `Integer`; the compiler rejects it

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (SPEC BUG — documentation contradicts the implementation and the type-inference spec)

Status: Open
Regression Test: tests/syntax/collections/list-literal-float-then-int-invalid (to add, pins the current rejection)

`src/docs/spec/language/12_collections.md`, paragraph "Bare-literal synthesis is
asymmetric", says:

> `[1, 2.0]` infers `List OF Integer` and **rejects** `2.0`
> (`TYPE_LIST_ELEMENT_MISMATCH`), while `[2.0, 1]` infers `List OF Float` and
> accepts the `Integer`, because an `Integer` is expression-compatible with
> `Float` but not the reverse.

The first half is true. The second half is false: `[2.0, 1]` is rejected with
`TYPE_LIST_ELEMENT_MISMATCH` ("List element has type Integer, expected Float."),
and so is the annotated `LET a AS List OF Float = [1, 2.5]`. `Integer` is not
expression-compatible with `Float` anywhere in the language: `LET a AS Float = 1`
is `TYPE_BINDING_MISMATCH` and `f(2)` for `f(x AS Float)` is
`TYPE_CALL_ARGUMENT_MISMATCH`. The cited rule in
`./mfb spec architecture type-inference` ("Literal Coercion —
`expression_compatible`") lists only Integer-literal→Byte/Fixed/Money coercions,
none into `Float`, so the collections spec contradicts its own reference.

Single correct outcome: the spec example is corrected (code is consistent with
the type-inference spec). E.g. use a coercion that exists: `[1.5, 1]` with an
expected `List OF Fixed` accepts `1`, or show `[toFloat(1), 2.0]`; and state that
`[2.0, 1]` is also rejected. The element-order asymmetry claim can be illustrated
with `Byte` (`LET b AS List OF Byte = [1, 255]` accepts; the bare `[1, …]` infers
`Integer`).

References:

- `src/docs/spec/language/12_collections.md` — "Bare-literal synthesis is asymmetric".
- `src/docs/spec/architecture/22_type-inference.md` — "Literal Coercion — `expression_compatible`".
- Found during hardening batch B03, topic `language/12_collections`.

### Failing Reproduction

```
IMPORT io
IMPORT collections
FUNC main() AS Nothing
  LET a = [2.0, 1]
  io::print(toString(collections::get(a, 1)))
END FUNC
```
`/tmp/hardening/mfb build`

- Observed:
  ```
  ./src/main.mfb:4 error[2-203-0051 TYPE_LIST_ELEMENT_MISMATCH]: list element type does not match the expected element type
                 List element has type Integer, expected Float.
  ```
- Expected (per spec text): builds, prints `1`. Per code and the type-inference
  spec: the rejection above — so the spec text is what is wrong.

Also rejected (same build): `LET a AS List OF Float = [1, 2.5]`; `LET f = 1.0` then
`[f, 2]`; `[1.5, 2, 3.0]`. Contrast that is accepted: `LET fx AS List OF Fixed = [1, 2]`,
`LET b AS List OF Byte = [1, 255]`.

### Root Cause

`src/ir/verify/values.rs:check_value_depth` (`IrValue::ListLiteral` arm) checks
each element with `src/ir/verify/compat.rs:assignable` →
`expression_compatible`, whose literal arms are only `Byte←Integer` (≤255) and
`Fixed|Money←Integer` (incl. negated). There is no `Float←Integer` arm, by design
(the type-inference spec lists none). The 12_collections paragraph predates or
misstates that rule; `git log -S` shows it unchanged since
`1588adfc4` (docs unification), so no code change removed the behavior.

### Goal

- The paragraph's examples all match `mfb build` output.

#### Non-goals (must NOT change)

- Do not add an Integer→Float coercion to make the doc true; that is a language
  change requiring an owner decision.

### Validation Plan

- Doc sync: `src/docs/spec/language/12_collections.md`; render with `mfb spec language collections`.
- Pin with a syntax fixture that `[2.0, 1]` is `TYPE_LIST_ELEMENT_MISMATCH`.

### Open Decisions

- **Owner decision (2026-10-05): fix the doc.** No implicit Integer→Float in list literals.
  The spec shows `[2.0, 1]` as `TYPE_LIST_ELEMENT_MISMATCH`, pinned by the syntax fixture.

---

## Finding: collections spec misclassifies `reduceRight` as a source generic and says native IR targets are dequalified to bare names (they stay `collections.get`)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (SPEC BUG — stale implementation description)

Status: Open
Regression Test: none (doc-only); verify by rendering `mfb spec language collections` and `mfb build --ir`

`src/docs/spec/language/12_collections.md` describes the two implementation groups
of the `collections` package. Two statements there are false against HEAD:

1. **"Native members — code-generated list/map primitives whose IR target is
   dequalified back to the bare native name"**. The IR keeps the qualified,
   dot-containing target: `collections::get(l, 0)` lowers to
   `"target": "collections.get"` (likewise `collections.set`,
   `collections.reduceRight`). The code comment at the very site says so:
   `src/ir/lower.rs` (call lowering, "Migrated `collections::`/`strings::` members
   keep their qualified, dot-containing target all the way to codegen … the freed
   bare names (`get`, `transform`, ...) can be redefined by user code").
   `src/codegen/builtins/mod.rs:native_builtin_target` only *maps* a qualified name
   to its bare lowering key for lookups; its doc comment ("the IR call target for
   these members is dequalified back to the bare name") is equally stale.
2. **`collections::reduceRight` is listed among the "Source generics"** (and
   omitted from the native list). It is native:
   `src/codegen/builtins/collections/func_reduce_right.rs` registers
   `body: Body::abi_inline(lower_reduce_right)`, and the IR shows a direct
   `collections.reduceRight` call with no `#collections_reduceRight$…` instance,
   while `collections::sort` in the same program becomes
   `#collections_sort$Integer`. The same misclassification is repeated in
   `src/docs/spec/language/18_builtin-functions.md` (§18 Collections line,
   "plus the MFBASIC-source generics `collections::sort`, `sortBy`, `take`, `drop`,
   `reduceRight`, …").

Minor, same paragraph: "A call `collections::sort(x)` is rewritten to
`__collections_sort(x)`" — the rewritten and instantiated symbol seen in IR is the
internal-sigil form `#collections_sort$Integer` (`src/monomorph/lower.rs` doc
comment: "the rewritten `#collections_sort` must flow into
`instantiate_function`"). `__collections_sort` is the source spelling, so this
is acceptable if the spec says so explicitly.

Single correct outcome: the spec (both topics) lists `reduceRight` with the
native members, describes native IR targets as the qualified `collections.<name>`
form, and names the internalized `#collections_<name>$<types>` instance.

References:

- `src/docs/spec/language/12_collections.md` — "Native members" / "Source generics" paragraphs.
- `src/docs/spec/language/18_builtin-functions.md` — Collections line.
- Found during hardening batch B03, topic `language/12_collections`.

### Failing Reproduction

```
IMPORT io
IMPORT collections
FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC
FUNC main() AS Nothing
  LET l = collections::sort([3, 1, 2])
  LET r = collections::reduceRight(l, 0, add)
  LET g = collections::get(l, 0)
  io::print(toString(r) & " " & toString(g))
END FUNC
```
`/tmp/hardening/mfb build --ir && grep -o '"target": "[^"]*"' ir1.ir | sort -u`

- Observed (main's binds, from `ir1.ir`):
  `"target": "#collections_sort$Integer"`, `"target": "collections.reduceRight"`,
  `"target": "collections.get"`; program prints `6 1`.
- Expected per spec text: native target `get` (bare), and a monomorphized
  `reduceRight` instance. Per code: what is observed — so the spec is wrong.

Body kinds measured with
`for f in src/codegen/builtins/collections/func_*.rs; do grep -o "body: Body::[a-z_]*" $f; done`:
`func_reduce_right.rs: Body::abi_inline`, `func_reduce.rs: Body::abi_inline`,
`func_sort.rs: Body::mfb_with_fast_path`.

### Root Cause

Doc drift: `reduceRight` became an `abi_inline` native lowering and the native
dispatch moved to qualified names (plan-01-functions.md §5 follow-up), but the
spec paragraphs and the `native_builtin_target` doc comment were not updated.

### Goal

- Every member named in the two group lists has the body kind the list claims.

#### Non-goals (must NOT change)

- No code change; grouping is an implementation description only.

### Validation Plan

- Doc sync: both spec topics; render `mfb spec language collections` and
  `mfb spec language builtin-functions`.

---

## Finding: language/13 modules-and-packages: stale import-resolution order and several imprecise claims and citations

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (spec drift)

Status: Open
Regression Test: none (doc); render-check `mfb spec language modules-and-packages`

Several statements in src/docs/spec/language/13_modules-and-packages.md disagree with
the implementation, which is deliberate and correct. A few citations do not back the
sentence they are attached to. Item 1 is the substantive one. **Correct behavior:** the
spec describes the order and forms the resolver actually implements, and every
citation backs its sentence.

Class: SPEC BUG for items 1–5. Item 6 is a CODE text bug (diagnostic wording). Item 7 is UNCLEAR.

References:

- src/docs/spec/language/13_modules-and-packages.md (numbered "IMPORT packageName resolves a package" list; §13.1)
- src/resolver/packages.rs:resolve_imported_package, src/manifest/package.rs:source_dependency

### Failing Reproduction / evidence

#### 1. Resolution order: installed `.mfp` is checked before `local://`, and the `file:<dir>` form is missing

The spec lists the order as 1 builtin, 2 declared, **3 `local://`** (checks `/abs/project.json`,
non-absolute → `IMPORT_LOCAL_PATH_INVALID`), 4 `packages/<name>.mfp`, 5 `packages/<name>/project.json`,
6 not installed.

The code (`resolve_imported_package`) checks `packages/<name>.mfp` first, "whatever the
`source` field says", and only then calls `source_dependency`. Measured:

- `prec_b`: `"source": "local:///…/shapes_v2"` (Limit=1) plus an installed `packages/shapes.mfp` (Limit=99) prints `99`. The `.mfp` wins over `local://`.
- `rs_relmfp`: `"source": "local://relative/x"` plus an installed `.mfp` builds with **no** `IMPORT_LOCAL_PATH_INVALID`. The spec says a non-absolute remainder is that error.
- `fsrc_b`: `"source": "file:deps_shapes"` (a project-relative directory) builds the source package. This form, named in §13.1, is absent from the numbered list.
- `rs_nodir`: `local:///…/does_not_exist` gives `IMPORT_PACKAGE_NOT_INSTALLED` ("not found at `./packages/shapes.mfp` or `…/does_not_exist/project.json`"). The spec's step 3 does not say what happens when the directory is missing.
- `fsrc_a`: `"source": "file:vend/shapes.mfp"` gives `IMPORT_PACKAGE_NOT_INSTALLED` (only `packages/shapes.mfp` is consulted). This is consistent with tooling/project-manifest's "`source` = origin URL the dependency was added from", but 13 should say a `.mfp`/`file://` source is provenance only.

Fix: reorder to builtin → declared → installed `packages/<name>.mfp` → source directory
(`local://` absolute, `file:<rel-dir>`, or implicit `packages/<name>/`; missing manifest →
`IMPORT_PACKAGE_NOT_INSTALLED`) → otherwise not installed. The `IMPORT_LOCAL_PATH_INVALID`
check applies only when no `.mfp` is installed.

#### 2. `Result` listed as a wrapper an exported surface can name

"through `List`, `Set`, `Map`, `Result`, `RES`, `FUNC` and `Thread` types". `pk_result`:
`EXPORT FUNC f(s AS Result OF Hid)` gives `TYPE_RESULT_NOT_USER_VISIBLE` (language/08
§: `Result` is never user-writable). All other listed wrappers were measured to give
`EXPORT_NAMES_NON_EXPORTED_TYPE` (pk_list/set/map/functype/thread/let/mut/unionvar/param/ret/field/privfield).
Drop `Result`, or say "an internal `Result`" if it matters for decoded IR.

#### 3. Field-visibility default cites code that does not implement it

"omitted field visibility defaults to `EXPORT` when the containing type is `EXPORT`,
otherwise to `PUBLIC` … [[src/binary_repr/writer.rs]]". `writer.rs:put_field_payload`
writes an omitted visibility as `0`, which `reader.rs:decode_type_field` decodes as
`Export` regardless of the containing type. `lower.rs:lower_field` keeps `None`, and no
code applies the documented default. The citation is file-only and does not show the claim.
(Enforcement consequences are in B03-public-field-visible-to-importers.)

#### 4. `[[src/ir/lower_link.rs:native_resources]]` on the top-level-visibility sentence

The sentence lists which top-level declarations accept `PRIVATE`/`PUBLIC`/`EXPORT`.
`native_resources` collects LINK resource metadata and has nothing to do with visibility
modifiers. Better anchors: `src/ast/types.rs:Visibility` and the parser's top-level item
visibility handling.

#### 5. `[[src/manifest/package.rs:resolved_package_file]]` attached to "A package may not be part of its own dependency graph"

`resolved_package_file` implements the *preceding* sentence (installed `.mfp` wins). The
self/cycle rejection is `src/cli/build/source_packages.rs:build_source_dependencies` /
`report_dependency_cycle`. Measured with `selfp`/`cy1`: `IMPORT_PACKAGE_MANIFEST_INVALID … depends on itself through its `packages` entries; a package may not be part of its own dependency graph.`
Move the citation. The spec also does not name the code (`IMPORT_PACKAGE_MANIFEST_INVALID`)
for cycle or depth (> 32) rejection. Consider adding it.

#### 6. (CODE text) Diagnostic contradicts the spec's re-export rule

`src/ir/shape.rs` (builtin record field-access arm) emits: "…whose fields are not visible
in this file. Imports are not transitive and **a package cannot re-export another's types**,
so add `IMPORT datetime`." The spec says "a type imported from another package may be
re-exported", and that was measured working (`producer` re-exports `origin::Item`; `fapp`
prints 42). Reword to something like "a dependency's types are not imported along with it".

#### 7. (UNCLEAR) "every required field"

"constructors for records with hidden fields are callable only from scopes that can see
every required field". Record fields have no defaults (`PUBLIC hidden AS Integer = 7` gives
`MFB_PARSE_UNEXPECTED_TOKEN`), so every field is required. Consider saying "every field". (Coordinator default, no owner question needed: say "every field", since every field is required.)

### Goal

- Items 1–5 and 7 are corrected in the spec, and item 6's message is reworded. The render
  `mfb spec language modules-and-packages` is checked.

#### Non-goals (must NOT change)

- Resolver behavior (installed `.mfp` precedence is deliberate per the code comment and
  `source_dependency_dirs`).

### Validation Plan

- Render the topic. Re-run the scratch projects named above.

### Summary

These are doc corrections. Only item 1 would mislead a user debugging which package copy got used.

---

## Finding: spec §14 promises diagnostics and copy/move rules the compiler does not implement (ThreadWorker copies alias; no "copy of non-copyable" or "cyclic construction" check)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (spec drift)

Status: Open
Regression Test: none (doc change) / optional compile-fail fixture if the owner picks the code fix for item 1

Class: UNCLEAR (item 1, owner decision) + SPEC (items 2–4).

1. **ThreadWorker assignment neither copies nor moves — it aliases.** §14.1 "Threads and
   resource handles are not copyable"; §14.2 "If the value's type is not copyable, assignment
   moves it and the source binding becomes unusable"; §14.8 "The compiler must diagnose ... Copy
   attempts for non-copyable types." Observed: inside a worker, `LET t2 = t` (t a
   `ThreadWorker`) compiles, and both `t` and `t2` stay usable and share one handle — sends via
   both arrive, and a `STATE` write through `t2` is visible through `t`.
2. **No "copy attempt for non-copyable type" diagnostic exists.** `is_copyable`
   (`src/ir/verify/resources.rs:is_copyable`) is consumed only by
   `src/ir/verify/compat.rs:check_closure_captures` (grep `is_copyable(` in src/ir/verify). Every
   other "non-copyable" case is handled by RES-binding rules (`TYPE_RESOURCE_REQUIRES_RES`) or the
   move check. §14.8's bullet describes no reachable rule.
3. **No "cyclic value construction" diagnostic exists** (`grep -n 'name: "' src/rules/table.rs`
   shows no CYCLE rule other than `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION`, which is the
   declaration-time check). Value cycles are unconstructible because values are immutable and
   owned, so §14.5/§14.8 should say "cannot be expressed" rather than "the compiler rejects /
   must diagnose".
4. **§14.9 message claim is incomplete.** §14.9: ordinary moved-name reporting uses the message
   "Binding `name` was moved and cannot be used again." regardless of path. `check_resource_moves`
   also emits a second text under the same code — "Resource `name` is closed when an error
   reaches the function's TRAP, so the handler cannot use it." (`src/ir/verify/resources.rs`,
   `trap_closed` branch). "Ordinary" arguably covers it; spelling out the TRAP variant avoids
   readers grepping for the one message.

Also observed (cosmetic): `LET ws AS List OF ThreadWorker OF String TO Integer = [t]` emits the
`2-203-0056 TYPE_COLLECTION_OWNERSHIP_VIOLATION` diagnostic **twice** at the same line
(`d_twlist.mfb`).

References:

- src/docs/spec/language/14_memory-semantics.md §14.1, §14.2, §14.5, §14.8, §14.9.
- src/docs/spec/language/16_threads.md line ~82 ("A `ThreadWorker` has no close op: the runtime
  owns it for the worker's whole run").

### Failing Reproduction

```
IMPORT io
IMPORT thread
TYPE W
  n AS Integer
END TYPE
ISOLATED FUNC w(t AS ThreadWorker OF String TO Integer STATE W, seed AS String) AS Integer
  t.state.n = 1
  LET t2 = t
  t2.state.n = 9
  RETURN t.state.n * 100 + t2.state.n
END FUNC
FUNC main() AS Integer
  RES t AS Thread OF String TO Integer = thread::start(w, "s")
  io::print(toString(thread::waitFor(t)))
  RETURN 0
END FUNC
```

(`/tmp/hardening/B03/memory/src/tw_alias.mfb`; messaging variant `d_copytw2.mfb` prints `five`,
`six`, `1`.)

- Observed: builds; prints `909` (alias: write via `t2` seen via `t`; `t` still usable).
- Expected per §14.2: `t` unusable after `LET t2 = t` (a later read is `TYPE_USE_AFTER_MOVE`), or
  a compile-time copy diagnostic per §14.8.

Contrast: parent-side `RES t2 = t` for a `Thread` marks `t` moved (`d_copythr.mfb` →
`TYPE_USE_AFTER_MOVE`); `LET g = f` for a resource → `TYPE_RESOURCE_REQUIRES_RES`
(`d_copyres2.mfb`).

### Root Cause

ThreadWorker is not a registered resource (no close op) and is not bound with RES, so neither
the RES move table nor `TYPE_RESOURCE_REQUIRES_RES` applies; `LET` of any type is a plain value
bind. The "non-copyable" concept exists only for closure captures.

### Goal

- Spec and code agree on what `LET t2 = t` of a `ThreadWorker` means, and §14.8 lists only
  diagnostics that exist.

#### Non-goals (must NOT change)

- Aliasing is harmless here (runtime-owned, no close), so do not add a runtime guard.

### Blast Radius

- §14.1/§14.2/§14.8 text; optionally the verifier's bind check if item 1 goes the code route.

### Fix Design

Recommended: SPEC — state that a `ThreadWorker` binding copies the view (shares the handle and
its STATE, like a record copy of a `RES` field), remove "copy attempts" and "cyclic value
construction" from §14.8 (or reword as "unexpressible"), and mention the TRAP-variant message.
Alternative: CODE — treat `ThreadWorker` as non-copyable at `LET`/argument binds and mark the
source moved.

### Phases

#### Phase 1 — decide item 1
- [ ] Owner picks spec vs code for ThreadWorker binding.
Commit: —
#### Phase 2 — edit spec (and code if chosen)
Commit: —
#### Phase 3 — `mfb spec language memory-semantics` render check
Commit: —

### Validation Plan

- Doc sync: §14.1, §14.2, §14.5, §14.8, §14.9. Full suite only if code changes.

### Open Decisions

- **Owner decision (2026-10-05): neither.** A `ThreadWorker` must be a Resource, bound only with
  `RES`, so `LET t2 = t` must not compile. Split out as **bug-975**. Items 2–4 here stay spec fixes.

### Summary

Spec over-promises; only item 1 has an observable consequence, and it is benign.

---

## Finding: language spec §15 cross-references a nonexistent "§15.4"

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other

Status: Open
Regression Test: none (doc)

`src/docs/spec/language/15_resource-management.md` cites "§15.4" for variant→union
widening, but the topic has no §15.1–§15.4 headings at all: its headings are
`# 15.`, `### The owner of a resource`, `### Resource identity, views, ...`,
`### Transfer outcome and cleanup routing`, `## 15.5`, `## 15.6`, `### Aggregate
routing examples`. The widening rule actually lives in the unnumbered body of §15
(the "This variant→union widening is **directional**" paragraph, line 139).
`src/docs/spec/stdlib/17_transports.md` line 74 has the same dangling "§15.4"
(for the record-field rule, which is also unnumbered §15 body text, line 143).

Class: SPEC BUG. Correct state: every "§15.N" reference resolves to a heading.

References:

- `src/docs/spec/language/15_resource-management.md` line 242: "(variant→union widening, §15.4)"
- `src/docs/spec/stdlib/17_transports.md` line 74: "§15.4 permits"

### Failing Reproduction

```
cd /Users/justinzaun/Development/mfb
grep -n "^#" src/docs/spec/language/15_resource-management.md
grep -rn "15\.4" src/docs/spec/
```

- Observed: no `15.4` heading; two references to it.
- Expected: either a `## 15.4` heading that holds the widening/record-field rules, or the references rewritten to name the paragraph / "§15".

### Root Cause

Section renumbering left the numbered references behind (guess — not traced in git).

### Goal

- `grep -rn "§15\.4" src/docs/spec` returns nothing, or a `15.4` heading exists.

### Summary

Doc-only; no code change.
