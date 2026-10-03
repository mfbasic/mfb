# Type Inference and Assignability

MFBASIC infers expression types during IR lowering: `expression_type` answers
the type of a HIR expression against the lowering context, and
`lower_expression_with_expected` threads an optional *expected* (contextual)
type into the lowered value.[[src/ir/lower.rs:expression_type]] The pre-lowering
shape pass borrows exactly that oracle for the source rules it checks over the
HIR, and the IR verifier re-derives types from the lowered `IrValue`s
(`infer_type`). These are separate checks; sharing the HIR oracle does not
make the HIR and IR compatibility predicates identical. Inference is **partially
bidirectional**: the expected type reaches a few syntactic positions, while
everything else synthesizes types **bottom-up**. There is no general unification,
no type variables, and no least-upper-bound; the only "widening" steps are
literal coercion (asymmetric, literal-shapes only) and union-variant subsumption.
[[src/ir/lower.rs:lower_expression_with_expected]] [[src/ir/shape.rs:type_of]]
[[src/ir/verify/mod.rs:infer_type]]

The per-type rules, literal range checks, and the *defaultable* predicate are
canonical in `./mfb spec language types`; this page owns how those types are
**inferred** and the **assignability** relation that decides whether an actual
type fits an expected one.

## Expected-Type Propagation

`lower_expression(expr)` lowers with `expected = None`;
`lower_expression_with_expected(expr, Some(T))` is used only at these positions,
where the declared or contextual type re-types an unsuffixed literal.
[[src/ir/lower.rs:lower_expression_with_expected]]

| Position | Expected type | Site |
|----------|---------------|------|
| Typed `LET`/`MUT`/`DIM` binding init | declared `AS T` | local binding & statement binding |
| `RETURN <expr>` | enclosing function return type | return statement |
| `RECOVER <expr>` (inline `TRAP`) | the trap's success type | recover statement |
| `r.state = <expr>` | resource's state type | state assignment |
| Constructor field value `Field := <expr>` | the field's declared type | constructor lowering |
| `WITH` field update value | the field's declared type | `WITH` lowering |
| Typed list-literal element | `expected_element` of `List OF E` | list-literal lowering |
| Assignment to an existing local/global | binding type | assignment lowering |
| Map-literal key/value | enclosing expected `Map OF K TO V`, when present | map lowering |
| Set-literal element | enclosing expected `Set OF E`, otherwise literal annotation | set lowering |
| Call argument | the parameter's type (see below) | builtin & user call lowering |
| Inline `TRAP <call>` success value | propagated through to the inner call | `Trapped` arm |

An unannotated binding first infers its initializer type; ordinary expression
statements have no enclosing expected type. Assignments use the target binding's
type. [[src/ir/lower.rs:lower_statement]]

These positions are **synthesized bottom-up only** (expected is never consulted):

- **Binary and unary operands.** The `Binary`/`Unary` arms type each operand
  with no expected type, then combine. [[src/ir/lower.rs:expression_type]]
- **Member-access targets.** The target of `a.field` / `a::member` is inferred
  with no expected type.

Map literals retain their explicit `Map OF K TO V` annotation as the resulting
IR type. An enclosing expected map can separately supply expected types while
lowering each entry. [[src/ir/lower.rs:lower_expression_with_expected]]

### Call arguments — the parameter type IS the expected type

A call argument is lowered with its parameter type as the expected type, from
`call_argument_expected_type`, which answers in this order:
[[src/ir/lower.rs:call_argument_expected_type]]

1. the built-in's positional parameter types, when the member has one
   monomorphic overload (`argument_types_typed`);
2. the type every overload agrees on at that position (`agreed_argument_type`) —
   this is what decides union wrapping, so a union-typed parameter still receives
   a tagged union after its member gains a second overload;
