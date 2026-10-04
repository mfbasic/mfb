# bug-806: module-level aggregate retains a resource after its local owner is reclaimed

Last updated: 2026-10-03
Effort: large (3h–1d)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: proposed `tests/runtime/rt_resource_review_806.rs` plus syntax/behavior fixtures described below.

A function creates a file and appends a record containing it to a module-level
list. The build succeeds, but using that file after the function returns reports
a moved-resource error even though no transfer occurred. The single correct
behavior is for a module-level aggregate store to retain valid resource views
and migrate cleanup to the module instance's lifetime, closing once at instance
teardown unless explicitly closed/transferred earlier. The freed-record read is
a memory-safety concern; the exact erroneous runtime message is not a stable
contract.

References:

- `src/docs/spec/language/14_memory-semantics.md` — binding validity and lexical cleanup.
- `src/docs/spec/language/15_resource-management.md` — resource views, cleanup scope, aggregate routing.
- Found while addressing memory/resource specification review items 4.1–4.5.

## Failing Reproduction

From the repository root, after `cargo build`:

```sh
mkdir -p /tmp/mfb-bug-806/src
printf 'first\nsecond\n' > /tmp/mfb-resource-review-data.txt
cat > /tmp/mfb-bug-806/project.json <<'JSON'
{"name": "global", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "entry": "main", "targets": ["native"]}
JSON
cat > /tmp/mfb-bug-806/src/main.mfb <<'BASIC'
IMPORT fs
IMPORT io
IMPORT collections
IMPORT thread
TYPE Holder
  handle AS RES fs::File
END TYPE
FUNC fresh() AS RES fs::File
  RETURN fs::openFile("/tmp/mfb-resource-review-data.txt")
END FUNC
FUNC read(RES f AS fs::File) AS String
  LET line = fs::readLine(f) TRAP(e)
    RECOVER e.message
  END TRAP
  RETURN line
END FUNC
MUT saved AS List OF Holder = []
FUNC put() AS Nothing
  RES f = fresh()
  saved = collections::append(saved, Holder[f])
END FUNC
FUNC main() AS Nothing
  put()
  LET h = collections::get(saved, 0)
  io::print(read(h.handle))
END FUNC
BASIC
target/debug/mfb build /tmp/mfb-bug-806
/tmp/mfb-bug-806/build/global.out
```

- Observed: Build exits 0. Runtime exits 0 because the probe catches the error and prints:
``Resource handle was moved to another thread by `thread::transfer` and is no longer usable by the sender.``
There is no thread start or transfer call in this program.
- Expected: The executable prints `first`, exits 0, and cleanup follows module-instance
lifetime rather than the `put` frame. A valid module store must not retain a
pointer into a reclaimed resource record.

Reproduced on macOS AArch64 with a freshly built debug compiler, and again at
unchanged HEAD `ecd062151` using `git worktree add --detach
/tmp/mfb-resource-review-head HEAD` followed by
`CARGO_TARGET_DIR=/Users/justinzaun/Development/mfb/target cargo build
--manifest-path /tmp/mfb-resource-review-head/Cargo.toml` and the same programs.
No other native target has been executed for this report. The source mechanism is in shared
verification/lowering; cross-target behavior still needs the validation below.

## Root Cause

`src/ir/resource_escape.rs:Analyzer::walk_statement` assigns declaration depth
using per-function facts, including assignment targets; it has no module-lifetime
owner. `src/codegen/engine/control/builder_control.rs:lower_ops_inner` handles
`NirOp::StoreGlobal` by storing/copying the data value, without assigning the
referenced resource a module cleanup owner.
`src/codegen/resource/owner.rs:emit_scope_token` represents function-frame scopes,
and `emit_resource_float` can raise to those scopes only.
`src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_resource_cleanup_call`
therefore closes the file at `put`'s exit; `emit_resource_block_reclaim` can free its
record. The global still carries the original pointer. The later moved-resource
message is consistent with reading the allocator's reused/freed record bytes,
not with a real transfer. A debugger/allocator-poison regression should pin the
freed-record read, rather than relying on that incidental message.

## Goal

