# bug-915: a non-predicate built-in (`toString`, `len`, `io::print`) passed as a callback argument fails with the unlocated internal error `NIR local reference '…' does not resolve`

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Other (internal error leaked to the user)

Status: Open
Regression Test: tests/syntax/functions/builtin-as-callback-argument-invalid (new) — name suggested

Naming a general built-in other than the `is*` predicates in a value position is
not supported, and a `LET` binding of one gets a located `TYPE_UNKNOWN_VALUE`. But
the same name passed as an argument to a **user function's** `FUNC`-typed
parameter, or to a built-in callback such as `collections::forEach`, passes every
front-end pass and fails in native lowering with an unlocated internal message and
exit 1 — no file, line, or rule code.

**Correct behavior:** a located diagnostic at the argument (e.g.
`TYPE_UNKNOWN_VALUE` / `TYPE_CALL_ARGUMENT_MISMATCH`), as `LET f AS FUNC(Integer) AS String = toString`
already gets — or, if built-ins are meant to be values, a working program.

References:

- `src/docs/spec/architecture/12_monomorphization.md` "Function-value references": for an overridable built-in name with no selected override "the bare name stays for IR lowering's built-in reference handling, and nothing is reported here."
- `src/docs/spec/language/06_functions.md`: "A general built-in predicate named as a value … is selected by a separate mechanism in IR lowering" — only the predicates are values.
- Same family as bugs/completed/bug-368 (the `is*` predicates hit the identical NIR message before that fix).

## Failing Reproduction

```
IMPORT io

FUNC apply(f AS FUNC(Integer) AS String, v AS Integer) AS String
  RETURN f(v)
END FUNC

FUNC main AS Integer
  io::print(apply(toString, 5))
  RETURN 0
END FUNC
```
`/tmp/hardening/mfb build -q .` (project /tmp/hardening/B11/coll)

- Observed: `error: NIR local reference 'toString' does not resolve` (exit 1, no location).
- Expected: a located front-end diagnostic (or `5`).

Same message for:
- `FUNC applyL(f AS FUNC(List OF Integer) AS Integer, v AS List OF Integer)` … `applyL(len, [1,2])` → `NIR local reference 'len' does not resolve`;
- `collections::forEach(["a"], io::print)` → `NIR local reference 'io.print' does not resolve`.

Contrasts:
- `LET f AS FUNC(Integer) AS String = toString` → `./src/main.mfb:4 error[2-203-0043 TYPE_UNKNOWN_VALUE]` (located, correct shape).
- `collections::transform([1, 2], toString)` → located `TYPE_CALL_ARGUMENT_MISMATCH`.
- With a user override `FUNC toString(p AS P) AS String`, `applyP(toString, P[2])` selects the override and prints `P(2)` (works).

## Root Cause

Hypothesis (ordered): (1) `src/monomorph/lower.rs:resolve_builtin_override_reference`
leaves the bare built-in name and nothing downstream rejects it in a user-call or
built-in-callback argument slot — `ir::shape`/`ir::verify` treat the identifier's
type as matching the expected `FUNC` type (no `TYPE_UNKNOWN_VALUE` for arguments),
and IR lowering emits `IrValue::Local { name: "toString" }` — **confirmed** by
`mfb build --ir`: `"target": "apply", "args": [{ "kind": "local", "name": "toString" }, …]`
(a `local` that no `bind` introduces, which the IR verifier accepts); (2) NIR lowering
(`src/target/shared/nir/`) then fails to find a local of that name. The `LET` path
is caught because binding-initializer typing reports the unknown type.

## Goal

- Each repro produces a located diagnostic with a rule code and exits 1; no "NIR …" message reaches the user.

## Blast Radius

- Any general built-in name (`len`, `toString`, `typeName`, …) and qualified package members (`io::print`) in callback-argument position — three reproduced.
