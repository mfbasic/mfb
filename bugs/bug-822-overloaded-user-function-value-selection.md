# bug-822: annotated overloaded user function values report an undeclared identifier

Last updated: 2026-10-04
Effort: medium (guess; includes contextual propagation and imported-name audit)
Severity: MEDIUM
Class: Correctness
Status: Open
Regression Test: proposed — `tests/runtime/rt_overloaded_function_values.rs`, plus
valid/invalid fixtures under `tests/rt-behavior/functions/` and `tests/syntax/functions/`.

A declared overloaded user function cannot be selected as a value by a complete
`FUNC` annotation: the compiler reports that its name is undeclared. The correct
outcome is to select the unique declaration matching the complete expected
callable type, including its return type, and execute that declaration. An
ambiguous or incompatible reference must receive a source-level overload/type
diagnostic, not an undeclared-identifier diagnostic for a declared name.

Filed during the functions/templates/packages specification review. This is a
compiler fix work order; the accompanying documentation change describes the
current limitation and does not implement the feature.

References:

- `src/docs/spec/language/06_functions.md` — overload identity and function values.
- `src/docs/spec/architecture/12_monomorphization.md` — overload rewriting.
- `src/monomorph/lower.rs:Monomorphizer::lower_expression`.
- `src/ir/lower.rs:lower_expression_with_expected`.

## Failing Reproduction

From the repository root, build the current compiler, then run:

```sh
cargo build --bin mfb
python3 - <<'PY'
import json, pathlib, subprocess, tempfile
root = pathlib.Path(tempfile.mkdtemp(prefix='mfb-overload-value-'))
(root / 'src').mkdir()
(root / 'project.json').write_text(json.dumps({
    'name': 'overload_value', 'version': '0.1.0', 'mfb': '1.0',
    'kind': 'executable', 'sources': [{'root': 'src'}],
    'entry': 'main', 'targets': ['native']
}))
base = '''IMPORT io
FUNC pick(x AS Integer) AS Integer
  RETURN x + 1
END FUNC
'''
main = '''FUNC main() AS Nothing
  LET f AS FUNC(Integer) AS Integer = pick
  io::print(toString(f(4)))
END FUNC
'''
for label, extra in [
    ('parameter', 'FUNC pick(x AS String) AS Integer\n RETURN len(x)\nEND FUNC\n'),
    ('return', 'FUNC pick(x AS Integer) AS String\n RETURN toString(x)\nEND FUNC\n'),
    ('single-control', '')
]:
    (root / 'src/main.mfb').write_text(base + extra + main)
    result = subprocess.run(['target/debug/mfb', 'build', str(root)],
                            text=True, capture_output=True)
    print(label, 'build exit', result.returncode)
    print(result.stdout + result.stderr)
    if result.returncode == 0:
        executable = next(line.removeprefix('Wrote executable to ')
                          for line in result.stdout.splitlines()
                          if line.startswith('Wrote executable to '))
        run = subprocess.run([executable], text=True, capture_output=True)
        print('run exit', run.returncode, 'stdout', repr(run.stdout))
PY
```

Observed on macOS AArch64 with the debug compiler (`python3
/tmp/mfb-spec6-probe.py`, followed by the standalone reproduction above): both
`parameter` and `return` builds exit 1 with:

```text
error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: identifier could not be resolved
Identifier `pick` is not declared in this scope.
```

Expected: both build successfully and the resulting executable prints `5\n`
and exits 0. The single-declaration control already does so. Additional measured
controls (`python3 /tmp/mfb-spec6-probe.py`): a direct `pick(4)` call and a value
`LAMBDA(x AS Integer) -> pick(x)` each build and print `5\n`; an untyped overloaded
reference also reports `SYMBOL_UNKNOWN_IDENTIFIER` (it should instead diagnose
ambiguity). Other hosts are UNMEASURED; the fault is in shared front-end code.

## Root Cause

`Monomorphizer::new` records overloaded declarations under mangled symbols;
`Monomorphizer::lower_expression` rewrites `HirExpression::Call` through
`resolve_overload`, but its `HirExpression::Identifier` arm clones the original
name unchanged (`rg -n 'resolve_overload|HirExpression::Identifier|overload_names'
src/monomorph/lower.rs`). The subsequent resolver therefore sees `pick` after
its declarations have become concrete mangled names. `lower_expression_with_expected`
in `src/ir/lower.rs` has contextual selection for built-in predicate references,
but its ordinary function lookup is by name, not by an overload set (`sed -n
'4780,4875p' src/ir/lower.rs`). The expected callable type never selects the user
reference. Fixing only that late lookup would miss the earlier resolver failure.

