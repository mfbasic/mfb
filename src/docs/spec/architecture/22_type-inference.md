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
[[src/ir/verify/env.rs:infer_type]]

The per-type rules, literal range checks, and the *defaultable* predicate are
canonical in `./mfb spec language types`; this page owns how those types are
**inferred** and the **assignability** relation that decides whether an actual
type fits an expected one.

## Expected-Type Propagation

`lower_expression(expr)` lowers with `expected = None`;
`lower_expression_with_expected(expr, Some(T))` is used only at these positions,
where the declared or contextual type re-types an unsuffixed literal. A suffixed
literal keeps its suffix type, with one conversion: an `f`-suffixed (`Float`)
literal at a `Fixed` expected type is lowered as the `Fixed` constant nearest its
binary64 value (see "Literal Coercion").
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

At every position in the table, a value whose static type is a variant of the
expected union is wrapped into that union (`UnionWrap`), whatever its expression
kind: a call result, a `collections::get` result, a field read and a `WITH`
result are wrapped exactly as an identifier or a variant constructor is. The wrap
is applied once, by `lower_expression_with_expected` itself (architecture `ir`,
"Implicit union auto-wrap"). [[src/ir/lower.rs:lower_expression_with_expected]]

The same step converts a **collection** of variants: a `List` whose element type,
or a `Map` whose value type, is a variant of the expected element (value) type,
at any nesting depth, is replaced by a call that builds the union form — a
`collections::transform` over a lifted converter for a list, a lifted
set-each-key loop for a map (`union_conversion`). It is one O(n) copy per
position; the call's result is a statement temporary like any other. A value
that already has the expected type gets no call. A list literal is lowered
against the expected element type directly and a map literal against its own
annotation, so neither is converted twice. The conversion is one-directional: a
union form at a variant position is not converted, and the verifier rejects it.
A collection of `RES` elements is not converted (its copy would have no owner to
free it), and is the one container form the verifier still accepts unconverted.
[[src/ir/lower.rs:union_conversion]] [[src/ir/lower.rs:union_converter]]

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

Rule 3 also carries union-variant subsumption into a generic call, in the one
direction it goes: a member type fits a union bucket, a union never fits a member
bucket. The registry's unifier knows where in a parameter each type variable is
met (`VarPosition`): `Value` when the variable is the whole parameter (`append`'s
item `T`), `Element` when it is a `List` element or `Map` value type at any depth
(`append`'s `List OF T`), and `Invariant` everywhere else (a `Set` element, a
`Map` key, a callable's parameters and result, a `Result`, a user generic's
argument, a thread slot). It asks the caller's declared-type oracle
(`TypeKinds::union_accepts`) for union membership, the same membership lowering
wraps by.
[[src/codegen/registry/unify.rs:unify_with_kinds]] [[src/codegen/registry/unify.rs:VarPosition]] [[src/codegen/builtins/mod.rs:TypeKinds]]

- **A union bucket accepts a member.** A variable bound to a union form accepts
  its variant form at a `Value` or `Element` position and stays bound to the
  union. `collections::append(shapes, Square[2.0])` and `collections::append(shapes,
  squares)` (`squares AS List OF Square`) both resolve `T = Shape` with `shapes AS
  List OF Shape`, so the argument's expected type is `Shape` (it is wrapped) or
  `List OF Shape` (it is converted).
- **The collection argument is the bucket and fixes its variable.** A variable
  bound or matched at an `Element` or `Invariant` position is pinned, and a pinned
  variable never widens. `collections::append(squares, aShape)` binds `T = Square`
  from the list, and the `Shape` item does not fit: `TYPE_CALL_ARGUMENT_MISMATCH`,
  whatever the value holds. The call is never re-typed `List OF Shape`. The same
  holds for `prepend`, `insert`, `set`, `getOr`'s default, `repeat`, `contains`,
  `find`, and for the two-collection form: `append(squares, shapes)` is a
  mismatch, `append(shapes, squares)` is not.
- **A variable bound only from whole values widens to a later union.** It is not
  pinned, so a later occurrence that is a union accepting the bound variant
  rebinds it. `collections::reduce(shapes, Square[1.0], f)` with `f AS FUNC(Shape,
  Shape) AS Shape` binds `U = Square` from the initial value and then `U = Shape`
  from the callback, which is the bucket; the initial value's expected type is
  `Shape`. The reverse is rejected: `collections::reduce(squares, aShape, f)` with
  `f AS FUNC(Square, Square) AS Square` is a `TYPE_CALL_ARGUMENT_MISMATCH`.

