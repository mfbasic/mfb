# bug-919: `mfb audit` exits 0 with `errors: 0` for a program that does not type-check

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/cli/audit-type-error-exits-3 (new; name suggested)

`mfb build` rejects the program below with three type errors. `mfb audit` on the
same project prints nothing on stderr, renders a full report with
`errors: 0  warnings: 0  infos: 1`, and exits **0**. A CI gate of the form
`mfb audit --locked` passes for source that cannot be compiled. The audit's
fallibility/resource model is then computed over a program that has no meaning.

Since plan-107 the source type rules (`TYPE_BINDING_MISMATCH`,
`TYPE_BINARY_OPERATOR_MISMATCH`, …) live in `ir::verify`, which runs over
**lowered IR**. The build merges `ir::shape` and `ir::verify`. The audit seam
"keeps `ir::shape::check_project` and loses the checker's call" (commit d60640d73
message), and audit never lowers, so it never runs `ir::verify`. Before plan-107
audit ran the source checker, so this looks like a regression from that refactor.

**Correct behavior (owner decision on mechanism):** a project that `mfb build`
rejects at type checking makes `mfb audit` print those diagnostics and exit `3`
("Unreadable or malformed input"). Equivalently, audit runs the same front end a
build does, as the spec's first paragraph says.

References:

- `src/docs/spec/tooling/04_audit-format.md`: "Audit runs the same front-end pipeline a build does (manifest validation, parse, resolve, monomorphize, re-resolve, entry validation, shape pass)", and exit `3` = "Unreadable or malformed input".
- `src/audit/mod.rs` comment: "the shape pass gates audit as on the build path (its stream renders first; `ir::verify` runs over the lowered IR below it)". For audit there is no lowered IR below it.
- commit d60640d73 (plan-107-D).

## Failing Reproduction

```
mkdir -p /tmp/am3/src && cd /tmp/am3
printf '{"name":"am3","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src"}]}' > project.json
cat > src/main.mfb <<'EOF'
IMPORT io
FUNC main() AS Integer
  LET x AS Integer = "not an int"
  io::print(x & 5)
  RETURN "str"
END FUNC
EOF
mfb build; echo "build=$?"
mfb audit 2>err.txt >out.txt; echo "audit=$?"; wc -l < err.txt; sed -n '/Summary/,+3p' out.txt
```

- Observed: `build=1` with `error[2-203-0007 TYPE_BINDING_MISMATCH]` and `error[2-203-0001 TYPE_BINARY_OPERATOR_MISMATCH]`. Then `audit=0`, 0 stderr lines, and the summary shows `errors: 0`.
- Expected: `audit=3`, with the type diagnostics on stderr.

Contrast: an unresolved identifier (`RETURN undefinedThing`) does make audit exit
`3`, because the resolver is in audit's pipeline. So do parse errors and shape-pass
errors.

## Root Cause

`src/audit/mod.rs:run` stops its front end at `crate::ir::shape::check_project`.
The type rules are emitted only by `src/ir/verify/compat.rs` (for example the
`TYPE_BINDING_MISMATCH` emitter) over IR produced by lowering, which the audit
path never invokes.

## Goal

- The repro exits 3 and prints the type diagnostics, and a clean project's audit output is byte-identical to today's.

### Non-goals (must NOT change)

- The audit report schema, and audit staying offline and never executing user code.

## Blast Radius

- `src/audit/mod.rs:run`: fixed by this bug.
- `src/cli/doc.rs` (`mfb doc`) and `mfb fmt` also stop before verify. That is intended for them (doc/format do not claim to type-check). Out of scope.

## Fix Design

Lower the concrete HIR and run `ir::verify` in audit exactly as the build's
front end does (discarding the IR), returning 3 on any error-severity diagnostic.
Alternatively, document that audit does not type-check and make exit `0`
conditional on a successful `mfb build`. That alternative is rejected here
because it makes the audit gate weaker than its spec.

## Validation Plan

- Regression test: the repro as a CLI test asserting exit 3.
- Full suite: audit goldens (`*.audit` under tests/) must stay byte-identical. There are 13 (`find tests -name '*.audit'`), and each has a sibling `.ir` golden, i.e. a project that lowers cleanly.
- Doc sync: `04_audit-format.md` stage list (add lowering + verify) and the exit-3 row.
