# bug-616: `mfb man` declarations show `AS Arg0` instead of the real return type (78 lines, `math` and `collections`)

Last updated: 2026-09-13
Effort: small
Severity: LOW
Class: Documentation (renderer)

Status: Open
Regression Test: none yet — see Phase 1

A descriptor can say "this overload returns the same type as argument n" with
`ParameterType::Arg(n)`. The renderer prints that internal placeholder literally,
so the reader sees a type that does not exist:

```
1. `math::abs(value AS List OF Integer) AS Arg0`
5. `math::abs(value AS Float) AS Arg0`
`math::atan2(y AS Fixed, x AS Fixed) AS Arg0`
`collections::reduceRight(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS Arg1`
```

A developer cannot write `Arg0`, and the page never says what it means.

**The single correct behavior a fix produces:** every declaration, Overloads line
and "Returns" sentence shows the concrete return type. For an overload with
concrete parameters that is `Arg(n)`'s parameter type (`math::abs(value AS Float)
AS Float`). For a generic parameter it is the type variable (`reduceRight(…) AS U`).
No rendered page contains `ArgN`.

Found by plan-125-C Phase 1 while reading `mfb man math atan2`. **Filed, not
fixed**, by user instruction during a documentation-only plan ("file all bugs,
make no fixes").

## Reproduction

```
./scripts/man-manual.sh > /tmp/manual.txt
grep -c -E '\b(AS|OF|TO) Arg[0-9]+\b' /tmp/manual.txt
# 78 at worktree-P-125 f011d27b9
grep -E '\b(AS|OF|TO) Arg[0-9]+\b' /tmp/manual.txt | grep -oE '`[a-zA-Z]+::' | sort | uniq -c
#  66 `math::    11 `collections::   (+1 wrapped reduceRight line)
```

Expected: `0`.

## Root cause

`src/types.rs:ParameterType::name` renders `ParameterType::Arg(n)` as
`format!("Arg{n}")`. `src/cli/man.rs:public_type_name`, which is
`ty.name().replace('.', "::")`, passes that through for every declaration and
return line (the `render_*` callers of `public_type_name(&implementation.return_type)`).
Nothing substitutes the n-th parameter's type before rendering.

## Non-goals

- Changing `ParameterType::name`'s output for diagnostics or IR, which other
  consumers may depend on (`src/types.rs` notes rendering stability for
  diagnostics). The fix belongs at the man-rendering layer.
- Changing any descriptor's `return_type`.

## Blast-radius audit

- Every `ParameterType::Arg(` in `src/codegen/builtins/**` (`math/mod.rs` shared
  helpers, `collections`). All render through `public_type_name`.
- Nested `Arg` inside `List OF Arg0` or `FUNC(...) AS Arg0`: the resolver must
  substitute recursively.
- HTML docs (`mfb doc`, `mfb pkg doc`) may share the rendering; check them.

## Fix

Phase 1 — renderer unit test: a descriptor with `return_type: Arg(0)` on a
`Float` parameter renders `AS Float` (RED); census 78 → 0 target. Commit:

Phase 2 — resolve `Arg(n)` against the overload's parameter list in the man
renderer, recursively (GREEN); update proven-wrong pins. Commit:
