# plan-01 — Function Overload Resolution: Overridable Built-ins & Return-Type Overloads

Last updated: 2026-06-26

This document is the **normative definition and implementation plan** for two
related extensions to MFBASIC's function overload resolution. Both touch the same
machinery (`src/resolver.rs` duplicate detection, `src/typecheck.rs`
`lookup_visible_call_sig` / `call_shape_matches_sig`, `mfbasic.md` §6) and are
specified together so the resolution rule stays coherent:

- **Parts A–E — Overridable general built-ins.** Make the **general (universal,
  unqualified) built-in functions** — `toString`, `len`, `typeName`, the `to*`
  conversions, and the `is*` predicates — **overridable** by ordinary user- or
  package-defined functions, the way `net::Url` already overrides `toString`.
- **Part F — Return-type-distinguished overloads.** Allow an overload set to be
  distinguished by **return type** when the parameter lists are identical, resolved
  by the call's expected (contextual) type. First consumer:
  `encoding::utf8Encode` (plan-02-encoding.md).

## Part A–E overview — overridable general built-ins

A program (or a package) may declare
`FUNC toString(value AS MyType) AS String`, `FUNC len(value AS Grid) AS Integer`,
or `FUNC isEmpty(value AS Ring) AS Boolean`, and a plain `toString(x)` / `len(x)` /
`isEmpty(x)` call binds to that declaration whenever its argument types match — the
scalar built-in keeps handling the types it already supports.

> **Scope in one sentence.** These are the functions in `src/builtins/general.rs`
> that are called **without a package qualifier** (`is_general_call`); package
> members (`strings::lower`, `collections::get`, …) are a separate, already-shadowable
> mechanism (plan-01-functions.md §5) and are **out of scope** here.

It complements:

- `specifications/mfbasic.md` §6 (FUNC/SUB overloading), §18 (the universal builtins)
- `specifications/plan-03-http.md` §A.3 (the `toString(net::Url)` precedent)
- `specifications/plan-02-encoding.md` (`encoding::utf8Encode`, the first consumer
  of the Part F return-type overloads)
- the existing `toString` override machinery: `builtins::to_string_override_target`
  (`src/builtins/mod.rs`), the gap-fill check in `check_general_builtin_call`
  (`src/typecheck.rs`), and the call-target routing in IR lowering (`src/ir.rs`).

---

# Part A — Model

## A.1 The precedent: `toString(net::Url)`

`toString` is the universal renderer (`mfbasic.md` §18). plan-03-http.md added the
first override: a `toString(url)` call over a `net::Url` routes to the package's
internal renderer instead of the scalar built-in. Two facts made that safe and they
generalize directly:

1. **Gap-fill, not hijack.** The override is consulted **only when the built-in
   rejects the argument types** — `builtins::general::resolve_call("toString",
   ["Url"])` returns `None`, so there is nothing to hijack. `toString(42)` is
   untouched.
2. **Collision-free symbol.** The override is an ordinary function with a symbol
   that is **distinct** from the built-in dispatch name `toString`, so code
   generation never confuses the two.

This plan turns that one hand-wired case into a **uniform rule** covering every
overridable general built-in and both override sources (user code and packages).

## A.2 The overridable set

| Built-in | Overridable | Notes |
|----------|-------------|-------|
| `toString` | ✅ | Render a value as text. |
| `len` | ✅ | A "length"/"size" for a user container type. |
| `typeName` | ✅ | A custom type label. |
| `toInt` `toFloat` `toFixed` `toByte` | ✅ | Parse/convert a user type to a number. |
| `isEmpty` `isNotEmpty` | ✅ | Emptiness of a user container type. |
| `isNumeric` `isEven` `isOdd` `isPositive` `isNegative` `isZero` | ✅ | Numeric-style predicates over a user type. |
| `error` | ❌ **reserved** | `error(code, message)` is a language primitive that builds the read-only `Error` record (`typecheck::infer_constructor`); it is **not** overridable. |

