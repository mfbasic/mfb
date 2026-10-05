# bug-882: `LET x = []` binds `List OF Unknown` silently, and the first `collections::append(x, v)` then fails the build with an internal error

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness (owner-confirmed: reject as ambiguous)

Status: Open
Regression Test: tests/rt-error/collections/unannotated-empty-list-binding (new) — name suggested

**Owner decision (2026-10-05):** an unannotated empty literal is ambiguous and must fail
to compile. `LET x = []` gets a located type diagnostic that it has no element type
(add an annotation: `LET x AS List OF Integer = []`), not a silent `List OF Unknown`
and not a later internal codegen error. `LET x = {}` must fail too. Measured today
(`/tmp/hardening/coord/eb`): `{}` is not an expression at all, so `LET x = {}` (and even
`LET x AS Map OF String TO Integer = {}` / `LET y AS Set OF Integer = {}`) already
fails, but with four cascading parse errors per line (`MFB_PARSE_EXPECTED_EXPRESSION`
x2, `MFB_PARSE_UNEXPECTED_TOKEN`, `MFB_PARSE_UNEXPECTED_STATEMENT`). The fix must not
start accepting a bare `{}`; the parse-error cascade is a separate, lower-priority
wording issue.

An unannotated empty list binding compiles with the internal element type `Unknown` —
`typeName(x)` prints `List OF Unknown`, a type name no program can write — and the natural next
step, `collections::append(x, 5)`, aborts the build with
`error: native collection list item must be Unknown, got Integer while lowering bind y AS List OF Unknown`
(no rule code, no location). bug-550 fixed `append([], 1)` inline and explicitly left this
binding-level residue open ("What is still `List OF Unknown`"); it is not tracked anywhere now.

Correct behavior (UNCLEAR — owner picks one): reject `LET x = []` with a located diagnostic asking
for an annotation (`TYPE_UNKNOWN_VALUE` / `TYPE_BINDING_REQUIRES_TYPE_OR_VALUE`-style), or infer the
element type from first use; in no case reach codegen with `Unknown` and fail there.

References:

- src/docs/spec/architecture/22_type-inference.md §"Bare List-Literal Synthesis": "if values empty → List(Unknown) ; only where there is no context at all".
- bugs/completed/bug-550-append-to-an-empty-list-literal-does-not-build.md §"What is still `List OF Unknown`".
- Outside B05's assigned topics; found while sampling `TYPE_UNKNOWN_VALUE` for diagnostics/rule-codes.

## Failing Reproduction

```
IMPORT io
IMPORT collections
FUNC main() AS Nothing
  LET x = []
  io::print(typeName(x))
  LET y = collections::append(x, 5)
  io::print(toString(len(y)))
END FUNC
```

- Observed: build fails `error: native collection list item must be Unknown, got Integer while lowering bind y AS List OF Unknown`; without the `append` lines it builds and prints `List OF Unknown`.
- Expected: a located compile diagnostic at line 4, or a working program printing `List OF Integer` / `1`.

## Root Cause

`src/ir/lower.rs:expression_type` synthesizes `List(Unknown)` for an empty literal with no expected
type and the unannotated `LET` freezes that as its static type; no verifier pass rejects an
`Unknown` element surviving into a binding, so the collection lowering
(`typed_list_element_type` → `collection_argument_as_list_slot`, per bug-550) meets it.

## Goal

- No program reaches codegen with a `List OF Unknown` binding.

## Blast Radius

- `LET e = []` then `collections::get(e, 0)` reports `argument type(s) (Unknown)` (bug-550 note) — same root.
