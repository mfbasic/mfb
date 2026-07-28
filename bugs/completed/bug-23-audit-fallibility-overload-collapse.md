# bug-23: `mfb audit` fallibility analysis keys functions by bare name, so overloads collapse and are mislabeled fallible/pure

Last updated: 2026-07-08
Effort: medium (1h–2h)

`src/audit/collect/source.rs::fallible_functions` (`:309-343`) builds
`functions: BTreeMap<&str, &Function>` keyed by `function.name` (`:311-317`). When
two user functions share a name (a supported overload — see MEMORY
`func-sub-overloading`), `insert` **overwrites**, so only one overload's body is
analyzed. The resulting `fallible: HashSet<String>` is keyed by bare name, and both
`collect_source` (`fallible.contains(&function.name)`) and `is_fallible_call`
(`fallible.contains(callee)`, `:468`) then apply that single verdict to **every**
same-named overload. Audit runs on the pre-monomorph `inputs.ast` (mod.rs `run()`
passes `ast`, not `concrete_ast`), so overloaded names are still present.

Result: for `FUNC parse(n AS Integer)` (pure) + `FUNC parse(s AS String)` that
calls `fs::read` (fallible), audit reports `parse` as pure-or-fallible depending
only on iteration/overwrite order, and callers of the fallible overload are not
marked fallible. The control-flow / fallibility section — which the module docs
claim "matches real build behavior" — is wrong.

The single correct behavior a fix produces: fallibility is computed per overload
(by the same identity monomorph uses), so each overload and its callers get the
correct fallible/pure verdict.

Severity MEDIUM: a silent wrong-output defect in the `mfb audit` tool (misleads a
security/compliance reviewer), not a compiler miscompile.

References:

- `src/audit/collect/source.rs:309-343` (`fallible_functions`; name-keyed
  `BTreeMap` at `:311-317`, name-keyed `fallible` set at `:320-342`).
- `src/audit/collect/source.rs:~66` (`collect_source` applies `fallible` by bare
  name), `:464-469` (`is_fallible_call` same).
- `src/audit/mod.rs` `run()` — audits `ast` (pre-monomorph), overloads present.
- MEMORY `func-sub-overloading` (overloading is supported, resolved by
  arity+positional types).
- Found during goal-01 review of `src/audit/**`.

## Failing Reproduction

```
FUNC parse(n AS Integer) AS Integer
  RETURN n
END FUNC
FUNC parse(s AS String) AS Integer
  RETURN len(fs::readText(s))   ' fallible
END FUNC
SUB main()
  LET x = parse("f.txt")        ' calls the fallible overload
END SUB
```

- Observed: `mfb audit` reports `parse` (and `main`) as pure-or-fallible depending
  on which `parse` body the name-keyed map retained — not per overload.
- Expected: the String overload (and `main`) are fallible; the Integer overload is
  pure.

Contrast: non-overloaded user functions are analyzed correctly; builtin
fallibility (`fs`/`io`/`json`/`net`/`thread`) is package-prefixed and unaffected.

## Root Cause

`fallible_functions` conflates distinct overloads under one map key and one result
key (bare name). The fixpoint therefore analyzes one body per name and broadcasts
its verdict to all overloads and their call sites.

## Goal

- Fallibility is tracked per overload identity (name + arity + param types), so
  each overload and its callers receive the correct verdict.

### Non-goals (must NOT change)

- Builtin fallibility classification (package-prefixed, correct).
- The set of packages considered inherently fallible.

## Blast Radius

- `fallible_functions`, `collect_source`, `is_fallible_call` — all key on bare
  name. Fixed together by switching to a disambiguated identity, or by auditing the
  monomorphized AST.

## Fix Design

Key the function map and `fallible` set by the same mangled identity monomorph uses
(name+arity+param types), and resolve each call site to its concrete overload
before checking membership. Alternatively, run audit on the monomorphized AST where
overloads are already distinct — but that changes what audit reports (concrete
instantiations vs. source functions), so the per-overload keying is preferred.

## Phases

### Phase 1 — failing test + audit

- [ ] Add an audit fixture with a pure + fallible overload pair; assert only the
      fallible overload and its callers are marked fallible. Confirm it fails today.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Switch `fallible_functions`/`is_fallible_call`/`collect_source` to overload
      identity keys and per-call overload resolution.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh`; audit goldens updated only where overloads were
      previously mislabeled.

## Validation Plan

- Regression test(s): the overloaded-fallibility audit fixture.
- Runtime proof: `mfb audit` on the reproduction marks the right overload fallible.
- Full suite: `scripts/test-accept.sh`.

## Summary

Audit's fallibility fixpoint collapses overloads to one bare-name verdict; the fix
keys on overload identity (or audits the monomorphized AST). Risk is matching the
compiler's overload-resolution so the audit verdict tracks the real callee.
