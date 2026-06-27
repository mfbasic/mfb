# Type Inference and Assignability

MFBASIC infers expression types in `src/typecheck.rs`. Inference is **partially
bidirectional**: a single entry point, `infer_expression_with_expected`, threads
an optional *expected* (contextual) type down to a few syntactic positions, while
everything else synthesizes types **bottom-up**. There is no general unification,
no type variables, and no least-upper-bound; the only "widening" steps are
literal coercion (asymmetric, literal-shapes only) and union-variant subsumption.
[[src/typecheck.rs:infer_expression_with_expected]]

The per-type rules, literal range checks, and the *defaultable* predicate are
canonical in `./mfb spec language types`; this page owns how those types are
**inferred** and the **assignability** relation that decides whether an actual
type fits an expected one.

## Expected-Type Propagation

`infer_expression(expr)` is a wrapper that calls
`infer_expression_with_expected(expr, expected = None)`. The expected type is set
to `Some(T)` only at these call sites; everywhere else it is `None`.
[[src/typecheck.rs:infer_expression]]

| Position | Expected type | Site |
|----------|---------------|------|
| Typed `LET`/`MUT`/`DIM` binding init | declared `AS T` | local binding & statement binding |
| `RETURN <expr>` | enclosing function return type | return statement |
| `RECOVER <expr>` (inline `TRAP`) | the trap's success type | recover statement |
| `SET STATE OF r TO <expr>` | resource's state type | state assignment |
| Constructor field value `Field := <expr>` | the field's declared type | `infer_constructor` |
| `WITH` field update value | the field's declared type | `infer_with_update` |
| Typed list-literal element | `expected_element` of `List OF E` | `infer_list_literal` |
| Inline `TRAP <call>` success value | propagated through to the inner call | `Trapped` arm |

A binding **without** an annotation, an assignment to an existing variable, and a
plain expression statement all infer with `expected = None`.

These positions are **synthesized bottom-up only** (expected is never consulted):

- **Binary and unary operands.** `Binary`/`Unary` arms infer each operand with a
  bare `infer_expression` (`ExprMode::Read`, no expected), then combine.
  [[src/typecheck.rs:infer_binary]]
- **Member-access targets.** The target of `a.field` / `a::member` is inferred
  with no expected type.
- **Map-literal entries.** Map literals are inferred from their **explicit**
  `Map OF K TO V` annotation; `K` and `V` are never inferred from the entries.
  Each key/value expression is then *checked* against `K`/`V`. A bare map literal
  with no `OF` clause is not a valid synthesis source.
  [[src/typecheck.rs:infer_map_literal]]

### Call arguments — expected is NOT pushed into the argument

The prose model says a call argument is checked against its parameter type, and
it is — but the parameter type is **not** threaded into argument *inference*.
`check_call` infers each argument with `infer_expression` (`expected = None`),
then validates it with `expression_compatible(param_type, actual, Some(expr))`.
Literal coercion (e.g. `Integer` literal → `Byte`/`Fixed`) therefore happens at
the **check** site, not by re-inferring the literal at the parameter type. The
only contextual use of the parameter type for a call is **return-type-overload
disambiguation** (see below). [[src/typecheck.rs:check_call]]

### Return-type-overload disambiguation

When a name resolves to more than one visible signature that all match the call's
*shape* (arity + named-arg layout), the surviving set is a **return-type overload
set**. `lookup_visible_call_sig` then picks the one signature whose `return_type`
equals the call's expected (contextual) type. If no expected type uniquely
selects one, it falls back to the **last** candidate (preserving prior behaviour);
the hard `TYPE_OVERLOAD_AMBIGUOUS` error is raised later, in the monomorphizer,
when the inferred argument/expected types still leave the call unresolved. The
final, authoritative overload resolution + symbol mangling lives in
`./mfb spec architecture monomorphization` — see `resolve_overload`/`params_match`,
which require **exact** arity and positional type equality with no coercion.
[[src/typecheck.rs:lookup_visible_call_sig]] [[src/monomorph.rs:resolve_overload]]

## Literal Coercion — `expression_compatible`

`expression_compatible(expected, actual, expr)` is the assignability check used at
every typed slot (bindings, returns, fields, list/map elements, call arguments,
match patterns). It first tries the structural relation `compatible`; if that
fails it permits a small set of **literal-only** coercions that widen the *actual
literal* toward the *expected* type:

```text
expression_compatible(E, A, expr) =
    compatible(E, A)                                  ; structural, see below
  | E=Byte  ∧ A=Integer ∧ expr = Number n ∧ n ≤ 255  ; small int literal → Byte
  | E=Fixed ∧ A∈{Integer,Float} ∧ expr = Number      ; numeric literal → Fixed
  | E=Fixed ∧ A∈{Integer,Float} ∧ expr = -Number     ; negated numeric literal → Fixed
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
- `Fixed` accepts any numeric literal **unconditionally** (no range check at this
  layer); range/precision rules for `Fixed` are in `./mfb spec language types`.

[[src/typecheck.rs:expression_compatible]] [[src/typecheck.rs:numeric_literal_type]]

## Structural Assignability — `compatible`

`compatible(expected, actual)` is the pure structural relation (no expression in
hand, so no literal coercion). [[src/typecheck.rs:compatible]]

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
                                          ∨ trailing-segment(en) == trailing-segment(an)
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
  `RES File` slot and vice versa. `./mfb spec language resource-management`.
  [[src/typecheck.rs:strip_res]]
- **Containers are invariant.** `List`, `Map`, `Result`, `Thread`,
  `ThreadWorker` compare element-/component-wise via `compatible` recursively;
  there is no covariance. The optional resource plane of a thread type uses
  `compatible_optional`: both absent, or both present and compatible (a
  present/absent mismatch is incompatible). [[src/typecheck.rs:compatible_optional]]
- **Bare vs qualified user types.** An imported type is registered under its bare
  name (`Db`) while an importer writes a qualified reference (`binding.Db`); a
  trailing-segment match makes these equal so a returned package type fits a
  `binding::Type` annotation. See `./mfb spec language type-name-encoding`.
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

`is_numeric(T)` is `true` for `Byte`, `Fixed`, `Float`, `Integer`, **and
`Unknown`**. [[src/typecheck.rs:is_numeric]]

Operator typing in `infer_binary` follows from these predicates rather than from
`compatible`: [[src/typecheck.rs:infer_binary]]

- **`=` / `<>`** accept **any two numerics** with *no* compatibility requirement
  (e.g. `Byte = Float` is allowed and yields `Boolean`). Otherwise the operands
  must be mutually `compatible` *and* both `is_comparable`.
- **`<` `>` `<=` `>=`** accept **two numerics** or **two Strings**
  (`is_orderable_string` is `String` or `Unknown`). Mixed String/numeric is a
  type error. [[src/typecheck.rs:is_orderable_string]]
- **`AND` / `OR` / `XOR`** require `Boolean`-compatible operands; **`NOT`** a
  `Boolean` operand; **`&`** two `String`-compatible operands.
- Other arithmetic operators require two numerics and produce
  `numeric_binary_result_type(op, left, right)` — a bottom-up promotion (e.g.
  `Integer + Float → Float`) defined by the numeric-promotion table, never by
  the expected type. [[src/typecheck.rs:numeric_binary_result_type]]

`Unknown` flows through every predicate as permissive (numeric, orderable,
comparable), so a single upstream error does not cascade into spurious
operator-mismatch diagnostics.

## Bare List-Literal Synthesis

When a list literal has **no** expected `List OF E` context, `infer_list_literal`
synthesizes the element type from the **first element** and then *checks* every
later element against it: [[src/typecheck.rs:infer_list_literal]]

```text
infer_list_literal(values, expected):
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
* ./mfb spec language type-name-encoding — bare/qualified user-type name forms
* ./mfb spec language collections — list/map element rules and resource elements
* ./mfb spec language resource-management — the `RES` ownership-axis marker
