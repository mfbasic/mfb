# bug-797: Function-level TRAP reads a local whose declaration was skipped

Last updated: 2026-10-02
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness
Status: Open
Regression Test: proposed `tests/syntax/trap-local-before-initialization-invalid` and verifier unit tests

A function-level handler can read a local declared after the failing operation.
The program compiles and returns unwritten frame data. The correct behavior is
to reject a handler read unless the local is initialized on every incoming
failure path reaching that read. Resolving the name is not proof of initialization.
This requires failure-edge dataflow, so it is filed separately from the spec repair.

References: `src/docs/spec/language/08_error-model.md` §8.6a;
`src/docs/spec/language/14_memory-semantics.md` §14.2 (failed replacement preserves
an already initialized mutable value).

## Failing Reproduction

Run from the repository root:

```sh
cargo build
mkdir -p /tmp/mfb-spec-review/handler_late/src
cat > /tmp/mfb-spec-review/handler_late/project.json <<'JSON'
{"name": "handler_late", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "entry": "main", "targets": ["native"]}
JSON
cat > /tmp/mfb-spec-review/handler_late/src/main.mfb <<'MFB'
IMPORT io
FUNC example() AS Integer
  mightFail()
  LET value = 42
  RETURN value
  TRAP(e)
    RETURN value
  END TRAP
END FUNC
FUNC mightFail() AS Nothing
  FAIL error(90000001, "probe")
END FUNC
FUNC main() AS Integer
  io::print(toString(example()))
  RETURN 0
END FUNC
MFB
target/debug/mfb build -q /tmp/mfb-spec-review/handler_late
/tmp/mfb-spec-review/handler_late/build/handler_late.out
```

Observed on Darwin arm64 at HEAD `5d06970c9c8bbd3f50a5c20319e3463727ae172f`:
build exits 0, executable prints `4294967296` and exits 0. The particular integer
is not contractual: the defect is the accepted read of a never-initialized local.
Expected: compilation rejects `RETURN value` in the handler, identifying the
failure edge before `LET value = 42`; no executable is produced by that build.

Contrast: move `LET value = 42` before `mightFail()`. The same build/run commands
then succeed and print `42`. Both observations used a fresh `cargo build`.
Other native targets have not been executed; the missing analysis is shared.

## Root Cause

- `src/resolver/resolution.rs:resolve_function` resolves the whole normal body,
  then clones its accumulated locals into the trap environment. This grants
  lexical visibility regardless of when failure occurs.
- `src/ir/verify/ops.rs:check_ops` handles `IrOp::Trap` by cloning the locals and
  mutability maps accumulated from preceding operations. It does not intersect
  initialization facts from the failure edges to that handler. `LET value` is
  present in that map even when runtime control never executes it.
- Native lowering consequently receives an accepted read of a local slot whose
  defining store was bypassed. The runtime observation above establishes the
  consequence; a specific backend register-allocation mechanism is not needed
  to reproduce the acceptance defect.

Evidence commands: `sed -n '850,893p' src/resolver/resolution.rs`,
`sed -n '760,815p' src/ir/verify/ops.rs`, and the reproduction above.

## Goal

Reject reads without initialization on every reaching error path, on both
source input and decoded package IR. Preserve reads of locals initialized before
all relevant failure edges, including the old value after a failed MUT RHS.

### Non-goals (must NOT change)

Do not zero-initialize source locals to mask the read. Do not ban all body-local
reads in a handler, move declarations in the negative test, or change recovery,
resource cleanup, defaultable MUT declarations, or assignment commit semantics.
No ABI or package-format change is presumed necessary.

## Blast Radius

Search: `rg -n 'IrOp::Trap|check_ops|check_resource_moves' src/ir/verify` and
`rg -n 'handler_locals|trap_locals' src/resolver/resolution.rs`.

- `resolution.rs:resolve_function` and `ops.rs:check_ops` function-level handlers:
  affected; fixed by this bug. Include failures in calls, operators, explicit
  FAIL, nested branches/loops, and inline-handler propagation.
- `ops.rs:check_ops` assignment/Bind paths: included in the analysis; establish
  initialization only after RHS success, and preserve prior initialization on
  failed reassignment.
- `resolution.rs:resolve_expression` inline handler environment: contrast path;
  it starts from locals visible at the expression, not later body declarations.
  Audit recovery and propagation joins; no failure reproduced here.
- `resources.rs:check_resource_moves`: separate consumed-resource analysis;
  preserve its rejection of own-resource reads after trap cleanup. It is not
  a replacement for ordinary-local definite initialization.
- `verify/mod.rs:collect_diagnostics_with`: shared source/package verification
  entry; enforce the new rule for decoded IR too.

## Fix Design

Track definitely initialized locals along normal and error edges. At handler
entry intersect the facts of all incoming failure edges; inside the handler,
propagate those facts to each read. Include raising operators and implicit
propagation, not just explicit FAIL. Initializer success establishes a binding;
failed reassignment leaves the previous established value initialized.

Use a conservative, documented failure-edge policy. If infallibility pruning
is used, it must agree with the compiler's existing effect classification.
Resource move/cleanup rules remain independent of initialization facts.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a negative source fixture for the reproduction and an equivalent
      decoded-IR verifier test; prove both currently accept the invalid read.
- [ ] Add positive guards for declaration-before-failure and failed MUT RHS.
- [ ] Cover checked-operator errors, branches, loops, and inline propagation;
      finish the consumer audit above.

Acceptance: negative tests fail for missing diagnostics; positive guards pass.
Commit: —

### Phase 2 — the fix

- [ ] Implement failure-edge initialization analysis in shared verification.
- [ ] Add a precise diagnostic and update its owning spec/rule registry.
- [ ] Update §8.6a to the implemented rule and remove its current limitation.

Acceptance: negative tests reject the read; initialized-local reads still work.
Commit: —

### Phase 3 — expected outputs + full validation

- [ ] Run the full suite before changing any existing golden; prove each changed
      diagnostic line wrong using AGENTS.md's history/contract/dependency checks.
- [ ] Run `cargo test`, `scripts/artifact-gate.sh target/release/mfb all`, and
      `scripts/test-accept.sh` with the rebuilt release binary as required by its help.
- [ ] Re-run the original rejection probe and runtime positive contrasts.

Acceptance: all required gates pass; only intended diagnostics change.
Commit: —

## Validation Plan

The negative test proves compile-time rejection, independently of the arbitrary
uninitialized integer. Positive runtime tests prove `42` for the early-declaration
case and the preserved old value after a failed RHS. Include a decoded-package
case to prevent bypassing source validation. Keep the error-model, architecture
verification topic and diagnostic registry consistent with the implementation.

## Open Decisions

Choose the diagnostic identifier and conservative treatment of calls whose
infallibility is not established; default to treating such calls as failure edges.

## Summary

The risk is missing an implicit error edge or rejecting a correctly initialized
handler local. This is a dataflow repair, not a slot-zeroing repair.
