# bug-797: Function-level TRAP reads a local whose declaration was skipped

Last updated: 2026-10-03
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness
Status: Fixed
Regression Test: `tests/syntax/trap/trap-local-before-initialization-invalid`; `tests/rt-behavior/trap/trap-local-initialized-before-failure-rt` (positive, prints 53); `src/ir/verify/tests.rs` (`rejects_a_handler_read_of_a_local_declared_after_a_failure` + three positive guards)

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

- [x] Add a negative source fixture for the reproduction and an equivalent
      decoded-IR verifier test; prove both currently accept the invalid read.
- [x] Add positive guards for declaration-before-failure and failed MUT RHS.
- [x] Cover checked-operator errors, branches, loops, and inline propagation;
      finish the consumer audit above.

Acceptance: negative tests fail for missing diagnostics; positive guards pass.
Commit: —

### Phase 2 — the fix

- [x] Implement failure-edge initialization analysis in shared verification.
- [x] Add a precise diagnostic and update its owning spec/rule registry.
- [x] Update §8.6a to the implemented rule and remove its current limitation.

Acceptance: negative tests reject the read; initialized-local reads still work.
Commit: —

### Phase 3 — expected outputs + full validation

- [x] Run the full suite before changing any existing golden; prove each changed
      diagnostic line wrong using AGENTS.md's history/contract/dependency checks.
- [x] Run `cargo test`, `scripts/artifact-gate.sh target/release/mfb all`, and
      `scripts/test-accept.sh` with the rebuilt release binary as required by its help.
- [x] Re-run the original rejection probe and runtime positive contrasts.

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

## Resolution (2026-10-03)

**Rule.** `TYPE_TRAP_LOCAL_UNINITIALIZED` (`2-203-0143`; `0141`/`0142` are
reserved by plan-167-B). A function-level handler may read a top-level local only
if nothing in the body up to and including that local's declaration can fail. A
failure in its own initializer counts.

- Parameters are always readable.
- Assignments do not un-initialize: a failed replacement keeps the old value.
- The function's own resources are left to bug-676's trap-cleanup rule, which
  already rejects every handler read of one. This avoids reporting the same read
  twice.

**Failure census** (`src/ir/verify/trap_init.rs`). It is the IR twin of
`ir::fallible`:

- `FAIL`;
- raising arithmetic operators (the negative-literal exemption applies);
- calls, unless the built-in census proves them total or they are project
  functions shown total by an IR fixpoint. Imported/unknown callees count as
  failing, the doc's conservative default.

A `CallResult`/`Checked` is captured by its inline `TRAP` and is not an edge. The
check runs in `ir::verify`, so source and decoded packages are held alike.

**Tests corrected (four questions).** These are bug-465's tcp/tls EOF
characterization fixtures (542bd4d1f). They protect "`read` raises at EOF and the
drain idiom terminates", and nothing else depends on the moved lines.
`drainUntilClosed` / `drainToEof` declared `MUT total = 0` *after*
`tcp::listen`/`connect`/`accept` (and `tls::connect`). A setup failure therefore
entered the handler and read `total` uninitialized: exactly this bug, and the
"checking defect" spec §8.6a already named. The `mfb man tcp read` example they
mirror declares it first. Fix: only that line moves to the top of each function.
Their `.ast`/`.ir` goldens move with it and are regenerated at the batch gate.

**Proof.**

- The reproduction is rejected with the new rule.
- The positive fixture runs and prints `53`
  (`value`/total-function/builtin/failed-`MUT`).
- Every fixture with a function-level `TRAP` (118) was run: the only new
  diagnostics were the tcp/tls fixtures above and the bug-676 fixture's
  duplicate, now suppressed. The remaining mismatches are bug-785's stale
  `ThreadWorker` packages.
- Verifier unit tests pass, 461.
- Specs: `mfb spec language error-model` §8.6a (the limitation is replaced by the
  rule), `diagnostics rule-codes`, and `language resource-management`'s cross-ref.
