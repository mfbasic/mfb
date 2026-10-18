# bug-913: two template FUNCs sharing a name (or a template and a plain FUNC) — the earlier declaration is silently discarded, calls bind to the wrong body

Last updated: 2026-10-18
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Fixed (bs24-agent-h, bug-staging-24)
Regression Test: tests/runtime/rt_template_overloads.rs (`cargo test --release --test rt_template_overloads`)

## Cause (coordinator, bug-staging-24)

- Rule: templates overload by signature exactly like plain FUNCs (owner decision 2026-10-05): identity is name + ordered parameter shapes + return type (§6 Overloading); resolution picks by the call's argument types; a concrete plain overload wins over a template instantiation when both match exactly (state it in the spec); a true duplicate is `SYMBOL_DUPLICATE_TOP_LEVEL` naming the source symbol.
- Where: `src/monomorph/lower.rs:Monomorphizer::new` keys `function_templates` by bare name (a later template overwrites an earlier one) and `instantiate_function` never falls back to `resolve_overload`; the instantiation dedup key `name<args>` omits template identity.
- Fix: templates are stored per signature and enter the same overload resolution plain FUNCs do; the instantiation key includes the template's identity.

**Owner decision (2026-10-05):** templates overload by signature exactly like plain FUNCs. Identity is the name plus
the ordered parameter shapes plus the return type (§6 "Overloading"). Two template FUNCs with
different parameter shapes, or a template and a plain FUNC of the same name, are distinct
overloads, and resolution picks by the call's argument types (a concrete plain overload wins
over a template instantiation when both match exactly; state the tie-break in the spec). A true
duplicate (same shapes) is `SYMBOL_DUPLICATE_TOP_LEVEL`, naming the source symbol and never the
mangled name. Nothing is ever silently dropped. Fixtures: both template bodies run for their
own calls, and the plain overload beside a template is callable.

The language allows overloading by signature ("two declarations collide
(`SYMBOL_DUPLICATE_TOP_LEVEL`) only when *all three* match"). For template
functions the monomorphizer keeps one template per *name*: a second
`FUNC f OF T(...)` with a different parameter list silently replaces the first,
and a plain `FUNC f(...)` overload beside a template `f` is unreachable (the
template is tried first and its arity/unification failure is final). No
declaration-time diagnostic is emitted; identical-signature duplicates are
correctly rejected.

The dangerous case is silent: a call that matches the *discarded* template
type-checks against the surviving one and runs the wrong body.

**Correct behavior (owner decision):** either template overloading by signature
works like plain overloading, or a second template/plain declaration of a
template's name is rejected at declaration (`SYMBOL_DUPLICATE_TOP_LEVEL` or a
dedicated rule). Never a silent drop.

References:

- `src/docs/spec/language/06_functions.md` "Overloading" bullet (identity = name + ordered parameter types + return type).
- `src/docs/spec/architecture/12_monomorphization.md` "Functions — instantiate_function" step 1: "Look up the template by base name; non-template names return `None` (the call falls through to overload resolution)." — the spec is silent on a name with several templates, or a template plus plain overloads.

## Failing Reproduction

```
IMPORT io

FUNC f OF T(x AS List OF T) AS String
  RETURN "list"
END FUNC

FUNC f OF T(x AS T) AS String
  RETURN "scalar"
END FUNC

FUNC main AS Integer
  io::print(f([1]))
  RETURN 0
END FUNC
```
`/tmp/hardening/mfb build -q . && ./build/coll.out` (project /tmp/hardening/B11/coll)

- Observed: builds, prints `scalar`.
- Expected: `list` (the only exact structural match), or a declaration error.

Swapping the two declarations: `f([1])` prints `list` and `f(5)` fails with
`TYPE_CALL_ARGUMENT_MISMATCH: Call to `f` cannot infer template arguments from `Integer`.`

Arity variant:
```
FUNC f OF T(x AS T) AS String          ' RETURN "one"
FUNC f OF T(x AS T, y AS T) AS String  ' RETURN "two"
... io::print(f(1) & f(1, 2))
```
- Observed: `error[2-203-0022 TYPE_CALL_ARITY_MISMATCH]: … Call to `f$Integer` has 1 argument(s), expected 2 to 2.` — also leaks the mangled name `f$Integer` (the bug-697 class).

Template + plain overload:
```
FUNC f OF T(x AS T) AS String               ' "tmpl"
FUNC f(x AS Integer, y AS Integer) AS String ' "ov2"
... f(1, 2)
```
- Observed: `TYPE_CALL_ARITY_MISMATCH: Call to `f` has 2 argument(s), expected at most 1.` The plain overload can never be called.

Contrast: two identical `FUNC f OF T(x AS T) AS String` → `SYMBOL_DUPLICATE_TOP_LEVEL` (correct).

## Root Cause

`src/monomorph/lower.rs:Monomorphizer::new` stores templates in
`function_templates: HashMap<String, HirFunction>` keyed by bare name
(`function_templates.insert(function.name.clone(), function.clone())`), so a later
template overwrites an earlier one. `instantiate_function` looks up by that name
first and, on a hit, never falls back to `resolve_overload`. The resolver's
duplicate check compares full signatures, so neither shape is diagnosed. The
dedup key `name<args>` also omits template identity, so two templates instantiated
at the same args would share one concrete symbol even if both were kept.

## Goal

- The first repro either prints `list` or fails at declaration; no program silently runs the discarded body.

### Non-goals

- Mangled names of single-template programs must not change.

## Blast Radius

- `type_templates` in the same constructor is keyed the same way, but a second
  `TYPE Box OF T` is already `SYMBOL_DUPLICATE_TOP_LEVEL` (verified), so types are unaffected.

## Fix

`Monomorphizer::function_templates` keeps every template of a name (with its
declaring file) instead of the last one, and `instantiate_function` enters the same
overload resolution plain FUNCs do: a plain overload whose parameter types match the
call exactly wins (the call then resolves through `resolve_overload`); otherwise each
template is bound silently (`bind_template`, named arguments reordered per template)
and the applicable ones compete — one wins outright, several pick the most specific
(`most_specific`: `a` is at least as specific as `b` when `b`'s patterns bind `a`'s
parameter types), and a tie is `TYPE_OVERLOAD_AMBIGUOUS`. A name with ONE template and
no plain overload reports exactly as before (same rules, same messages). The
instantiation key carries the template's identity (ordered parameter types + return)
when a name has several, so two templates instantiated at the same arguments get two
symbols (`f$Integer`, `f$Integer$2`); a single template's key, and so every existing
mangled name, is unchanged.

Duplicates: the resolver's signature key compares a template's type parameters by
position (`ParameterType::rename_vars`), so `FUNC f OF T(x AS T)` beside
`FUNC f OF U(y AS U)` is `SYMBOL_DUPLICATE_TOP_LEVEL` naming `f`.

Also fixed (the doc's arity variant): a downstream arity diagnostic on a lone
template's call named the mangled `f$Integer`; each function instantiation now maps
back to the called name in the bug-697 render table (`template_spellings`).

RED (before): `rt_template_overloads` — 3 of the 4 behaviour tests failed (printed
`scalar` for `f([1])`; `TYPE_CALL_ARITY_MISMATCH` on the arity and plain-overload
programs), and `template_overload_diagnostics_name_the_source_function` failed on the
pre-fix compiler (the ambiguous program built). GREEN (after): 5/5.

Spec: `language functions` (Overloading), `architecture monomorphization`
(`instantiate_function`).

Goldens expected to move: none (single-template keys and symbols are unchanged; no
in-tree program declares two templates of one name or a template beside a plain FUNC
of its name).