The reserved set is exactly `{ error }`. `builtins::general::is_overridable(name)`
(new) returns `true` for every `is_general_call(name)` except `error`.

## A.3 The dispatch rule (normative)

For an **unqualified** call `f(args)` whose callee `f` is an overridable general
built-in, resolution proceeds in this order:

1. **Built-in first.** If `builtins::general::resolve_call(f, arg_types)` succeeds,
   the call is the **built-in** (`toString(Integer)`, `len("abc")`, …). The
   built-in is authoritative for the scalar/collection types it already supports;
   a user overload can **never** shadow them.
2. **Override fills the gap.** Otherwise, if a **visible** user- or package-defined
   `FUNC f` has an overload whose parameter types match `arg_types`, the call binds
   to that override and yields its declared return type.
3. **Neither matches** → the existing `TYPE_CALL_ARGUMENT_MISMATCH` error, unchanged.

"Gap-fill, built-in wins for its own types" is the deliberate choice: it is a pure
**extension** of today's behavior (every call that compiles today compiles
identically), so it carries **zero regression risk** for existing programs.
Shadowing a scalar built-in (e.g. redefining `toString(Integer)`) is a **non-goal**
(§E.3).

## A.4 Overload selection is by argument type

An override name may carry several overloads (`FUNC len(Grid)`, `FUNC len(Ring)`),
and it coexists with the built-in. Selection is therefore **type-directed**: the
chosen overload is the visible `FUNC f` whose parameter type list **equals** the
call's argument type list (the same exact-match rule
`monomorph::resolve_overload` already uses for ordinary overloaded calls). This is
stricter than the current arity-only `call_shape_matches_sig`
(`src/typecheck.rs`), which §C extends. When no overload matches the arity/types,
the built-in is the only candidate (rule A.3.1) and its own diagnostic applies.

## A.5 Definition legality and visibility

- **Declaring** `FUNC f(...)` for an overridable general built-in `f` is **legal**
  (it is already accepted by the parser/resolver; this plan blesses it). Declaring
  `FUNC error(...)` is **rejected** (`SYMBOL_RESERVED_BUILTIN_NAME`, new) because
  `error` is reserved (§A.2).
- An override obeys ordinary **visibility**: a `Private` override is visible only in
  its file; a `Package`/`Export` override is visible per the normal rules. A call in
  a file that cannot see any matching override falls back to the built-in (rule
  A.3.1) or errors — there is no "spooky action" from an invisible override.
- Overrides compose with **recursion**: a `FUNC toString(value AS List OF Point)`
  may call `toString(p)` on each element, which re-enters override resolution for
  `Point`.

---

# Part B — Two override sources

Overrides reach a program two ways; both obey Part A but differ in how the compiler
sees them, because **injected package source skips monomorphization**
(`monomorph::monomorphize_project` runs on the raw AST; `json`/`net`/`http` source
is added later by `augmented_project` in resolve/typecheck/IR).

## B.1 User-defined overrides (in monomorphized user code)

A `FUNC toString(Point)` written in the user's own project is an ordinary AST
function. It flows through `monomorph`, so the call site `toString(p)` is rewritten
there to the override's concrete symbol (§C.3). This is the general path.

## B.2 Package-provided overrides (injected source)

A built-in package may ship a renderer for one of its value types — the existing
`net::Url` case. Because the injected file bypasses `monomorph`, its overrides are
declared with the `__pkg_name` convention (internalized to a `#pkg_name` sigil
symbol, `internal_name.rs`) and **registered** so the call site can be routed during
IR lowering. The current single-purpose `to_string_override_target(type)` registry
generalizes to a **two-key** registry:

```rust
// src/builtins/mod.rs
pub(crate) fn general_override_target(builtin: &str, arg_type: &str) -> Option<&'static str>;
//  ("toString", "Url") => Some("__net_urlToString")
//  ...one row per package-provided override
```

