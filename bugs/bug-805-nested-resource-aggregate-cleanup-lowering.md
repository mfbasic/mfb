# bug-805: nested resource-bearing aggregate return fails native cleanup lowering

Last updated: 2026-10-03
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: proposed `tests/runtime/rt_resource_review_805.rs` plus syntax/behavior fixtures described below.

A function that declares its result container before producing a resource,
then returns `List OF List OF Holder` with a `RES fs::File` field, passes source
checking but fails native lowering. The single correct behavior is to compile,
keep the file open through the returned aggregate, and close it once at the
receiving cleanup scope. Container depth and record wrapping must not require a
close operation on an ordinary aggregate type.

References:

- `src/docs/spec/language/14_memory-semantics.md` — binding validity and lexical cleanup.
- `src/docs/spec/language/15_resource-management.md` — resource views, cleanup scope, aggregate routing.
- Found while addressing memory/resource specification review items 4.1–4.5.

## Failing Reproduction

From the repository root, after `cargo build`:

```sh
mkdir -p /tmp/mfb-bug-805/src
printf 'first\nsecond\n' > /tmp/mfb-resource-review-data.txt
cat > /tmp/mfb-bug-805/project.json <<'JSON'
{"name": "nested_return", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "entry": "main", "targets": ["native"]}
JSON
cat > /tmp/mfb-bug-805/src/main.mfb <<'BASIC'
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
FUNC make() AS List OF List OF Holder
  MUT out AS List OF List OF Holder = []
  RES f = fresh()
  out = collections::append(out, [Holder[f]])
  RETURN out
END FUNC
FUNC main() AS Nothing
  LET out = make()
  LET row = collections::get(out, 0)
  LET h = collections::get(row, 0)
  io::print(read(h.handle))
END FUNC
BASIC
target/debug/mfb build /tmp/mfb-bug-805
```

- Observed: Build exits 1 with:
`error: owned-list element type 'List OF Holder' has no registered close op while lowering bind out AS List OF List OF Holder`
- Expected: Build exits 0 and the executable prints `first`, exits 0, and releases the
fresh file at the caller's cleanup scope.

Reproduced on macOS AArch64 with a freshly built debug compiler, and again at
unchanged HEAD `ecd062151` using `git worktree add --detach
/tmp/mfb-resource-review-head HEAD` followed by
`CARGO_TARGET_DIR=/Users/justinzaun/Development/mfb/target cargo build
--manifest-path /tmp/mfb-resource-review-head/Cargo.toml` and the same programs.
No other native target has been executed for this report. The source mechanism is in shared
verification/lowering; cross-target behavior still needs the validation below.

## Contrast

A direct `List OF RES fs::File` result works: declare the empty `out` before
`RES f`, append `f`, return `out`, then extract its element and read it in the
caller. `python3 /tmp/mfb-resource-review/probe.py target/debug/mfb direct_return`
built and ran with exit 0 and output `first`. The failing case adds a record and
an outer list around the same resource; the direct-return ownership contract
must remain intact.

## Root Cause

`src/codegen/resource/cleanup/builder_resource_cleanup.rs:collection_resource_drop`
extracts one list/map payload type, then requires that payload itself to have a
concrete resource close op or resource-union descriptor. Here it is `List OF
Holder`, which is ordinary data containing a resource, not a resource itself.
`record_resource_field_element` only handles a direct record owner and also
requires uniform direct resource-field types.
`src/ir/resource_escape.rs:Analyzer::scan_collection_expr` propagates nesting and
record-construction edges, so it can select an outer aggregate as a float target
that the native cleanup descriptor cannot represent. The reproduced failure is
at `setup_owned_list`, before the program can run.

## Goal

Build exits 0 and the executable prints `first`, exits 0, and releases the
fresh file at the caller's cleanup scope.

### Non-goals (must NOT change)

Do not ban otherwise valid nested resource aggregates, discard their resource
fields, suppress lowering errors, leak their handles, or update goldens to bless
the internal error. Preserve borrowed-resource ownership, return-order checking,
and ordinary data copy semantics.

## Blast Radius

- `collection_resource_drop` / `setup_owned_list`: fix nested float targets and
  returned aggregates without requiring an aggregate close op.
- `emit_owned_list_push`, `emit_owned_list_seed_from_collection`,
  `emit_owned_list_drain` in `src/codegen/cleanup/owned/builder_owned_cleanup.rs`:
  audit descriptor choice, resource discovery, and freeing for each nested leaf.
- `emit_release_returned_value` in `src/codegen/resource/owner.rs`: audit release
  of every nested owned resource; release only resources owned by this frame.
- Map values and records containing collections: same shallow-descriptor hazard,
  audit in this bug; not yet runtime-confirmed by the primary reproduction.
- Heterogeneous resource fields: descriptor limitation explicitly present in
  `record_resource_field_element`; include as a descriptor-design sibling.
- Borrowed nested results from collection helpers: preserve bug-643's fix;
  they must not adopt/close the caller's resources.
- Direct record float and `WITH` replacement: observed working controls in this
  review; preserve their shared identity and cleanup behavior.

## Fix Design

Make floated cleanup entries describe the resource leaf being registered,
rather than assuming the outer aggregate's immediate payload is a closable
resource. Recursively discover resource leaves when adopting/releasing returned
aggregates. Handle mixed resource types with per-entry dispatch or equivalent
fully typed cleanup; do not call a close op with an aggregate pointer. Audit
alias deduplication, moved/closed records, and owner-word guards before freeing.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/runtime/rt_resource_review_805.rs` reproducing the input above,
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

- Choose per-entry resource drop descriptors versus a recursive aggregate descriptor; both must retain the resource owner-word check and support heterogeneous leaves.

## Summary

The repair crosses ownership analysis and observable resource behavior. This
report preserves a runnable reproduction and a test-first implementation plan;
the accompanying specification change describes current behavior, not a shipped
compiler fix.
