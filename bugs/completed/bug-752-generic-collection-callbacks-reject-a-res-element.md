# bug-752: `groupBy`/`partition`/`sortBy`/`findIndex`/… reject a callback over a `List OF RES` element

Last updated: 2026-09-27
Effort: small
Severity: MEDIUM
Class: Correctness (valid program rejected)

Status: Fixed
Regression Test: `tests/rt-behavior/collections/bug752-res-element-callbacks-rt`;
unit test `monomorph::helpers::tests::unify_agrees_modulo_the_res_marker`

Found by the bug-643 lane while auditing every `collections` helper over a
`List OF RES fs::File`. Every source-generic member that takes a callback over the
element fails to build when the element is a `RES` handle:

```basic
FUNC rank(RES x AS fs::File) AS Integer
  RETURN 1
END FUNC
...
LET files AS List OF RES fs::File = [a, b]
LET sorted = collections::sortBy(files, rank)
```

```
error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: function call argument type does not match parameter type
    Call to `__collections_sortBy` cannot infer template arguments from `FUNC(fs.File) AS Integer`.
```

Same for `groupBy`, `partition`, `findIndex`, `findLastIndex`, `all` and `any`
(measured at `a0082e4ae`). The native callback members (`filter`, `transform`) were
fine. §15.6 makes a resource collection an ordinary copyable collection of pointers,
so nothing forbids these calls.

**The single correct behavior:** a callback over a `RES` element (`FUNC(RES x AS
fs::File)`, or a `LAMBDA(RES x AS fs::File)`) is accepted by every collections member
that takes one, the call runs, and the handles stay open.

## Root cause

Three layers disagreed about the `RES` ownership marker, which `ParameterType`
documents as RES-*transparent* for matching:

1. `monomorph::helpers::unify_type`: `List OF T` against `List OF RES fs.File` binds
   `T := RES fs.File`; the callback `FUNC(T) AS K` against `FUNC(fs.File) AS Integer`
   then compares the bound `RES fs.File` with `fs.File` by plain equality and fails.
2. `ir::verify::compat::compatible`: after instantiation the body's parameter is
   `FUNC(RES fs.File) AS Integer`, and the call's argument `FUNC(fs.File) AS Integer`
   was compared structurally only for lists/maps/results; a `Func` fell to the
   name comparison and mismatched.
3. `partition`'s source body bound each element with `LET item AS T = get(value, i)`,
   which `TYPE_RESOURCE_REQUIRES_RES` rejects for a resource `T` (the diagnostic that
   surfaced once 1 and 2 were fixed, reported at the call thanks to bug-642).

## Fix

- [x] `unify_type` compares a second binding modulo the top-level `RES` marker and
  keeps the marked one; `compatible` compares callable parameters modulo it;
  `partition` reads each element where it appends it instead of binding it. RED at
  `a0082e4ae` (the fixture fails to build with seven `TYPE_CALL_ARGUMENT_MISMATCH`).
  Commit: e4aa704b9

STATUS: FIXED (e4aa704b9, goldens e703c61e3). Gates on the merged tip (82926917c + e703c61e3): `cargo test --release --no-fail-fast` 251 binaries, 6311 passed, 0 failed; `scripts/artifact-gate.sh target/release/mfb all` 0 diffs over 2176 goldens.
