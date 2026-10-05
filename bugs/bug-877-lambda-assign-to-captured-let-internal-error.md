# bug-877: a lambda assignment body targeting a captured `LET` reaches the NIR validator as an internal error

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/rt-error/... (new) — name suggested

The grammar's assignment-bodied lambda (`LAMBDA(a AS Integer) -> n = a`) whose target `n` is a
captured immutable `LET` is not diagnosed by the verifier. The build instead dies late with the
unstructured `error: NIR assignment targets immutable local 'n'` — no rule code, no source location.
The same happens inside the sanctioned non-escaping position
(`collections::forEach(xs, LAMBDA(x AS Integer) -> total = total + x)` with `LET total`).

Correct behavior: a located `TYPE_ASSIGN_REQUIRES_MUT` ("Binding `n` is immutable and cannot be
assigned"), exactly what the same lambda assigning its own parameter already reports.

References:

- src/docs/spec/language/19_grammar.md `lambda` production: "the `ident = expr` body mutates a captured MUT binding and yields Nothing".
- src/docs/spec/language/06_functions.md (Closures: capture `LET` by value; `MUT` only in the forEach non-escaping position).
- Found during B05 hardening (grammar + diagnostics sampling).

## Failing Reproduction

```
FUNC main() AS Nothing
  LET n = 5
  LET f = LAMBDA(a AS Integer) -> n = a
END FUNC
```

- Observed: `error: NIR assignment targets immutable local 'n'` (build rc=1, no diagnostic header).
- Expected: `main.mfb:3 error[2-203-0048 TYPE_ASSIGN_REQUIRES_MUT] ...`.

Contrast (measured): `MUT n` + same lambda → `TYPE_LAMBDA_CAPTURE_UNSUPPORTED` (correct);
`LAMBDA(a AS Integer) -> a = 3` → `TYPE_ASSIGN_REQUIRES_MUT` (correct); a top-level `LET g` as the
target → `TYPE_UNKNOWN_VALUE` "Assignment target `g` is not a local binding" (located, though
misleading); `MUT total` under `collections::forEach` compiles and runs (`6`).

## Root Cause

The mutability check for an assignment target runs on the lambda's own scope (parameters, own
bindings); a captured outer binding is not looked up, so nothing rejects it until
`src/target/shared/validate/body.rs` (the NIR validator, "NIR assignment targets immutable
local") — a backend invariant check whose message is not a diagnostic.

## Goal

- An assignment-bodied lambda whose target resolves to a captured immutable binding (local or top-level `LET`) is rejected with a located `TYPE_ASSIGN_REQUIRES_MUT`.

### Non-goals (must NOT change)

- The forEach by-reference `MUT` capture keeps working; escaping `MUT` capture stays `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`.

## Blast Radius

- Top-level `LET` target case (`TYPE_UNKNOWN_VALUE` "not a local binding") — same family, wrong rule; fix together.