`resolved_parameter_type` lets the ARGUMENTS decide first, as the call's own type
is decided, and consults the call's expected type only to break a tie or fill an
unresolved variable. Seeding first would let a declared union form re-type an
argument that the call's type still describes by its variant form.
[[src/codegen/registry/lookup.rs:resolved_parameter_type]]

A list **literal** needs no rule of its own: `[Square[2.0], Circle[3.0]]` types
bottom-up as `List OF Square`, is accepted at `append`'s `List OF T` by the first
bullet, and is then lowered against `List OF Shape`, element by element.

### Overload resolution

For an ordinary user `FUNC` name with more than one visible signature, the monomorphizer
selects the overload — **before** the shape pass and lowering see the program,
so every call they check names one concrete, mangled symbol. Resolution requires
**exact** arity and positional type equality with no coercion, and the expected
(contextual) type is the tie-breaker for a set that differs only in return type;
`TYPE_OVERLOAD_AMBIGUOUS` is raised when the inferred argument/expected types
still leave the call unresolved. See `./mfb spec architecture monomorphization`
(`resolve_overload`/`params_match`). [[src/monomorph/lower/resolve.rs:resolve_overload]]

An overloaded name used as a **value** (`LET f AS FUNC(Integer) AS Integer = pick`)
has no arguments, so the monomorphizer selects it by the complete callable type
its context expects: the member whose ordered parameter types and return type
equal the expected `FUNC` type's, and which is `ISOLATED` whenever that type is.
It reads the same contextual sources as a return-type overload (the monomorphizer
table "Expected-type propagation"). No expected type is `TYPE_OVERLOAD_AMBIGUOUS`;
an expected type no member has is `TYPE_OVERLOAD_NO_MATCH`. A name that a local
binding shadows is the local and is never selected this way, as a call or as a
value. [[src/monomorph/lower/resolve.rs:resolve_function_reference]]

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
pseudocode describes the HIR shape predicate, where `Number f` is an
`f`-suffixed (`Float`) literal:

```text
expression_compatible(E, A, expr) =
    compatible(E, A)                                  ; structural, see below
  | E=Byte  ∧ A=Integer ∧ expr = Number n ∧ 0 ≤ n ≤ 255  ; small int literal → Byte
  | E=Fixed ∧ A∈{Integer,Float} ∧ expr = Number      ; numeric literal → Fixed
  | E=Fixed ∧ A∈{Integer,Float} ∧ expr = -Number     ; negated numeric literal → Fixed
  | E=Money ∧ A∈{Integer,Float} ∧ expr = Number, not Number f   ; decimal literal → Money
  | E=Money ∧ A∈{Integer,Float} ∧ expr = -Number, not -Number f ; negated decimal literal → Money
  | E=List OF Ee ∧ A=List OF _ ∧ expr = ListLiteral vs
        ∧ ∀ v ∈ vs: v is a numeric literal
        ∧ expression_compatible(Ee, lit_type(v), v)   ; recurse element-wise
```

IR verification sees the literal after lowering has typed it, so its coercions
are narrower: the small-int → `Byte` arm, and an `Integer` constant (or its
negation) into `Fixed` or `Money`. It admits **no** `Float` constant into a
`Fixed` or `Money` slot, and has no recursive list-literal arm. Lowering retypes
an unsuffixed literal to the slot's type and converts an `f`-suffixed one into a
`Fixed` slot, so a `Float` constant still in such a slot is either a `Float`
literal in a `Money` slot (a type error) or package IR that would hand the slot
an f64 bit pattern as its carrier (bug-820).

Properties:

- **Asymmetric.** It only widens `actual` toward `expected`; it never widens the
  expected type and is not symmetric. An `Integer`-typed *variable* assigned into
  a `Byte` slot is **not** coerced — only an `Integer` *literal* is.
- **Literal-shapes only.** The `expr` must be a `Number`, a unary-minus over a
  `Number`, or a `ListLiteral` of such literals (`numeric_literal_type` decides
  each element). A general expression that merely *has* type `Integer` is never
  coerced; the small-int → `Byte` rule re-parses the literal text and bounds it
  at `255`.
- `Fixed` accepts any numeric literal, and `Money` any numeric literal that is
  not `f`- or `F`-suffixed, with **no range check at this layer**; range/precision
  rules for `Fixed`/`Money` are in `./mfb spec language types` (§4.12).
