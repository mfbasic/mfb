# bug-990: a union-variant value produced by a call, field read, `collections::get` or `WITH` is not union-wrapped at a call argument, list/map element, constructor field, `WITH` field, `MUT` assignment or global initializer, so the program traps `Allocation failed.`, segfaults, or `MATCH` fires no arm

Last updated: 2026-10-06
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/types/union-variant-expression-wrap (new; name suggested)

Take `UNION Shape  Circle  Square`, `FUNC mk AS Square`, and `FUNC show(s AS Shape)`, which
`MATCH`es on `s`. Then `show(mk())` builds cleanly and traps with `Error: 7-701-0001 Allocation
failed.` (exit 255). The same happens for `show(h.sq)` (field `sq AS Square`),
`LET sh AS List OF Shape = [mk()]`, `Box[mk()]` (field `s AS Shape`), and
`Map OF String TO Shape { "a" := mk() }`. `MUT m AS Shape = Circle[1.0]` then `m = mk()` builds,
and a later `MATCH m` fires **no** arm and exits 0. `WITH b { s := h.sq }` segfaults (exit 139).
A global `LET g AS Shape = mk()` traps too. The lowered IR passes the bare `Square` record where a
`Shape` is expected, with no `unionWrap`:
`{"kind": "call", "target": "show", "args": [{"kind": "call", "type": "Square", "target": "mk"}]}`.
Codegen and `MATCH` then read that record as a `{tag, size, data}` union block.

The bug depends on the **expression kind** of the value. A plain variable (`show(v)`), a
`MATCH`-bound variable, and a variant constructor (`show(Square[2.0])`) are wrapped correctly at
every position. A call result, field access, `collections::get` result, or `WITH` update result is
wrapped only at `LET <name> AS <union> = …` and `RETURN` from a union-typed function, because those
two statements re-wrap after lowering. Everywhere else it is not wrapped.

**Correct behavior:** wherever a value whose static type is a variant of union `U` is lowered
against expected type `U`, the lowered value is `UnionWrap { union: U, member: <variant>, value }`.
This holds for every expression kind and at every expected-type position, as it already does for
identifiers and constructors. All ✗ rows below then print `square 2.00` and `after` and exit 0.

The bug is dangerous because the build is clean and the IR verifier accepts the program. Depending
on the bytes misread as `tag`/`size`, the program crashes, or a `MATCH` silently matches nothing.

References:

- `src/docs/spec/architecture/22_type-inference.md` opening paragraph: union-variant subsumption is
  one of the only two "widening" steps. § "Structural Assignability", IR rule 5 ("Accept an actual
  variant of the expected union") accepts the `Square` argument. § "Expected-Type Propagation"
  lists the positions that receive the expected type: call argument, typed list-literal element,
  map-literal value, constructor field, `WITH` field update, assignment, typed binding init. § "Call
  arguments" says the expected parameter type "is what decides union wrapping, so a union-typed
  parameter still receives a tagged union". That promise is broken for any argument that is not an
  identifier or constructor.
- `src/docs/spec/architecture/04_ir.md` § "3. Implicit union auto-wrap": the wrap makes "the tag
  explicit in the IR … rather than re-derived in codegen". The site list there ("`LET`/`MUT`
  binds, `RETURN`, inside record constructor lowering, on plain identifier uses, `RECOVER`") is
  exactly the set of sites that work today. It is the spec text this fix updates.
- `src/docs/spec/memory/03_heap-values.md` § "Union": a data union is a self-describing
  `{tag, size, data}` block. "Constructing a variant wraps its built record block at `+16`;
  `MATCH` dispatches on `tag@0`". A bare variant record has neither word.
- `src/docs/spec/language/04_types.md` § 4.3: `LET s AS Shape = Circle[radius := 2.0]`. A union
  value is a member value. The language has no form in which a variant-typed value used as the union
  stays untagged.
- Found while measuring bug-989
  (`bugs/bug-989-list-of-variant-value-accepted-as-list-of-union-untagged.md`, Blast Radius,
  "Separate bug observed while measuring"; fixtures `/tmp/bug989/{call_arg,member_arg,callelem}`).
  That bug is an assignability gap at a **container** position. This one is a lowering-wrap gap at
  a **scalar** position. The two are independent.
- Prior instances of the same symptom from other un-wrapped seams: bug-642
  (`bugs/completed/bug-642-a-union-with-a-variant-from-an-uncalled-package-fails-to-build.md`,
  inline-`TRAP` slot) and bug-648 E (`bugs/completed/bug-648-a-trapped-poll-result-tombstones-a-live-list-element.md`,
  `RECOVER` into a union slot). Both were fixed by adding one more post-lowering
  `wrap_union_value` call at one statement, which is the per-site pattern that left these positions
  uncovered.

## Failing Reproduction

Built with `target/release/mfb` at `468ff0f94`, macOS aarch64. Harness (not checked in):
`/tmp/bug990/mk.sh <name>` writes `/tmp/bug990/<name>/{project.json,src/main.mfb}` from stdin,
builds, and runs. `/tmp/bug990/gen.sh` generates the full producer × position matrix from
`/tmp/bug990/hdr.mfb`.

`project.json`:

```json
{ "name": "call_arg", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
```

`src/main.mfb` (`/tmp/bug990/call_arg`):

```
IMPORT io

TYPE Circle
  r AS Float
END TYPE

TYPE Square
  side AS Float
END TYPE

UNION Shape
  Circle
  Square
END UNION

FUNC show(s AS Shape) AS Nothing
  MATCH s
    CASE Circle(c)
      io::print("circle " & toString(c.r))
    CASE Square(q)
      io::print("square " & toString(q.side))
  END MATCH
END FUNC

FUNC mk AS Square
  RETURN Square[2.0]
END FUNC

FUNC main AS Integer
  show(mk())
  io::print("after")
  RETURN 0
END FUNC
```

```
target/release/mfb build /tmp/bug990/call_arg && /tmp/bug990/call_arg/build/call_arg.out
```

- Observed: the build succeeds. The run prints `Error: 7-701-0001` / `Allocation failed.`, exit 255.
- Expected: `square 2.00`, `after`, exit 0.

Measured matrix. The header also declares `TYPE Hold  sq AS Square` and `TYPE Box  s AS Shape`. Run
with `/tmp/bug990/gen.sh`, then each `build/<name>.out` was run and `$?` recorded. Every cell builds
cleanly. ✓ = prints `square 2.00` then `after`, exit 0.

Producers (rows): `call` = `mk()`; `field` = `h.sq` (`LET h AS Hold = Hold[Square[2.0]]`);
`get` = `collections::get(sqs, 0)` (`sqs AS List OF Square`); `with` = `WITH w0 { side := 2.0 }`
(`w0 AS Square`); `var` = `v` (`LET v AS Square`); `matchvar` = `q` bound by `CASE Square(q)`;
`ctor` = `Square[2.0]`.

Positions (columns): `arg` = `show(X)`; `elem` = `LET sh AS List OF Shape = [X]` + `FOR EACH`;
`ctor` = `Box[X]`; `map` = `Map OF String TO Shape { "a" := X }` + loop over `collections::values`;
`with` = `WITH Box[Circle[1.0]] { s := X }`; `mut` = `MUT m AS Shape = Circle[1.0]` then `m = X`;
`let` = `LET one AS Shape = X`; `ret` = `FUNC r AS Shape … RETURN X`.

| Producer | arg | elem | ctor | map | with | mut | let | ret |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `call` | Alloc failed, 255 ✗ | Alloc failed, 255 ✗ | Alloc failed, 255 ✗ | Alloc failed, 255 ✗ | no arm, 0 ✗ | no arm, 0 ✗ | ✓ | ✓ |
| `field` | Alloc failed, 255 ✗ | Alloc failed, 255 ✗ | Alloc failed, 255 ✗ | Alloc failed, 255 ✗ | SIGSEGV, 139 ✗ | no arm, 0 ✗ | ✓ | ✓ |
| `get` | no arm, 0 ✗ | Alloc failed, 255 ✗ | no arm, 0 ✗ | Alloc failed, 255 ✗ | no arm, 0 ✗ | no arm, 0 ✗ | ✓ | ✓ |
| `with` | Alloc failed, 255 ✗ | Alloc failed, 255 ✗ | Alloc failed, 255 ✗ | Alloc failed, 255 ✗ | no arm, 0 ✗ | no arm, 0 ✗ | ✓ | ✓ |
| `var` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `matchvar` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `ctor` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

Additional measured cells:

| Fixture | Fragment | Result |
| --- | --- | --- |
| `call_global` | top-level `LET g AS Shape = mk()`; `show(g)` in `main` | Alloc failed, 255 ✗ |
| `fallible_arg` | `show(mkf(1))`, `mkf` a `FAIL`-capable `FUNC … AS Square` | Alloc failed, 255 ✗ |
| `call_lambda` | `LET f AS FUNC(Integer) AS Shape = LAMBDA(x AS Integer) -> mk()` | rejected `TYPE_BINDING_MISMATCH` (`FUNC(Integer) AS Square` vs `… AS Shape`): callable invariance, not this bug ✓ |

The `var`, `matchvar` and `ctor` rows and the `let`/`ret` columns are the contrast cases. They are
regression guards for the fix.

## Root Cause

`src/ir/lower.rs:lower_expression_with_expected` calls `wrap_union_value` in exactly two of its arms:

- `HirExpression::Identifier` (… `wrap_union_value(base, expression, expected, locals, context)`)
- `HirExpression::Constructor` (same call)

Measured with `grep -n 'wrap_union_value(' src/ir/lower.rs`: 3 statement sites + those 2 arms.
The `Call`, `MemberAccess`, `WithUpdate` and `Trapped` arms return their lowered value
unwrapped, whatever `expected` is. `collections::get` is a `Call`. `Binary`/`Unary` cannot produce a
record variant.

Three statement arms in `src/ir/lower.rs:lower_statement` add a second, post-lowering
`wrap_union_value` on the result, which hides the gap there:

- `HirStatement::Let` (local typed binding): `wrap_union_value(base, value, Some(&lowered_type), …)`
- `HirStatement::Return`: `wrap_union_value(base, value, expected.as_ref(), …)`
- `HirStatement::Recover` with a slot (bug-648 E)

Every other expected-type consumer relies on `lower_expression_with_expected` alone, and so passes
a non-identifier variant through bare:

- user-call arguments: `src/ir/lower.rs:lower_local_call_arguments`. Built-in call arguments
  are lowered the same way inside the `Call` arm, with `call_argument_expected_type`.
- list-literal elements and map-literal keys/values: the `ListLiteral`/`MapLiteral` arms of
  `lower_expression_with_expected`;
- record constructor fields: `src/ir/lower.rs:lower_constructor_args`;
- `WITH` field updates: the `WithUpdate` arm;
- assignment to an existing local/global: `HirStatement::Assign` in `lower_statement`. It has no
  post-wrap, which is why `mut` fails for every non-identifier producer;
- global initializers: `src/ir/lower.rs:lower_binding` (`IrBinding.value`), which has no post-wrap.

Nothing downstream recovers. The IR verifier accepts the bare variant because
`src/ir/verify/compat.rs:TypeEnv::compatible` rule 5 accepts a variant where its union is expected.
That rule is correct for the type relation, but it does not check that a `UnionWrap` is present. Native
codegen trusts the IR (spec 04_ir § 3: the tag is "explicit in the IR … rather than re-derived in
codegen"). It lays out and reads the slot as `{tag, size, data}`, so the `Square`'s `side` bits
become the tag and the following word becomes the `size` used by the copy. That produces the
`Allocation failed.` trap on a by-value copy (argument, list/map store), the segfault, and the
no-arm `MATCH`.

**Why the contrast cases are immune.** `var` and `matchvar` are `HirExpression::Identifier`, and
`ctor` is `HirExpression::Constructor`. Both arms wrap. The `let` and `ret` columns get the
statement-level post-wrap. The wrap guard `matches!(base, IrValue::UnionWrap { .. })` keeps the
identifier/constructor case from double-wrapping there.

## Goal

- Every ✗ cell of the matrix, plus `call_global` and `fallible_arg`, prints `square 2.00` then
  `after` and exits 0. `mfb build --ir` shows a `unionWrap` (`union: Shape`, `member: Square`) around
  the value at that position.
- Every ✓ cell still prints the same output, and its IR is unchanged. In particular, no
  `unionWrap` nests inside another `unionWrap`.

### Non-goals (must NOT change)

- Type rules. This is a lowering-only defect. Which programs build, and the diagnostics they get,
  are unchanged. Do not touch `TypeEnv::compatible` / `ShapeChecker::compatible`. bug-989 owns the
  container-element assignability change.
- No container conversion. A `List OF Square` **value** at a `List OF Shape` position is bug-989's
  hole and is not "fixed" by wrapping here. `wrap_union_value` must keep wrapping only when the whole
  value's type is a variant of the expected union (`variant_belongs_to_union`).
- Callable types: `LAMBDA … -> mk()` into `FUNC(...) AS Shape` stays rejected (`call_lambda`).
- No double wrap. A value already lowered to `UnionWrap` (constructor, identifier) must not be
  wrapped again. Union-to-union (`show(z)` with `z AS Shape`) adds no wrap.
- Resource unions: §15.4's "representation-neutral" non-owning parameter widening and the
  resource-variant cleanup bookkeeping are unchanged. Identifiers already go through
  `wrap_union_value` for resource variants today, so the fix extends the existing behavior to more
  expression kinds and adds no new rule. The rt suite must confirm this.
- Tempting wrong fix: adding one more statement-level post-wrap per failing position (Assign,
  `lower_binding`, `lower_local_call_arguments`, `lower_constructor_args`, …). That is the bug-642 /
  bug-648 E pattern that produced this gap. It leaves the next new expected-type position broken, and
  it misses nested sites (list-literal elements, map values, `WITH` fields) that do not go through a
  statement. Forbidden. See Fix Design.

## Blast Radius

Found with `grep -n 'lower_expression_with_expected(' src/ir/lower.rs` (26 call sites) and
`grep -n 'wrap_union_value(' src/ir/lower.rs` (5), plus the measured matrix.

Expression kinds (arms of `lower_expression_with_expected`):

- `Call` (user function, fallible user function, built-in such as `collections::get`) — fixed by
  this bug (measured `call`, `get`, `fallible_arg`).
- `MemberAccess` (record field read) — fixed by this bug (measured `field`).
- `WithUpdate` result — fixed by this bug (measured `with`).
- `Trapped` (inline `TRAP` used directly as an argument/element) — same hazard, not measured: the
  fixture failed to parse (`MFB_LEX_UNCLOSED_BRACKET` with a multi-line `TRAP` inside `show(…)`).
  The fix covers it, because it applies at the single exit of `lower_expression_with_expected`.
  Phase 1 adds a single-line form if one parses.
- `Identifier`, `Constructor` — already wrap; unaffected (measured `var`, `matchvar`, `ctor`).
- `Binary`, `Unary`, literals, `Lambda`, `ListLiteral`/`SetLiteral`/`MapLiteral` (as a whole value)
  — unaffected, because none of them has a record-variant static type. A literal's **elements** are
  covered through the element position.

Positions (callers that pass a union `expected`):

- `lower_local_call_arguments` and the built-in argument path in the `Call` arm — fixed (measured `arg`).
  A built-in with a union-typed parameter (for example the canvas `DrawItem` surface) has the same
  hazard. Guess: a user `FUNC dot(...) AS canvas::Circle` placed in `[dot(1)]` as a
  `List OF canvas::DrawItem` breaks the same way. Not measured, because it needs the Canvas app mode.
- `ListLiteral` elements, `MapLiteral` values — fixed (measured `elem`, `map`). `SetLiteral` elements
  cannot hold a union (`TYPE_REQUIRES_COMPARABLE`, measured in bug-989 `setcov`).
- `lower_constructor_args` (positional and named) — fixed (measured `ctor`).
- `WithUpdate` field values — fixed (measured `with`).
- `HirStatement::Assign` (local and `AssignGlobal`) — fixed (measured `mut`).
- `lower_binding` (top-level `LET`/`MUT` initializers) — fixed (measured `call_global`).
- `HirStatement::StateAssign` (`r.state = <expr>`) — same hazard if a `STATE` type is a union. Not
  measured. Covered by the same change.
- `lower_parameter_default` / `lower_call_default` — defaults are literals or constants. Covered
  anyway, and nothing is expected to change.
- `FOR` bounds, `MATCH` pattern-literal lowering (the `pattern_expected` sites) — the expected type
  is numeric or the scrutinee's, never a union expecting a variant. Unaffected.
- `HirStatement::Let`, `HirStatement::Return`, `HirStatement::Recover` — correct today through their
  post-wrap. After the fix that post-wrap is redundant but harmless (double-wrap guard). Phase 2
  decides whether to delete it. See Open Decisions.
- `src/ir/verify/compat.rs:TypeEnv::compatible` rule 5 — accepts a bare variant at a union position,
  so the verifier cannot catch this class. It is not changed by this bug. See Open Decisions for a
  verifier assertion.

## Fix Design

Make the wrap a property of `lower_expression_with_expected` itself, not of selected arms or
statements. Rename the current body to an inner `lower_expression_with_expected_inner` and make the
public function

```rust
let base = lower_expression_with_expected_inner(expression, expected, locals, context);
wrap_union_value(base, expression, expected, locals, context)
```

Then remove the two per-arm calls in `Identifier` and `Constructor`, because the outer call does
the same thing. `wrap_union_value` is already idempotent: it returns early on `expected == None`,
on a `base` that is already a `UnionWrap`, on an untypeable expression, and when the whole type is
not a variant of the expected union. So every currently-correct site produces identical IR.

Where the risk concentrates:

- **`expression_type` must agree with the lowered value's type.** `wrap_union_value` types the
  *HIR* expression, not the IR value. For a generic built-in call (`collections::get`), the result
  type must resolve to the variant (`Square`), not `Unknown` or the element's generic form.
  Today it does resolve: the `get`/`let` cell is wrapped correctly by the `Let` post-wrap, which
  uses the same oracle. Phase 1 adds a unit test so this stays true.
- **Expected-type re-entry.** The `Call` arm already threads `expected` into nested lowering (the
  `Trapped` arm propagates it to the inner call). The outer wrap must happen once, at the outermost
  frame for that expression. The `UnionWrap` guard makes the inner/outer pair safe.
- **Resource variants.** `wrap_union_value` also wraps resource variants into resource unions
  (`RES s AS Stream = <File>`). Calls returning a resource variant at a resource-union argument
  will now be wrapped where they were not. That matches what an identifier already gets, but the
  ownership classification (`value_aliases_live_resource`, spec 15 table) keys on the value's kind.
  A wrapped call is no longer syntactically a `Call` to codegen. The full rt + resource suites
  must confirm that no close obligation moves. If one does, it is a bug-hunt trigger. Do not
  carve resources out.
- **Spec sync (Hard Completion Gate).** `src/docs/spec/architecture/04_ir.md` § "3. Implicit union
  auto-wrap": replace the site list with "applied by `lower_expression_with_expected` to every
  value lowered against an expected type", and keep the double-wrap sentence.
  `src/docs/spec/architecture/22_type-inference.md` § "Expected-Type Propagation": add one sentence
  saying that, at every position in the table, a variant-typed value of any expression kind is
  wrapped into the expected union. No `mfb man` text claims otherwise
  (`grep -rn -i 'wrap' src/docs/man` has no union-wrap claim; re-check in Phase 3).

Rejected alternatives:

- **Per-site post-wraps** (add `wrap_union_value` after `lower_local_call_arguments`, after
  `lower_constructor_args`, in `Assign`, in `lower_binding`, in the literal arms). This is the
  pattern that produced the gap (bug-642, bug-648 E). It needs eight or more edits, and the next
  position added to the § "Expected-Type Propagation" table would miss it again.
- **Wrap in codegen** when a call argument's type is a variant and the parameter is a union. That
  contradicts spec 04_ir § 3, which keeps the tag explicit in the IR and serialized into the binary
  representation. `.mfp` package IR would keep the bare form.
- **Make the IR verifier reject a bare variant at a union position.** This alone turns a silent
  miscompile into an internal-looking verifier error for valid programs. At most it is a backstop
  after the fix (Open Decisions).

Expected output shift: IR goldens (`*.ir`, binary-representation) of any fixture that passes a
call, field read or `WITH` result into a union slot at a non-`LET`/`RETURN` position gain a
`unionWrap`. Any such fixture is miscompiled today, so its runtime golden should be either absent or
wrong. Phase 1's audit lists them. Any delta outside that list is a bug-hunt trigger.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/rt-behavior/types/union-variant-expression-wrap` (layout as
      `tests/rt-behavior/types/default-values-rt`). One program covering each ✗ cell: producers
      `call`, `field`, `get`, `with` × positions `arg`, `elem`, `ctor`, `map`, `with`, `mut`, plus a
      global initializer and a fallible call argument. Each prints the matched arm. Also cover the ✓
      contrast cells (`var`, `matchvar`, `ctor` × all positions; `let`/`ret` columns). Golden =
      every line `square 2.00`. Confirm it fails today (trap at the first `arg` call).
- [ ] Add a resource-union variant: a call returning `RES fs::File` passed to a
      `RES Handle`-union parameter and placed in a `List OF RES Handle` literal. Record today's
      behavior, which is unmeasured.
- [ ] Try a single-line inline-`TRAP` argument form. If it parses, add it.
- [ ] Unit test in `src/ir/` lowering tests: `lower_expression_with_expected(Call mk, Some(Shape))`
      yields `UnionWrap { Shape, Square, Call }`. Same for `MemberAccess` and `WithUpdate`.
- [ ] Audit: grep `tests/**/src/*.mfb`, `examples/**/*.mfb` and `src/codegen/builtins/tests/*.rs`
      program strings for a call/field/`WITH` result at a union-typed argument, literal element,
      constructor field, `WITH` field, reassignment or global initializer. Record each hit and its
      current golden here. Each one is a latent miscompile, or explains a golden delta in Phase 3.

Acceptance: the rt test fails today with `Allocation failed.` at the documented cell. The unit tests
fail (no `UnionWrap`). The contrast lines are recorded. The audit list is complete.
Commit: —

### Phase 2 — the fix

- [ ] `src/ir/lower.rs:lower_expression_with_expected`: split into outer wrapper + `_inner`. The
      outer applies `wrap_union_value` once. Remove the `Identifier`/`Constructor` per-arm calls
      (Fix Design).
- [ ] Decide on and apply the post-wraps in `HirStatement::Let`/`Return`/`Recover` (Open
      Decisions). Either way, the IR of the `let`/`ret` contrast cells must be byte-identical
      before and after.
- [ ] Update the `wrap_union_value` and `wrap_trap_slot_value` doc comments, which state where
      wrapping happens.
- [ ] Spec: `src/docs/spec/architecture/04_ir.md` § 3 and `22_type-inference.md`
      § "Expected-Type Propagation" (Fix Design).

Acceptance: the Phase 1 rt and unit tests pass. Every contrast cell is unchanged. No `unionWrap`
nests inside a `unionWrap` in any test's IR (`grep` over generated `.ir`).
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Run the full suite before touching any golden. Each IR/binary-representation golden delta
      must be a fixture from the Phase 1 audit, gaining exactly one `unionWrap` per audited site.
      Any other delta is a bug-hunt trigger. Inspect one fixture to localize it before concluding.
- [ ] Run the full test suite and the rt suites, including resource/RES suites and the canvas
      builtin tests (`src/codegen/builtins/tests/canvas.rs`, `app_surface.rs`).
- [ ] Re-run `/tmp/bug990/gen.sh` plus `call_global` and `fallible_arg`. Every cell prints
      `square 2.00` / `after` and exits 0.

Acceptance: full suite green. Golden deltas are exactly the audited sites. The matrix is all ✓.
Commit: —

## Validation Plan

- Regression test(s): `tests/rt-behavior/types/union-variant-expression-wrap` (rt golden), plus
  the lowering unit tests in `src/ir/`.
- Runtime proof: `/tmp/bug990/gen.sh` matrix (56 cells) + `call_global` + `fallible_arg`, all
  printing `square 2.00` / `after`, exit 0. `mfb build --ir /tmp/bug990/call_arg` shows
  `unionWrap` around the `mk` call argument.
- Doc sync: `src/docs/spec/architecture/04_ir.md` § "3. Implicit union auto-wrap" and
  `src/docs/spec/architecture/22_type-inference.md` § "Expected-Type Propagation".
- Full suite: the project's full `cargo test` plus the rt/acceptance golden harness
  (`.ai/testing-gates.md`).

## Open Decisions

- Statement-level post-wraps in `Let`/`Return`/`Recover` after the fix — **delete them** (they
  become exact no-ops, and keeping them preserves the misleading "wrap is per-site" shape) vs. keep
  them as belt-and-braces. Deleting them is safe only if the IR of every `let`/`ret`/`RECOVER`
  fixture is byte-identical. Decide from that measurement.
- Verifier backstop — add an IR-verifier assertion that a value whose inferred type is a data-union
  **variant** never reaches a union-typed slot without `UnionWrap` (recommended, as a follow-up
  after this fix lands, so a future lowering gap is an internal error instead of a miscompile)
  vs. none. Resource unions must be excluded or proven consistent first (§15.4).

## Summary

The fix is small: one choke point in `lower_expression_with_expected` replaces two per-arm calls.
The risk is in the audit and golden regeneration. Every IR delta must map to an audited miscompiled
site. Resource-variant call results start getting wrapped at union arguments, and the rt/resource
suites must show that no cleanup obligation moves. Type rules, container assignability (bug-989),
callable invariance, and the double-wrap guard are untouched.