`to_string_override_target` is retired in favor of `general_override_target("toString", t)`.

---

# Part C — Implementation Plan

The work generalizes the three edits that landed `toString(net::Url)`; no new
compiler stage is introduced.

## C.0 Reuse inventory

| Need | Reused as | Source |
|------|-----------|--------|
| "Is this an overridable builtin?" | new `general::is_overridable` over `is_general_call` | `src/builtins/general.rs` |
| Built-in-rejects-these-args test | `general::resolve_call(name, arg_types).is_none()` | `general.rs` |
| Package override registry | generalize `to_string_override_target` → `general_override_target` | `src/builtins/mod.rs` |
| Type-exact overload pick | `monomorph::resolve_overload`'s match predicate | `src/monomorph.rs` |
| Collision-free package symbol | `__pkg_name` → `#pkg_name` internal sigil | `src/internal_name.rs` |
| Collision-free user symbol | force-mangled overload concrete name | `monomorph::overload_concrete_name` |
| Call-target routing at lowering | the `toString` override branch in `lower_value`'s `resolved_target` | `src/ir.rs` |

## Phase 1 — Overridable-set predicate

`src/builtins/general.rs`: add `is_overridable(name) -> bool` = `is_general_call(name)
&& name != "error"`. Add a `reserved_builtin_name(name) -> bool` = `name == "error"`
for the definition-time check (§C.2).

## Phase 2 — Definition legality

`src/resolver.rs`: when registering a top-level `FUNC`/`SUB`, if its name is a
**reserved** built-in (`general::reserved_builtin_name`), report
`SYMBOL_RESERVED_BUILTIN_NAME`. Names that are **overridable** built-ins are
accepted silently (today they already are; add a regression test so this stays
true). Generic (`template_params`) overrides of a built-in name are accepted and
mangled like any generic.

## Phase 3 — Type-directed override resolution (the shared helper)

Add one resolver-independent helper used by both typecheck and IR:

- `typecheck`: `find_general_override(file, callee, arg_type_names) -> Option<&FunctionSig>`
  — the visible `FUNC callee` overload whose parameter types **equal**
  `arg_type_names` (exact match, mirroring `resolve_overload`). Replaces the
  arity-only path for this case; `call_shape_matches_sig` is **extended** with an
  optional type-list filter so overloaded built-in-named calls select precisely.

## Phase 4 — Typecheck: gap-fill every overridable builtin

`src/typecheck.rs`, `check_general_builtin_call`: today the `toString`-only override
fallback runs when `general::resolve_call` returns `None`. Generalize it:

```text
let Some(resolved) = general::resolve_call(callee, &arg_type_names) else {
    if general::is_overridable(callee) {
        // package override (registry) …
        if let Some(_) = builtins::general_override_target(callee, &arg_type_names[0]) { return <builtin return type for callee>; }
        // user override (visible FUNC) …
        if let Some(sig) = self.find_general_override(file, callee, &arg_type_names) { return sig.return_type; }
    }
    <existing TYPE_CALL_ARGUMENT_MISMATCH>
};
```

The package-override return type is the built-in's declared result for that callee
(`toString`→`String`, `len`→`Integer`, …); a small `general::override_result_type`
table provides it. The user-override return type is the overload's own declared
return type (the language does not constrain it, but a lint may warn when it
diverges from the built-in's conventional result — §E.2).

## Phase 5 — Monomorph: mangle and route user overrides

`src/monomorph.rs`:

1. **Force-mangle.** In the concrete-name pass, a user `FUNC` whose name is an
   overridable built-in is **always** given a mangled concrete symbol
   (`overload_concrete_name(function, /*force=*/ true)`), even as a sole overload,
   so its codegen symbol never equals the built-in dispatch name.
2. **Route the call.** In `lower_expression`'s call rewriting, before falling back
   to the bare callee, if `callee` is an overridable built-in and a local overload's
   parameter types match the (already-inferred) `arg_types`, rewrite the callee to
   that overload's mangled name. A non-matching call (scalar args) is left as the
   bare built-in name for codegen to dispatch. `resolve_overload` is generalized to
   fire for **single-candidate** built-in-named functions (today it requires
   `candidates.len() > 1`).

## Phase 6 — IR: route package overrides

`src/ir.rs`, the `resolved_target` computation in call lowering: the existing
`toString` branch becomes general — for **any** overridable built-in `f`, look up
`general_override_target(f, first_arg_type)` and, when present, internalize it as the
call target. User overrides need no IR change: monomorph already rewrote them to a
concrete symbol (Phase 5), so they arrive as ordinary user calls.

> Codegen needs **no** change: every override resolves to either a `#pkg_name`
> internal symbol (package) or a mangled user symbol (user), both already emitted as
> ordinary function bodies. Only the un-overridden bare names (`toString`, `len`, …)
> reach the native built-in lowering, exactly as today.

## Phase 7 — Argument coercion uses the override's parameter types

`src/ir.rs`, `builtin_argument_types`: when an overridable built-in call selects a
user/package override, per-argument literal coercion (e.g. a `[]`/`{}` literal in an
argument position) must use the **override's** parameter types, not the built-in
signature. Thread the selected overload's parameter list (from Phase 5/6) into
`call_argument_expected_type` for this case.

## Phase 8 — Tests (golden)

Offline, deterministic. Mirror `tests/func_general_*` and the new
`func_net_url_toString_valid`:

- `func_override_tostring_user` — a user `FUNC toString(Point)`; `toString(p)` uses
  it while `toString(42)` stays built-in; a `List OF Point` renderer recurses.
- `func_override_len_user`, `func_override_isEmpty_user` — `len`/`isEmpty` over a
  user container type; scalar/collection calls unaffected.
- `func_override_toint_user` — `toInt(MyType)`; `toInt("7")` unchanged.
- `func_override_visibility` — a `Private` override is not seen across files (falls
  back to the built-in or errors); a `Package` override is.
- `func_override_overloaded` — two overloads of `len` selected by argument type.
- `func_override_reserved_invalid` — `FUNC error(...)` →
  `SYMBOL_RESERVED_BUILTIN_NAME`.
- `func_override_no_hijack_invalid` — a user `FUNC toString(Integer)` does **not**
  win; the built-in still handles `toString(42)` (the user overload is dead for the
  built-in's own types, optionally a `WARN_BUILTIN_SHADOWED` lint).
- Package path is already covered by `func_net_url_toString_valid`; add
  `func_override_package_len` if a built-in package adopts `len` (otherwise the
  registry path is exercised by the `net::Url` `toString` row).

Regenerate with `scripts/sync-goldens.sh`; verify with `scripts/test-accept.sh`.

---

# Part D — Worked examples

```basic
IMPORT io

TYPE Point
  x AS Integer
  y AS Integer
END TYPE

' Override the universal renderer for Point.
FUNC toString(p AS Point) AS String
  RETURN "(" & toString(p.x) & "," & toString(p.y) & ")"   ' inner calls stay built-in
END FUNC

' Override the universal length for a user container.
TYPE Ring
  items AS List OF Integer
END TYPE
FUNC len(r AS Ring) AS Integer
  RETURN len(r.items)                                        ' inner call stays built-in
END FUNC
FUNC isEmpty(r AS Ring) AS Boolean
  RETURN len(r.items) = 0
END FUNC

FUNC main AS Integer
  LET p AS Point = Point[3, 4]
  io::print(toString(p))            ' (3,4)        — override
  io::print(toString(42))           ' 42           — built-in, unchanged
  LET r AS Ring = Ring[[1, 2, 3]]
  io::print(toString(len(r)))       ' 3            — len override
  io::print(toString(isEmpty(r)))   ' false        — isEmpty override
  RETURN 0
END FUNC
```

A `List OF Point` renderer recurses through override resolution:

```basic
FUNC toString(ps AS List OF Point) AS String
  MUT out AS String = "["
  MUT first AS Boolean = TRUE
  FOR EACH p IN ps
    IF first THEN
      first = FALSE
    ELSE
      out = out & ", "
    END IF
    out = out & toString(p)         ' selects FUNC toString(Point)
  NEXT
  RETURN out & "]"
END FUNC
```

---

# Part E — Errors, edge cases, and non-goals

## E.1 Errors

| Condition | Code |
|-----------|------|
| `FUNC error(...)` (or `SUB`) declared | `SYMBOL_RESERVED_BUILTIN_NAME` (new, definition-time) |
| `f(args)` matches neither the built-in nor any visible override | existing `TYPE_CALL_ARGUMENT_MISMATCH` |

## E.2 Lints (optional, non-blocking)

- `WARN_BUILTIN_SHADOWED` — a user overload whose parameter types are **already**
  handled by the built-in (it can never be selected under the gap-fill rule, §A.3.1):
  flag it as dead so the author is not surprised.
- `WARN_OVERRIDE_RESULT_TYPE` — a `toString`/`typeName` override not returning
  `String`, or a `len`/`to*` override not returning the built-in's conventional
  result type: allowed, but warned, to keep `toString` "renders to text" intuitive.

## E.3 Non-goals

- **Hijacking scalar/collection built-ins.** The built-in is authoritative for the
  types it supports; overrides only fill gaps (§A.3). No redefining `toString(Integer)`.
- **Overriding `error`** and any other language primitive (constructor syntax,
  `MATCH`, operators).
- **Overriding package-qualified members** (`strings::lower`, `collections::get`).
  Those are already user-shadowable by the freed-bare-name mechanism
  (plan-01-functions.md §5) and are a different namespace.
- **Operator overloading / `WITH`-style protocols.** Out of scope; this plan is
  scoped to the unqualified general functions only.
- **Implicit/structural defaults.** This plan does not add a structural `toString`
  for arbitrary records; it only routes a call to an override the author **wrote**.

These keep the feature a clean, zero-regression extension: every general built-in
becomes a name a program or package can **extend** for its own types, selected by
argument type, with the scalar built-ins untouched — exactly the shape `toString`
already demonstrated for `net::Url`.

---

# Part F — Return-type-distinguished overload resolution

A second, independent extension to overload resolution. Its first consumer is
`encoding::utf8Encode` (plan-02-encoding.md), which needs two overloads with the
**same** parameter list (`value AS String`) differing **only** in result type —
`List OF Byte` vs `List OF Integer`. Today this is illegal: `mfbasic.md` §6 states
"the return type is not part of the signature and never distinguishes an overload,"
so the second declaration is a `SYMBOL_DUPLICATE_TOP_LEVEL` error, and overload
resolution (`src/typecheck.rs` `lookup_visible_call_sig`) selects purely bottom-up
by argument count and positional argument types, never consulting the caller's
expected type. This part extends both, narrowly and back-compatibly.

## F.1 Declaration rule

The symbol identity for duplicate detection becomes **(name, ordered parameter
types, return type)**. Two declarations collide (`SYMBOL_DUPLICATE_TOP_LEVEL`) only
when all three match. Declarations that share a name and parameter types but differ
in return type are legal and form a **return-type overload set**. (Default values
still never distinguish an overload, unchanged.)

`src/resolver.rs` `insert_function` (the `candidate.params == params` check) adds
the return type to the comparison key; the stored `FunctionSymbol` gains the return
type it does not record today.

## F.2 Resolution rule

Resolution stays a filter, with one appended tie-break:

1. **Shape + positional types** — filter candidates by argument count, named-arg
   names, and positional argument types. (This is the same type-directed selection
   §A.4 / Phase 3 introduce for the override feature — the two extensions share one
   resolution path.)
2. If **one** candidate remains, bind it. *(Unchanged: every existing
   param-distinguished overload set resolves here and never reaches step 3, so the
   expected type stays optional for all of today's code.)*
3. If **more than one** remains — they necessarily differ only by return type —
   bind the unique candidate whose return type equals the call's **expected
   (contextual) type**.

The **expected type** is propagated to the call expression from:

- the declared type of the assignment / `LET` / `DIM` target
  (`LET b AS List OF Byte = encoding::utf8Encode(s)`);
- the declared parameter type of the argument slot when the call is itself a call
  argument (`crypto::hmacSha256(encoding::utf8Encode(s), …)` → the `key`
  parameter's `List OF Byte`);
- the enclosing function's declared return type when the call is a `RETURN` operand;
- the declared element/field type when the call initializes a typed collection
  element or record field.

## F.3 Ambiguity

If step 3 is reached and there is **no** expected type, or the expected type matches
**zero or more than one** candidate, the call is a compile error,
`TYPE_OVERLOAD_AMBIGUOUS` (a new typecheck diagnostic, peer of
`TYPE_CALL_ARGUMENT_MISMATCH` — **not** a `7-705` runtime code). The fix is a type
annotation that supplies the expected type, e.g.
`LET b AS List OF Byte = encoding::utf8Encode(s)`. Bare
`LET x = encoding::utf8Encode(s)` (no annotation, inferred local) is therefore an
ambiguity error by design.

## F.4 Composition and scope

- **Shared path with Parts A–E.** Both extensions modify the one resolution
  pipeline. Order: the general-built-in gap-fill (§A.3) decides built-in-vs-override
  first; ordinary positional-type filtering selects among same-name candidates; the
  return-type tie-break (§F.2.3) runs **last**, only when ≥2 candidates remain
  indistinguishable by parameters. A name is never simultaneously a gap-filled
  built-in and a return-type set in practice, so the rules do not interact.
- **SUBs** have no return type and are unaffected.
- Existing param-distinguished overloading, default arguments, and named arguments
  are unchanged — return type is consulted **only** as a final tie-break among
  otherwise-indistinguishable candidates; it adds no implicit conversions or
  candidate ranking.

## F.5 Implementation

- **`src/resolver.rs`** `insert_function`: add the return type to the
  duplicate-detection key; store it on `FunctionSymbol` (§F.1).
- **`src/typecheck.rs`**: thread the (already-available) `expected` type into
  `lookup_visible_call_sig` — presently ignored for `Call` expressions — and apply
  it as the step-3 tie-break among return-type-only candidates, reusing each
  candidate's `FunctionSig::return_type` (already carried, cf. Phase 4). Emit
  `TYPE_OVERLOAD_AMBIGUOUS` when there is no expected type or it does not uniquely
  match. Propagate `expected` from assignment/`LET`/`DIM` targets, call argument
  slots, `RETURN` operands, and typed element/field initializers.
- **`mfbasic.md` §6:** replace the "return type is not part of the signature"
  sentence with the §F.1/§F.2 rule, citing the `TYPE_OVERLOAD_AMBIGUOUS` diagnostic
  and the annotation fix.

## F.6 Tests (golden)

- A return-type overload set resolves to each member by annotation, by
  argument-slot type, and by `RETURN` context.
- The unannotated/inferred-`LET` case reports `TYPE_OVERLOAD_AMBIGUOUS`.
- Existing param-distinguished overloads and the Parts A–E built-in overrides are
  unchanged (no new ambiguity on any call that compiles today).
- Integration: `encoding::utf8Encode` selects `List OF Byte` vs `List OF Integer`
  (cross-referenced from plan-02-encoding.md).

## F.7 Errors

| Condition | Code |
|-----------|------|
| Two declarations share name + parameter types + return type | existing `SYMBOL_DUPLICATE_TOP_LEVEL` |
| ≥2 return-type candidates with no / non-unique expected type | `TYPE_OVERLOAD_AMBIGUOUS` (new, typecheck-time) |