## Goal and Non-goals

- Select a unique overload using ordered parameter types, return type and the
  callable's isolation requirement wherever a concrete expected callable type
  is available; preserve exact type matching.
- Diagnose ambiguity/no matching callable without a false undeclared-name error.
- Preserve direct-call overload resolution, declaration visibility, nominal
  package identity, closure ownership, and native callback routing.
- Do not add default arguments or parameter names to function-value types; do
  not silently choose the first overload or weaken tests to use the lambda
  workaround. Open templates across compiled packages are a separate design.

## Blast Radius

Audit seed command: `rg -n 'lower_expression|arg_slot_expected|resolve_overload|resolve_imported_overload|HirExpression::Identifier' src/monomorph/lower.rs`.

| Site | Classification |
|---|---|
| `lower_expression`, annotated local/global initializer and assignment | In scope; annotated local failures measured, remaining contexts need RED tests. |
| `lower_expression`, return operands and argument slots (`arg_slot_expected`) | In scope; expected types are currently restricted in some paths to nested calls. Audit bare references explicitly. |
| `lower_expression`, typed record/collection initializers | In scope; shared reference arm, not separately reproduced yet. |
| `resolve_imported_overload`, `ImportedOverload` | In scope audit; imported metadata currently records parameter types, not an expected callable return type. Prove failures before choosing changes. |
| `resolve_overload`, direct call expressions | Regression control; direct call measured successful. |
| `lower_expression_with_expected`, `builtin_predicate_ref_type` | Regression control; separate built-in contextual mechanism must retain its behavior. |
| `function_types`, native LINK references | Regression control; already concrete signatures, no new C-callback ABI work. |

## Fix Design

Resolve references while the overload set and declaration visibility are still
available, before the second resolver pass. Propagate the expected complete
callable type through value contexts, compare exact signatures, and rewrite to
the chosen concrete symbol. Use the same symbol/visibility rules as direct calls;
carry return metadata where imported selection needs it. Do not reconstruct a
signature by parsing lossy mangled names. Rejecting a reference must preserve its
source location and public spelling.

Generated output should change only for newly accepted function references and
corrected diagnostics; the existing direct-call and closure ABI are unchanged.

## Phases

### Phase 1 — failing tests and context audit

- [ ] Add `tests/runtime/rt_overloaded_function_values.rs`: separate RED cases
      for parameter-type and return-type overloads, plus single/direct/lambda
      controls. Require runtime `5\n`, not only compilation.
- [ ] Add typed local/global, assignment, callback-argument, return and aggregate
      cases; audit imported overloads with visibility and aliases.
- [ ] Add ambiguity, no-match and isolation mismatch diagnostics fixtures under
      `tests/syntax/functions/`; record a verdict for every blast-radius row.

Acceptance: `cargo test --test rt_overloaded_function_values` fails on the
recorded overload reference cases, with controls passing.
Commit: —

### Phase 2 — contextual reference selection

- [ ] Implement expected-callable propagation and reference rewriting in
      `src/monomorph/lower.rs`, with any required signature metadata in
      `src/monomorph/mod.rs` / `helpers.rs`.
- [ ] Preserve visibility and imported nominal identity; add function-level
      valid/invalid fixtures for each changed function as required by `.ai/compiler.md`.
- [ ] Replace the current limitation in `language/06_functions.md` and document
      the algorithm in `architecture/12_monomorphization.md` and type-inference.

Acceptance: `cargo test --test rt_overloaded_function_values` passes, including
exact runtime outputs and diagnostic controls.
Commit: —

### Phase 3 — complete validation

- [ ] Run `cargo test --workspace` and
      `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- [ ] Inspect any unexpected artifact delta by localizing one fixture; never
      re-baseline existing tests without the AGENTS.md proof requirements.
- [ ] Run the standalone reproduction; render the changed spec topics and run
      `scripts/spec-census.sh --citations language architecture` and `--links`.

Acceptance: suites pass; reproduction prints `5\n` for each matching reference;
only intended artifacts differ.
Commit: —

## Validation Plan

Runtime selection is the primary proof. Negative fixtures prove ambiguous or
incompatible values are refused. Full-suite and acceptance runs occur after the
fix, not as a substitute for the runtime proof. No compiler fix is included in
the documentation commit that files this work order.

## Open Decisions

Prefer the existing `TYPE_OVERLOAD_AMBIGUOUS` family for ambiguous references;
choose the no-match diagnostic after checking existing source-level callable
mismatch diagnostics. Neither outcome may claim a declared name is undeclared.
