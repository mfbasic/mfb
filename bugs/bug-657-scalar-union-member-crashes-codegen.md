# bug-657: a `UNION` with a scalar member passes the verifier and dies in codegen

Last updated: 2026-09-19
Effort: small
Severity: MEDIUM
Class: Correctness (missing diagnostic)

Status: Open
Regression Test: none yet — see Phase 1

`UNION Shape` with `Integer` and `String` as members is accepted by the semantic verifier
and then fails in code generation with an internal error:

```
error: native code union wrap member 'Integer' is not a record while lowering bind sh AS Shape
```

The rule that should have caught it already exists — `TYPE_UNION_MEMBER_REQUIRES_TYPE`
(`2-203-0064`), whose message is *"union members must name concrete TYPE declarations"* —
but its enforcement only rejects a member that is a declared **union or enum**, never a
built-in scalar. The spec agrees with the rule, not with the code: §4.3's union members
are all record `TYPE`s, and `MATCH` reaches a payload that "members need not share a
field", which presumes a record.

**The single correct behavior a fix produces:** the program below is rejected at the
`UNION` declaration with a located `TYPE_UNION_MEMBER_REQUIRES_TYPE` naming the offending
member, and no program that builds today changes.

References:

- `src/ir/verify/types.rs:107-125` — the enforcement loop. Its own comment says "Each
  named member must be a concrete TYPE (record)", then tests only
  `self.unions.contains_key` / `self.enums.contains_key`; a name that is neither — a
  built-in scalar — falls through as acceptable.
- `src/rules/table.rs:705` — the rule and its message.
- `src/docs/spec/language/04_types.md` §4.3 — every documented union member is a record
  `TYPE`.
- Found while enumerating thread message types for bug-650 Phase 1 (the sweep needed a
  data union and reached for the scalar spelling first).

## Failing Reproduction

```
IMPORT io

UNION Shape
  Integer
  String
END UNION

SUB main()
  LET sh AS Shape = 5
  io::print("ok")
END SUB
```

`mfb build --debug` at `3d49a969e`, macOS aarch64:

- Observed: `error: native code union wrap member 'Integer' is not a record while lowering
  bind sh AS Shape`, exit non-zero. No file, no line, no rule code.
- Expected: `TYPE_UNION_MEMBER_REQUIRES_TYPE` at the `UNION` declaration, naming `Integer`.

Verified on a clean main-tip compiler. The record-member spelling of the same union
(`UNION Shape` over two `TYPE`s) builds and sends across a thread boundary fine, so this is
specific to the scalar member.

## Root Cause

Localized (above): the verifier's member check is written as a blocklist of the two
*declared* kinds it knew about rather than an allowlist of "is a record". Anything the
model does not have in `self.records` — every built-in scalar, `String`, a collection
spelling — passes. Codegen then reaches `union wrap member … is not a record` and has no
diagnostic vocabulary left.

## Goal

- The reproduction is rejected with the existing located rule.
- Every union that builds today still builds: the check must be "member is a record",
  computed the same way codegen's `is not a record` test is, so the two cannot drift.

### Non-goals (must NOT change)

- `INCLUDES` member merging, resource-union variants, or `TYPE_UNION_INCLUDE_REQUIRES_UNION`.

## Blast Radius

- Every `UNION` declaration in `tests/`, `examples/` and the built-in packages must still
  pass. No fixture uses a scalar member today (`grep` over `tests/**/main.mfb` for a union
  body line naming a scalar → 0 hits), so the new rejection should be inert on the tree —
  confirm by running the full suite, not by reading.
- Diagnostic goldens: a new rejection adds a `build.log` line wherever a fixture trips it.
  Expected to be none, per the grep above.

## Phases

### Phase 1 — failing test + audit

- [ ] Syntax fixture asserting the located diagnostic; confirm RED (today it builds past
      the verifier and dies in codegen).
- [ ] Confirm no existing union in the tree is newly rejected.

Commit: —

### Phase 2 — the fix

- [ ] Make the member check an allowlist ("is a record"), sharing codegen's predicate.

Commit: —

### Phase 3 — full validation

Commit: —

## Summary

The diagnostic for this exact mistake already exists; its enforcement tests the wrong
predicate, so the error surfaces as an internal codegen failure instead.
