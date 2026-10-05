# bug-833: a parameter of user-template type (`Stack OF T`, `Pair OF A, B`) cannot be followed by another parameter — the spec's own §3 template example does not parse

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/generics/user-generic-param-not-last-rt (to add)

**Owner decision (2026-10-05):** change the language. A template with more than one type argument is written with a
parenthesized argument list everywhere it appears: `Pair OF (A, B)`, never `Pair OF A, B`.
This applies to the declaration (`TYPE Pair OF (A, B)`) and to every use site: parameters,
return types, `LET`/`MUT` annotations, record fields, function types, LAMBDA and LINK
parameter lists, and nested templates. It is not limited to function parameters. With
the list parenthesized, `FUNC push OF T(s AS Stack OF T, value AS T)` and
`FUNC(Box OF Integer, Integer)` are unambiguous. The old unparenthesized multi-argument
form becomes a located parse error. Single-argument templates (`Stack OF T`, `List OF T`)
and `Map OF K TO V` are unchanged. The spec (§3 templates, grammar), the man pages, `mfb fmt`,
and every existing fixture/package source using the old form are updated in the same change.

The first template example in the spec does not compile:
`FUNC push OF T(s AS Stack OF T, value AS T) AS Stack OF T`. Neither does any parameter
list where a user-template type (or the built-in `Pair OF A, B`) is followed by another
parameter, for example `FUNC f(p AS Pair OF Integer, String, b AS Integer)`. The type
parser reads a non-builtin template's `OF` argument list greedily across commas, so
`Stack OF T, value` is taken as two template arguments. The parameter list then never
closes:
`error[1-102-0005 MFB_PARSE_UNEXPECTED_TOKEN]: Function declarations must close the parameter list.`
After that, every following line cascades into `MFB_PARSE_UNEXPECTED_STATEMENT`.

The same greedy read affects lambda parameter lists
(`LAMBDA(b AS Box OF Integer, x AS Integer)` gives a parse error) and function types.
`FUNC(Box OF Integer, Integer) AS Integer` silently becomes a one-parameter type
`Box OF Integer, Integer`, and a later `TYPE_BINDING_MISMATCH` is the only symptom.

The limitation is known internally. It is recorded in
planning/completed/plan-105-B-one-type-grammar.md (Correction 9: "A parameter of
user-generic type must come LAST in a parameter list") and in the comment in
tests/rt-behavior/generics/user-generic-single-param-rt/src/main.mfb. It was never
filed as a bug, and the language spec neither states the restriction nor follows it.

The single correct behavior: inside a parameter list (FUNC, LAMBDA, function type), a
`,` followed by `<identifier> AS` (or `RES <identifier>`) begins the next parameter and
ends the template argument list. `FUNC push OF T(s AS Stack OF T, value AS T)` parses
and runs. The alternative is a spec decision, which must then document the restriction
in §3 and fix the §3 example.

References:

- `src/docs/spec/language/03_templates.md` (the `push` example), §4.7 (`Pair OF A, B`).
- `src/docs/spec/language/19_grammar.md` (the type grammar, owned by B05; check its EBNF).
- planning/completed/plan-105-B-one-type-grammar.md Corrections 8 and 9.

## Failing Reproduction

```basic
IMPORT io
IMPORT collections
TYPE Stack OF T
  items AS List OF T
END TYPE
FUNC push OF T(s AS Stack OF T, value AS T) AS Stack OF T
  RETURN WITH s { items := collections::append(s.items, value) }
END FUNC
FUNC main() AS Nothing
  MUT s AS Stack OF Integer
  s = push(s, 1)
  io::print(toString(len(s.items)))
END FUNC
```
`mfb build /tmp/hardening/B01/repro/r_tparam`

- Observed: `main.mfb:6 error[1-102-0005 MFB_PARSE_UNEXPECTED_TOKEN]` … `Function declarations must close the parameter list.`, plus cascaded `MFB_PARSE_UNEXPECTED_STATEMENT` on lines 7–8.
- Expected: the program builds and prints `1`.

Also failing: `FUNC f(p AS Pair OF Integer, String, b AS Integer)` (r_tparam2), even
though `Pair` has a known arity of 2, and
`LET l = LAMBDA(b AS Box OF Integer, x AS Integer) -> x`.

Works (contrast): the template parameter placed last (`FUNC push OF T(value AS T, s AS Stack OF T)` → prints 2 after two pushes), `(a AS (Stack OF Integer), b AS Integer)` (grouped type), and `List OF T`/`Map`/`Set` in any position, because those have dedicated arity-1/`TO` arms.

## Root Cause

`src/ast/expr.rs:parse_type_name_inner`, the generic fall-through arm after the
`Map`/`List`/`Set`/`Thread` special cases:
```rust
let mut args = vec![self.parse_type_name()?];
while self.match_kind(TokenKind::Comma) {
    args.push(self.parse_type_name()?);
}
```
It consumes every `, <Type>` without checking whether the token after the comma begins a
new parameter. `Pair` has no special arm either. Callers are `src/ast/items.rs:parse_params`
and the lambda and function-type parameter parsers.

## Goal

- After a comma in a template argument list, stop if the next two tokens are
  `<identifier> AS` or the next token is `RES`. This applies to `Pair` and to user
  templates in FUNC, LAMBDA, and LINK parameter lists.
- In a function type (`FUNC(Box OF Integer, Integer)`) there are no names, so the
  ambiguity is real. Either keep greedy reading with documentation, or require grouping
  `(Box OF Integer)`. This is an owner decision; see Open Decisions.

### Non-goals

- `Holder OF Pairing OF Integer, String` nesting ambiguity (plan-105-B Correction 8) is
  out of scope.
- Do not change how `Map OF K TO V` / `List OF T` parse.

## Blast Radius

- `parse_type_name_inner` generic arm: fixed by this bug.
- Record field types are one per line, so they are unaffected.
- Function-type params: same hazard, separate decision.

## Fix Design

Add a one-token lookahead (`peek_n(1)` is `AS`) at the comma. The parser already peeks
for keywords, so this needs no new machinery.

## Open Decisions

- For function types, choose between grouping and documented greedy reading.
  The recommendation is to document the requirement for `( … )` grouping in §3 and in the
  grammar topic.

## Validation Plan

- An rt fixture with the §3 `push` example verbatim, the `Pair` case, and the lambda case.
  Then the full suite. Then update the user-generic-single-param-rt comment.

## Summary

This is a contained parser fix. The risk is any construct that relied on greedy reading.
Grep the corpus for templates with more than one argument.
