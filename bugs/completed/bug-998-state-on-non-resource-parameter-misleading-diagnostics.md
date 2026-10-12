# bug-998: `STATE` on a non-resource parameter is not rejected where it is declared; the user sees unrelated `TYPE_UNKNOWN_VALUE` / `TYPE_STATE_MISMATCH` errors instead

Last updated: 2026-10-06
Effort: small (<1h)
Severity: LOW
Class: Other (misleading diagnostic)

Status: Fixed (bs19-agent-a)
Regression Test: `tests/syntax/resources/param-state-on-non-resource-invalid`, `tests/syntax/resources/bug998_state_requires_resource_invalid`, `tests/syntax/native/bug998_native_state_requires_resource_invalid`, `ir::verify::tests::rejects_state_on_a_non_resource_at_every_position`, `ir::verify::tests::res_on_a_non_resource_is_still_the_res_rule`, `hir::resource_params::tests::a_stray_state_clause_is_moved_aside_only_with_imports_folded_in`

A `STATE` clause is only meaningful on a resource (`mfb man types`: a resource carries "its own
state with a trailing STATE S"). On a parameter whose type is not a resource, nothing rejects
the clause at the declaration. Instead the parameter gets a type no expression can use, and
the user sees errors that do not name the cause.

**Owner decision (2026-10-06): add a dedicated rule**, `TYPE_STATE_REQUIRES_RESOURCE` (an error,
code assigned in the registry), reported once, located at the parameter's `STATE` clause:
"STATE can only be attached to a resource". Register it in the diagnostics spec (rule-codes and the
error-codes Constant Registry) in the same change, per `.ai/compiler.md`. Do not reuse
`TYPE_STATE_INVALID`: that code means "STATE must be a copyable, defaultable data type".

The question as filed:

**Correct behavior (proposed):** one located error at the parameter, saying that `STATE` needs
a resource type. The recommendation is to reuse `TYPE_STATE_INVALID`, the existing code for an
invalid `STATE` use (bug-975 reuses it for `t.state.x` on a worker without `STATE`), rather
than adding a new rule. **Owner decision needed:** reuse `TYPE_STATE_INVALID`, or add a
dedicated rule.

## Failing Reproduction

Measured by the coordinator on 2026-10-06. "main" is 468ff0f94 (`/tmp/main-dbg-target/release/mfb`),
where a parameter needed the `RES` spelling. "bs17-agent-e" is bug-976's compiler, where `RES` on
a parameter is gone and `x AS Integer STATE Cur` parses. The result is the same on both.

```
IMPORT io

TYPE Cur
  n AS Integer
END TYPE

FUNC f(x AS Integer STATE Cur) AS Integer     ' main: FUNC f(RES x AS Integer STATE Cur)
  RETURN x + 1
END FUNC

FUNC main AS Integer
  io::print(toString(f(41)))
  RETURN 0
END FUNC
```

- With the call: `st/src/main.mfb:12 error[2-203-0129 TYPE_STATE_MISMATCH] … Argument 1 for 'f'
  is declared 'STATE Cur' but carries no STATE; a parameter cannot attach one — declare
  'STATE Cur' on the owning binding.` That advice is impossible to follow for an `Integer`.
- Without the call: `st/src/main.mfb:8 error[2-203-0043 TYPE_UNKNOWN_VALUE]: … RETURN value does
  not have a known type.` on `RETURN x + 1`.

Expected: a single error at line 7 (the parameter) saying that `STATE` requires a resource type.

## Related

- Found by bs17-agent-e while fixing bug-976 (`RES` removed from parameter lists). That change
  makes the `x AS Integer STATE Cur` spelling parse, so the gap is easier to reach than before.
  The diagnostics themselves are unchanged from main.

## Resolution

`TYPE_STATE_REQUIRES_RESOURCE` (2-203-0145, "STATE can only be attached to a resource"),
registered in `src/rules/table.rs` and `mfb spec diagnostics rule-codes`. The error-codes
Constant Registry is the *runtime* `errorCode::` table and has no row for any compile-time
rule (`grep -c '2-203-' src/docs/spec/diagnostics/02_error-codes.md` is 0), so nothing was
added there.

**Why the parameter cascaded, and what stops it.** The clause rode into the parameter's
type: the base compiler's own message for the repro is "Operator `+` requires numeric
operands, got Integer STATE Cur and Integer". No expression can use that type, hence
`TYPE_UNKNOWN_VALUE` at each use and `TYPE_STATE_MISMATCH` at each call.
`hir::resource_params` now takes the clause off a parameter or a return whose type is not
a resource (`ResourceTypes::take_stray_state`, into `HirParam::stray_state` and
`HirFunction::return_stray_state`). It does so only on the build path's second derivation,
where the imported packages' resources are folded in and "not a resource" is an answer
rather than "not one the sources declare". The parameter then types as the plain value it
is, and `ir::shape::check_stray_state` reports the clause once, at the declaration. A
lambda's parameter is covered by the same code, and so is a generic's parameter once it is
instantiated (`FUNC gen OF T(x AS T STATE Cur)` called with an `Integer` reports at the
declaration).

**Every other position** keeps the clause in the IR, so `ir::verify` reports it there, and
for decoded package IR, from the walk bug-1013 added (`check_state_clauses`): a local `RES`
binding, a record field, a collection element or map value, a thread's resource plane, and
a native `LINK` parameter and return. A top-level binding is reported from `ir::shape`,
since lowering drops its clause. None of these was reported by anything on the base
except the local binding, which reported `TYPE_RES_REQUIRES_RESOURCE` for its `RES` marker.

**Coherence with bug-1013.** One clause reports at most one of the two rules, in this
order, and the order is in `mfb spec language resource-management`:

1. `TYPE_STATE_REQUIRES_RESOURCE`: the host is not a resource. The clause's type is not
   examined: `x AS Integer STATE Mode` (an `ENUM`) reports this rule only.
2. `TYPE_STATE_INVALID`: the host is a resource and the type is not a copyable,
   defaultable record `TYPE`.

Only a type positively known to be data is refused; a name no table can place may be
another package's resource (`state_host_is_data`).

**The `RES` marker's rule is left alone.** `RES bound AS Integer STATE Cur` reported
`TYPE_RES_REQUIRES_RESOURCE` before this fix and still does; `TYPE_STATE_REQUIRES_RESOURCE`
is reported beside it, for the clause
(`ir::verify::tests::res_on_a_non_resource_is_still_the_res_rule`, and line 70 of
`bug998_state_requires_resource_invalid`). Removing an existing diagnostic is not something
this document settles, so it was not done. See "Decision left to the owner".

**RED before the fix** (compiler built from the parent commit, which has bug-1013):
`cargo test --release --bin mfb -- ir::verify::tests::rejects_state_on_a_non_resource_at_every_position
ir::verify::tests::res_on_a_non_resource_is_still_the_res_rule
ir::verify::tests::imported_types_are_judged_by_their_kind` fails all three, and
`scripts/test-accept.sh target/release/mfb <dir> 'bug998_*' param-state-on-non-resource-invalid`
mismatches all three fixtures, with none of the 16 expected
`TYPE_STATE_REQUIRES_RESOURCE` diagnostics reported. After the fix all pass.

### Decision left to the owner

Whether a declaration that is wrong on both counts, `RES n AS Integer STATE Cur`, should
report one diagnostic or two. It reports two today (the marker's and the clause's). The
previous agent's parked work suppressed `TYPE_RES_REQUIRES_RESOURCE` there, and also made
`res_axis_slot` peel a slot's `STATE` clause before consulting its tables, which changes
which declarations the `RES` marker rules fire on. Both were dropped as contract changes
beyond this document.

### Found and not fixed (reported to the coordinator)

1. **`res_axis_slot` does not peel a slot's `STATE` clause.** A `RES Integer STATE Cur`
   record field or collection element is not reported by `TYPE_RES_REQUIRES_RESOURCE`
   (the local binding of the same type is), because the slot type asked about is
   `Integer STATE Cur`, clause included, which is not provably data. After this fix the
   declaration is an error anyway (`TYPE_STATE_REQUIRES_RESOURCE`), so the gap no longer
   lets a program through.
2. **A generic parameter `x AS T STATE Cur` instantiated with a stateful resource
   doubles the clause.** `FUNC gen OF T(x AS T STATE Cur)` called with a
   `fs::File STATE Cur` value reports `TYPE_STATE_MISMATCH` "Argument 1 for
   `gen$fs$File$STATE$Cur` is declared `STATE Cur STATE Cur` but carries STATE `Cur`".
   Same on the base compiler.

## Checks run

- `cargo test --release --bin mfb -- ir::verify:: ir::shape:: rules:: spec docs:: hir::
  ir::resource_escape:: monomorph:: resolver::`: 840 passed, 0 failed.
- `cargo test --release --test no_type_strings --test no_operator_strings --test
  architecture_guards`: passed.
- `scripts/test-accept.sh target/release/mfb <dir> <names>` over the 438 fixtures that
  spell `STATE`, live under a `resources`, `threads`, `http`, `native`, `packages`, `net`,
  `tcp` or `audio` directory, or import `http` or `thread`: the 8 `build.log` wording
  mismatches bug-1013 lists and nothing else.
- `scripts/spec-census.sh --render` (0 leaks) and `--citations` (0 missing).

## Golden impact

None for existing fixtures: no fixture in the tree wrote a `STATE` clause on a
non-resource. The 438-fixture run above shows only bug-1013's eight wording changes.