- **Suffixed literals are intrinsically typed.** An `f`/`F`/`m`/`M`-suffixed literal
  (`mfb spec language lexical-structure` §2.1) has `actual` = `Float`/`Fixed`/`Money`
  from its suffix, *not* the untyped shape (`m` and `M` both yield `Money`, since
  there is only one money type). It is then checked by ordinary
  assignability, so a Fixed `2F` into a `Float` slot fails (no `Float`←`Fixed`
  coercion exists), while a Float `2f` into a `Fixed` slot still coerces via the
  `E=Fixed ∧ A=Float ∧ expr=Number` rule. The suffix therefore *wins over* an
  expected type: the expected type never re-types a suffixed literal the way it
  coerces an unsuffixed one. `numeric_literal_type` returns the suffix type.
- **A `Float` literal in a `Fixed` slot is converted, not re-typed.** `1.5f` in a
  `Fixed` slot is lowered as the `Fixed` constant `toFixed(1.5f)` would produce:
  the binary64 value, scaled by `2^32`, rounded to nearest with ties away from
  zero. It is the binary64 value, not the decimal spelling, that is rounded, so
  `2097152.0000000002f` is exactly `2097152` (the nearest binary64 is `2^21`)
  while the unsuffixed `2097152.0000000002` is one quantum above it. The range
  check is likewise decided on the binary64 value: `2147483647.99999994f` is
  `TYPE_FIXED_LITERAL_OVERFLOW` (its binary64 is `2^31`) even though the
  unsuffixed spelling is in range, and a non-finite `1e400f` is out of range.
  [[src/numeric.rs:fixed_constant_from_float_literal]]
- **A `Float` or `Fixed` literal in a `Money` slot is a type error.** Nothing
  converts into `Money` implicitly (`mfb spec language types`, `Money`), so
  `5.0f`, `-5.0f`, and `5.0F` in a `Money` slot are rejected by that slot's
  mismatch rule (`TYPE_BINDING_MISMATCH`, `TYPE_CALL_ARGUMENT_MISMATCH`,
  `TYPE_MATCH_PATTERN_MISMATCH`, …), exactly as a `Float` variable would be.
  Write `5.0`, `5.0m`, or `toMoney(5.0f)`.

[[src/ir/verify/compat.rs:expression_compatible]] [[src/ir/shape.rs:expression_compatible]]

## Structural Assignability

The shape and IR predicates are distinct and must be read at their own seams.
Both accept an `Unknown` operand to suppress cascading diagnostics and peel the
`RES` wrapper before comparing value types.
[[src/ir/shape.rs:compatible]] [[src/ir/verify/compat.rs:compatible]]

The **IR verifier** applies these rules in order:

1. Accept exact equality after peeling `RES`.
2. Compare `ListOf` and `ResultOf` elements recursively; compare `MapOf` keys
   and values recursively. The recursion uses rules 1, 3 and 4 only: rule 5 is
   not applied to an element, because lowering converts a collection of variants
   to its union form before the verifier sees it, and an unconverted one holds
   untagged elements. (An element marked `RES` keeps rule 5: resource
   collections are not converted.)
3. Accept function types with equal arity, isolation flag, and return type when
   corresponding parameter types are equal after peeling their `RES` markers.
4. Compare rendered identities, allowing removal of a **built-in** package
   qualifier only. Distinct registered built-in resources remain distinct even
   when they share a leaf name (`tcp.Socket` and `udp.Socket`). User-package
   qualifiers are not erased.
5. Accept an actual variant of the expected union, using its qualified identity
   or built-in bare leaf — for the whole value only (see rule 2). Otherwise
   reject.

Other composites, including `SetOf` and thread handles, therefore require exact
structural equality at this seam unless a later identity rule matches them.
`compatible` does not implement general callable variance or isolation widening:
it is also the annotation-integrity check, where a package's annotated type must
agree with the type the verifier derives. Every *assignment* position (binding,
assignment, return, call argument, constructor field, union wrap, `WITH` update,
collection element, `STATE` write) instead asks `assignable`. That is
`compatible` plus the literal coercions, plus one directional conversion: an
`ISOLATED FUNC` stored where the ordinary `FUNC` of the same parameters (after
peeling `RES`) and return is expected. The reverse, an ordinary callable in an
isolated slot, still mismatches.
[[src/ir/verify/compat.rs:compatible]] [[src/ir/verify/compat.rs:assignable]]

The **HIR shape predicate** is the source-level relation, so it does accept a
variant inside a `List` or `Map` value position (the conversion above makes it
true). It additionally peels a top-level `STATE` clause,
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