3. for a **generic** parameter (`collections::append`'s `List OF T`), the type
   the registry's own overload selection resolves it to from the call's actual
   argument types, with the call's own expected type seeding the unification
   (`resolved_parameter_type`); an ambiguous position — one where the surviving
   overloads disagree — yields no expected type rather than a guess;
4. the user function's declared parameter types, or a callable-typed local's.

An argument with no answer from any of those lowers with no expected type. The
argument is then *validated* with `expression_compatible(param_type, actual,
expr)`: on the source path by the shape pass over the HIR argument list (where
the literal shapes are still visible), on the package path by the IR verifier
over the lowered arguments.
[[src/ir/shape.rs:check_call_shape]] [[src/ir/verify/calls.rs:check_call_argument_types]]

Rule 3 is what gives an empty `[]` written at a generic parameter an element
type. Without it the literal lowers as `List OF Unknown` and the collections
lowering has no element type to check the item against.

### Overload resolution

For an ordinary user `FUNC` name with more than one visible signature, the monomorphizer
selects the overload — **before** the shape pass and lowering see the program,
so every call they check names one concrete, mangled symbol. Resolution requires
**exact** arity and positional type equality with no coercion, and the expected
(contextual) type is the tie-breaker for a set that differs only in return type;
`TYPE_OVERLOAD_AMBIGUOUS` is raised when the inferred argument/expected types
still leave the call unresolved. See `./mfb spec architecture monomorphization`
(`resolve_overload`/`params_match`). [[src/monomorph/lower.rs:resolve_overload]]

Registry-defined built-ins use their own signature/layout selection and may fill
trailing defaults; they are not subject to the user-overload default prohibition.
See `./mfb spec language functions` for the call-category contracts.

## Literal Coercion — `expression_compatible`

`expression_compatible(expected, actual, expr)` is the assignability check used at
every typed slot (bindings, returns, fields, list/map elements, call arguments,
match patterns). The IR verifier holds it over lowered values and the shape pass
holds a related predicate over HIR expressions (the literal shapes below are read
from whichever form is in hand). It first tries the structural relation
`compatible`; if that fails it permits a small set of **literal-only** coercions
that convert the *actual literal* toward the *expected* type. The following
pseudocode describes the HIR shape predicate; IR verification has the scalar
coercions but no recursive list-literal coercion arm:

```text
expression_compatible(E, A, expr) =
    compatible(E, A)                                  ; structural, see below
  | E=Byte  ∧ A=Integer ∧ expr = Number n ∧ 0 ≤ n ≤ 255  ; small int literal → Byte
  | E=Fixed ∧ A∈{Integer,Float} ∧ expr = Number      ; numeric literal → Fixed
  | E=Fixed ∧ A∈{Integer,Float} ∧ expr = -Number     ; negated numeric literal → Fixed
  | E=Money ∧ A∈{Integer,Float} ∧ expr = Number      ; decimal literal → Money
  | E=Money ∧ A∈{Integer,Float} ∧ expr = -Number     ; negated decimal literal → Money
  | E=List OF Ee ∧ A=List OF _ ∧ expr = ListLiteral vs
        ∧ ∀ v ∈ vs: v is a numeric literal
        ∧ expression_compatible(Ee, lit_type(v), v)   ; recurse element-wise
```

Properties:

- **Asymmetric.** It only widens `actual` toward `expected`; it never widens the
  expected type and is not symmetric. An `Integer`-typed *variable* assigned into
  a `Byte` slot is **not** coerced — only an `Integer` *literal* is.
- **Literal-shapes only.** The `expr` must be a `Number`, a unary-minus over a
  `Number`, or a `ListLiteral` of such literals (`numeric_literal_type` decides
  each element). A general expression that merely *has* type `Integer` is never
  coerced; the small-int → `Byte` rule re-parses the literal text and bounds it
  at `255`.
- `Fixed` and `Money` accept any numeric literal **unconditionally** (no range
  check at this layer); range/precision rules for `Fixed`/`Money` are in
  `./mfb spec language types`.
- **Suffixed literals are intrinsically typed.** An `f`/`F`/`m`/`M`-suffixed literal
  (`mfb spec language lexical-structure` §2.1) has `actual` = `Float`/`Fixed`/`Money`
  from its suffix, *not* the untyped shape (`m` and `M` both yield `Money`, since
  there is only one money type). It is then checked by ordinary
  assignability, so a Fixed `2F` into a `Float` slot fails (no `Float`←`Fixed`
  coercion exists), while a Float `2f` into a `Fixed` slot still coerces via the
  `E=Fixed ∧ A=Float ∧ expr=Number` rule. The suffix therefore *wins over* an
  expected type: the expected type never re-types a suffixed literal the way it
  coerces an unsuffixed one. `numeric_literal_type` returns the suffix type.

[[src/ir/verify/compat.rs:expression_compatible]] [[src/ir/shape.rs:expression_compatible]]

## Structural Assignability

The shape and IR predicates are distinct and must be read at their own seams.
Both accept an `Unknown` operand to suppress cascading diagnostics and peel the
`RES` wrapper before comparing value types.
[[src/ir/shape.rs:compatible]] [[src/ir/verify/compat.rs:compatible]]

The **IR verifier** applies these rules in order:

1. Accept exact equality after peeling `RES`.
2. Compare `ListOf` and `ResultOf` elements recursively; compare `MapOf` keys
   and values recursively.
3. Accept function types with equal arity, isolation flag, and return type when
   corresponding parameter types are equal after peeling their `RES` markers.
4. Compare rendered identities, allowing removal of a **built-in** package
   qualifier only. Distinct registered built-in resources remain distinct even
   when they share a leaf name (`tcp.Socket` and `udp.Socket`). User-package
   qualifiers are not erased.
5. Accept an actual variant of the expected union, using its qualified identity
   or built-in bare leaf. Otherwise reject.

Other composites, including `SetOf` and thread handles, therefore require exact
structural equality at this seam unless a later identity rule matches them.
The verifier does not implement general callable variance or isolation widening.
[[src/ir/verify/compat.rs:compatible]]

The **HIR shape predicate** additionally peels a top-level `STATE` clause,
recurses through sets and thread-handle components, and compares callable
parameters contravariantly and returns covariantly. It permits an isolated
callable in a non-isolated expected slot. Its nominal branch uses built-in-only
qualification normalization and checks declaration identity when both names
resolve. Passing shape checking alone is therefore insufficient: the lowered
program must also satisfy the IR predicate above.
[[src/ir/shape.rs:compatible]]

## Numeric and Ordering Predicates

`is_numeric(T)` is `true` for `Byte`, `Fixed`, `Float`, `Integer`, `Money`, **and
`Unknown`**. [[src/numeric.rs:is_numeric]]

Operator typing follows from these predicates rather than from `compatible`
(the IR verifier's operand rules; lowering's `expression_type` mirrors the result
types): [[src/ir/verify/values.rs]] [[src/ir/lower.rs:expression_type]]

- **`=` / `<>`** accept **any two numerics** with *no* compatibility requirement
  (e.g. `Byte = Float` is allowed and yields `Boolean`). Otherwise the operands
  must be mutually `compatible` *and* both `is_comparable`.
- **`<` `>` `<=` `>=`** accept **two numerics** or **two Strings**
  (`is_orderable_string` is `String` or `Unknown`). Mixed String/numeric is a
  type error. [[src/ir/verify/values.rs]]
- **`AND` / `OR` / `XOR`** require `Boolean`-compatible operands; **`NOT`** a
  `Boolean` operand; **`&`** two `String`-compatible operands.
- Other arithmetic operators require two numerics and produce
  `numeric_binary_result_type(op, left, right)` — a bottom-up promotion (e.g.
  `Integer + Float → Float`) defined by the numeric-promotion table, never by
  the expected type. [[src/numeric.rs]]

`Unknown` flows through every predicate as permissive (numeric, orderable,
comparable), so a single upstream error does not cascade into spurious
operator-mismatch diagnostics.

## Bare List-Literal Synthesis

When a list literal has **no** expected `List OF E` context, the element type
is synthesized from the **first element** and every later element is then
*checked* against it (`TYPE_LIST_ELEMENT_MISMATCH` is the IR verifier's):
[[src/ir/lower.rs:expression_type]] [[src/ir/verify/values.rs:check_value_depth]]

```text
list_literal_type(values, expected):
  if expected = List(Ee):
      for v in values: check expression_compatible(Ee, infer(v with expected Ee), v)
      → List(Ee)                              ; bidirectional path
  else:
      if values empty → List(Unknown)          ; only where there is no context at all
      element_type := infer(values[0])        ; FIRST element drives the type
      for v in values[1..]:
          a := infer(v)
          if !expression_compatible(element_type, a, v) → TYPE_LIST_ELEMENT_MISMATCH
      → List(element_type)
```

This is **order-sensitive** and one-directional: there is no least-upper-bound
across elements. `[1, 2.0]` infers `List OF Integer` from element 0, then rejects
`2.0` (a `Float` is not coercible *up* to an `Integer` literal slot); but
`[2.0, 1]` infers `List OF Float` but its `Integer` element also fails IR
verification. With no enclosing expected element type, neither literal is
retyped to the other. To force an element type, annotate the binding
(`LET xs AS List OF Float = [1, 2]`), which takes the expected-type path.

In both paths, list elements may not contain a `ThreadWorker` type, and resource
elements are validated separately (`./mfb spec language collections`,
`./mfb spec language resource-management`).

## See Also

* ./mfb spec language types — per-type rules, literal ranges, `defaultable` predicate
* ./mfb spec language operators — full operator typing and numeric promotion
* ./mfb spec language functions — overloading, default args, signatures
* ./mfb spec architecture monomorphization — overload resolution that consumes inferred types
* ./mfb spec architecture type-name-encoding — bare/qualified user-type name forms
* ./mfb spec language collections — list/map element rules and resource elements
* ./mfb spec language resource-management — the `RES` ownership-axis marker