The executable prints `first`, exits 0, and cleanup follows module-instance
lifetime rather than the `put` frame. A valid module store must not retain a
pointer into a reclaimed resource record.

### Non-goals (must NOT change)

Do not describe the observed freed-record access as safe runtime invalidation,
mask it by retaining all records forever, disable the guard, or weaken tests.
Do not remove support for module-level ordinary data or change thread isolation.
Do not silently close resources when replacing/removing their last global view.

## Blast Radius

- `NirOp::StoreGlobal` in `lower_ops_inner`: fix resource-bearing global stores,
  including append/self-update and replacement paths.
- `Analyzer::walk_statement` and its per-function routing facts: account for
  global destinations instead of assigning them local declaration depths.
- Scope tokens / owner comparisons in `src/codegen/resource/owner.rs`: add a
  module-instance owner or equivalent persistent cleanup representation.
- Global records, nested lists/maps, imported package globals, and resource
  views supplied through parameters: same lifetime boundary; audit in this bug.
- Module/worker teardown: drain persistent cleanup once per owning instance.
- Parameter passed from a still-live main scope and stored in the global:
  observed working while that scope remains live; does not prove safety after
  that owner exits. Preserve this control and extend it past the owner exit.
- Local outer-container float: observed working control; preserve it.
- Ordinary resource-free globals: unaffected by resource ownership, preserve
  their existing copy/reassignment behavior.

## Fix Design

Introduce a cleanup owner whose lifetime covers the module instance and route
resources stored into module aggregates to it, recursively through aggregate
values. Integrate persistent ownership with function-scope comparison, return,
explicit close, transfer, and worker-instance teardown. Preserve resources once
registered even when their final stored view is removed: ownership only floats
outward. The representation and teardown changes make this a semantic/runtime
fix, not a small StoreGlobal patch.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/runtime/rt_resource_review_806.rs` reproducing the input above,
      asserting the expected result rather than the observed failure.
- [ ] Add syntax and runtime fixtures for success, failure, alias, and nesting
      siblings in the blast-radius list; demonstrate RED before editing behavior.
- [ ] Complete the listed sibling audit, recording which cases fail and why.

Acceptance: the primary regression fails for the documented reason, and each
sibling has a measured outcome or an explicit untested status.
Commit: —

### Phase 2 — the fix

- [ ] Implement the design above and all confirmed in-scope sibling fixes.
- [ ] Preserve the non-goals and add matching valid/invalid function fixtures.
- [ ] Update the owning language/memory/architecture topics to the implemented
      contract; remove the as-is limitation when the corresponding proof passes.

Acceptance: primary and sibling regressions pass, including observable cleanup
and the existing contrast cases.
Commit: —

### Phase 3 — expected outputs + full validation

- [ ] Run the full suite before correcting any proven-wrong golden line; establish
      its history, protected contract, dependents, and independent proof first.
- [ ] Inspect any artifact differences fixture by fixture; change only expected
      lines disproved by the new contract, never blanket rebaseline.
- [ ] Run `cargo test`, including the new runtime regression and affected runtime
      targets, then `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- [ ] Run the primary and sibling programs on affected native backends using
      `.ai/remote_systems.md`; record unavailable environments explicitly.

Acceptance: all required gates pass; the source programs demonstrate the intended
behavior, and artifact differences are limited to reviewed changes.
Commit: —

## Validation Plan

- Regression: the Phase 1 tests and source fixtures, kept RED until implementation.
- Runtime proof: actual generated executables, exact output assertions, and
  cleanup observations (buffer flush / descriptor counts / allocator diagnostics).
- Doc sync: memory semantics, resource management, escape analysis, and any
  changed runtime/IR representation topic.
- Full suite: the commands in Phase 3; a compile or golden pass alone is not proof
  of correct resource lifetime.

## Open Decisions

- Choose the persistent module-instance owner representation and its ordering relative to frame tokens; do not encode a dead stack address as the owner.

## Summary

The repair crosses ownership analysis and observable resource behavior. This
report preserves a runnable reproduction and a test-first implementation plan;
the accompanying specification change describes current behavior, not a shipped
compiler fix.
