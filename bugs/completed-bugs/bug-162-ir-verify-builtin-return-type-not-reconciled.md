# bug-162 — IR verifier trusts a builtin `Call`'s annotated result type without reconciling it → member-confusion on the untrusted `.mfp` path

Last updated: 2026-07-12
Severity: MEDIUM — crafted package can fabricate a record return type on a builtin call, defeating the member-access type check.
Class: Memory-safety / Security (trust boundary).
Status: FIXED
Resolution: `check_call_result_type` now reconciles a builtin `Call`'s annotation
against `builtins::resolve_call_return_type(target, arg_types)` — the same
arg-typed oracle the monomorphizer uses (now shared, single-source). Added
`check_union_extract` mirroring `check_union_wrap` for the `UnionExtract` read
path. Acceptance (929 tests) stays green — the reconciliation never false-rejects
legitimately-compiled IR because the front end produced the annotation from the
same resolver.

## Finding

`src/ir/verify/mod.rs:3268` (`check_call_result_type`) reconciles a `Call`'s
`annotated_type` against the callee's declared `returns` only for *internal*
functions — it returns early when `self.functions.get(target)` is `None`, which
is every builtin ("Builtins have no `FnSig` and are skipped"). `infer_type`
(`:3922-3938`) then falls through to `usable_type(value.annotated_type())` and
hands the fabricated type to `check_member_access`. `check_builtin_call_args`
validates *argument* types via `resolve_call` but never the *return* type. So a
crafted `.mfp` can set a builtin call's annotation to a foreign record and pass a
member access that codegen then services by reading that record's layout off, say,
an `Integer`. The same unreconciled-annotation pattern applies to
`ResultValue`/`ResultError`/`UnionExtract` (the `check_value` arms at
`:1423-1428` only recurse into the inner value; there is no `check_union_extract`
mirroring `check_union_wrap`).

## Trigger

A crafted package (package path) with a `Call`/`CallResult` to a builtin whose
real return is a primitive — e.g. `strings.length(x)` returning `Integer` — but
whose `annotated_type` is set to a record `Account`, followed by
`MemberAccess { member: "owner" }`. `check_member_access` finds `owner` on the
real `Account` record and passes; codegen reads `Account`'s layout off an
`Integer`. Reachable before any signature is trusted. Confidence medium (needs a
handcrafted `.mfp`; narrows the module's stated "member-confusion class is
checked completely on the package path" claim).

## Fix

In `check_call_result_type`, when `target` is a builtin, derive the expected
return from `builtins::*::resolve_call(target, &arg_types)` (already computed in
`check_builtin_call_args`) and reconcile the annotation against it. Add an
analogous variant-type reconciliation for `UnionExtract`
(`check_union_extract`). Add a malformed-package fixture.
