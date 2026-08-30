# Type Inference and Assignability

MFBASIC infers expression types during IR lowering: `expression_type` answers
the type of a HIR expression against the lowering context, and
`lower_expression_with_expected` threads an optional *expected* (contextual)
type into the lowered value.[[src/ir/lower.rs:expression_type]] The pre-lowering
shape pass borrows exactly that oracle for the source rules it checks over the
HIR, and the IR verifier re-derives types from the lowered `IrValue`s
(`infer_type`), so the three agree by construction. Inference is **partially
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
| `SET STATE OF r TO <expr>` | resource's state type | state assignment |
| Constructor field value `Field := <expr>` | the field's declared type | constructor lowering |
| `WITH` field update value | the field's declared type | `WITH` lowering |
| Typed list-literal element | `expected_element` of `List OF E` | list-literal lowering |
| Inline `TRAP <call>` success value | propagated through to the inner call | `Trapped` arm |

A binding **without** an annotation, an assignment to an existing variable, and a
plain expression statement all infer with `expected = None`.

These positions are **synthesized bottom-up only** (expected is never consulted):

- **Binary and unary operands.** The `Binary`/`Unary` arms type each operand
  with no expected type, then combine. [[src/ir/lower.rs:expression_type]]
- **Member-access targets.** The target of `a.field` / `a::member` is inferred
  with no expected type.
- **Map-literal entries.** Map literals are inferred from their **explicit**
  `Map OF K TO V` annotation; `K` and `V` are never inferred from the entries.
  Each key/value expression is then *checked* against `K`/`V`. A bare map literal
  with no `OF` clause is not a valid synthesis source.
  [[src/ir/lower.rs:lower_expression_with_expected]]

### Call arguments — expected is NOT pushed into the argument

The prose model says a call argument is checked against its parameter type, and
it is — but the parameter type is **not** threaded into argument *inference*.
Each argument is typed with no expected type, then validated with
`expression_compatible(param_type, actual, expr)`: on the source path by the
shape pass over the HIR argument list (where the literal shapes are still
visible), on the package path by the IR verifier over the lowered arguments.
Literal coercion (e.g. `Integer` literal → `Byte`/`Fixed`) therefore happens at
the **check** site, not by re-inferring the literal at the parameter type.
[[src/ir/shape.rs:check_call_shape]] [[src/ir/verify/calls.rs:check_call_argument_types]]

### Overload resolution

When a name resolves to more than one visible signature, the monomorphizer
selects the overload — **before** the shape pass and lowering see the program,
so every call they check names one concrete, mangled symbol. Resolution requires
**exact** arity and positional type equality with no coercion, and the expected
(contextual) type is the tie-breaker for a set that differs only in return type;
`TYPE_OVERLOAD_AMBIGUOUS` is raised when the inferred argument/expected types
still leave the call unresolved. See `./mfb spec architecture monomorphization`
(`resolve_overload`/`params_match`). [[src/monomorph/lower.rs:resolve_overload]]

## Literal Coercion — `expression_compatible`

`expression_compatible(expected, actual, expr)` is the assignability check used at
every typed slot (bindings, returns, fields, list/map elements, call arguments,
match patterns). The IR verifier holds it over lowered values and the shape pass
holds the same relation over HIR expressions (the literal shapes below are read
from whichever form is in hand). It first tries the structural relation
`compatible`; if that fails it permits a small set of **literal-only** coercions
that widen the *actual literal* toward the *expected* type:

```text
expression_compatible(E, A, expr) =
    compatible(E, A)                                  ; structural, see below
  | E=Byte  ∧ A=Integer ∧ expr = Number n ∧ n ≤ 255  ; small int literal → Byte
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

## Structural Assignability — `compatible`

`compatible(expected, actual)` is the pure structural relation (no expression in
hand, so no literal coercion). [[src/ir/verify/compat.rs:compatible]] [[src/ir/shape.rs:compatible]]

```text
compatible(E, A):
  E = Unknown  ∨  A = Unknown            → true    ; cascade suppression
  strip RES from both, then:
    List(e),  List(a)                    → compatible(e, a)              ; invariant
    Map(ek,ev), Map(ak,av)               → compatible(ek,ak) ∧ compatible(ev,av)
    Result(e), Result(a)                 → compatible(e, a)
    Thread(em,er,eo), Thread(am,ar,ao)   → compat(em,am) ∧ compat_opt(er,ar) ∧ compat(eo,ao)
    ThreadWorker(...) — same as Thread
    Function{ep,er,eiso}, Function{ap,ar,aiso}:
        (!eiso ∨ aiso)                   ; isolated variance
      ∧ ep.len == ap.len
      ∧ ∀ i: compatible(ep[i], ap[i])    ; pairwise param compat
      ∧ compatible(er, ar)               ; return compat
    User(en), User(an)                   → en == an
                                          ∨ (trailing-segment(en) == trailing-segment(an)
                                             ∧ (either side is unregistered
                                                ∨ both resolve to the same registered TypeInfo))
                                          ∨ en (a UNION) has a variant named trailing(an)
    otherwise                            → E == A
```

Key points:

- **`Unknown` is universally compatible** on either side. `Unknown` is the
  fallback for any expression whose type could not be determined; treating it as
  compatible suppresses cascading errors. (It is also numeric and orderable —
  see below.)
- **`RES` is stripped before comparing.** The `RES` element marker is an
  ownership-axis annotation, not a distinct value type, so a `File` fits a
  `RES fs::File` slot and vice versa. `./mfb spec language resource-management`.
  [[src/ir/verify/mod.rs:resource_base_type]]
- **Containers are invariant.** `List`, `Map`, `Result`, `Thread`,
  `ThreadWorker` compare element-/component-wise via `compatible` recursively;
  there is no covariance. The optional resource plane of a thread type uses
  `compatible_optional`: both absent, or both present and compatible (a
  present/absent mismatch is incompatible). [[src/ir/verify/compat.rs:compatible]]
- **Bare vs qualified user types.** An imported type is registered under its bare
  name (`Db`) while an importer writes a qualified reference (`binding.Db`); a
  trailing-segment match makes these equal so a returned package type fits a
  `binding::Type` annotation. Two *registered* user types that merely share a
  trailing segment are **not** compatible unless they resolve to the same
  `TypeInfo`; the bare-name match only bridges a case where one side is
  unregistered. See `./mfb spec architecture type-name-encoding`.
- **Union subsumption is the only nominal widening.** If the *expected* user type
  is a `UNION`, any *actual* type whose (bare) name is one of its variant names
  is compatible — assigning a variant value into the union slot. No other
  nominal widening exists.
- **Function compatibility.** Equal parameter count, pairwise-compatible
  parameters, compatible return, and isolated variance `!expected_isolated ||
  actual_isolated` (an `ISOLATED` function value satisfies a non-isolated slot
  and an isolated slot; a non-isolated value does **not** satisfy an isolated
  slot). Note `compatible` does **not** distinguish parameter variance direction
  — params are checked with the same `compatible` as everything else, not
  contravariantly.

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
      if values empty → List(Unknown)
      element_type := infer(values[0])        ; FIRST element drives the type
      for v in values[1..]:
          a := infer(v)
          if !expression_compatible(element_type, a, v) → TYPE_LIST_ELEMENT_MISMATCH
      → List(element_type)
```

This is **order-sensitive** and one-directional: there is no least-upper-bound
across elements. `[1, 2.0]` infers `List OF Integer` from element 0, then rejects
`2.0` (a `Float` is not coercible *up* to an `Integer` literal slot); but
`[2.0, 1]` infers `List OF Float` and accepts the `Integer` literal `1` via
`expression_compatible`. To force an element type, annotate the binding
(`LET xs AS List OF Float = [1, 2]`), which takes the expected-type path.

In both paths, list elements may not contain a `Thread` type, and resource
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
