# bug-805: nested resource-bearing aggregate return fails native cleanup lowering

Last updated: 2026-10-03
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness

Status: Fixed (2026-10-03, bug-staging-7, 5814f14f4)
Regression Test: `tests/runtime/rt_resource_review_805.rs` (18 runtime tests)

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

- [x] Add `tests/runtime/rt_resource_review_805.rs` reproducing the input above,
      asserting the expected result rather than the observed failure.
- [x] Add runtime tests for success, failure, alias, and nesting siblings in the
      blast-radius list; RED against the base compiler (see Resolution). No
      `tests/rt-behavior` fixture was added: its goldens belong to the batch gate.
- [x] Complete the listed sibling audit, recording which cases fail and why.

Acceptance: the primary regression fails for the documented reason, and each
sibling has a measured outcome or an explicit untested status.
Commit: 5814f14f4

### Phase 2 — the fix

- [x] Implement the design above and all confirmed in-scope sibling fixes.
- [x] Preserve the non-goals (controls and non-goal cases in the runtime tests).
- [x] Update the owning language/memory/architecture topics to the implemented
      contract; remove the as-is limitation when the corresponding proof passes.

Acceptance: primary and sibling regressions pass, including observable cleanup
and the existing contrast cases.
Commit: 5814f14f4

### Phase 3 — expected outputs + full validation

- [ ] Run the full suite before correcting any proven-wrong golden line; establish
      its history, protected contract, dependents, and independent proof first.
- [ ] Inspect any artifact differences fixture by fixture; change only expected
      lines disproved by the new contract, never blanket rebaseline.
- [ ] Run `cargo test`, including the new runtime regression and affected runtime
      targets, then `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- [x] Run the primary and sibling programs on affected native backends using
      `.ai/remote_systems.md`; record unavailable environments explicitly.

Acceptance: all required gates pass; the source programs demonstrate the intended
behavior, and artifact differences are limited to reviewed changes.
Commit: — (full suite / acceptance / artifact gate: coordinator batch run)

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

## Resolution (2026-10-03, bug-staging-7)

Fixed in `5814f14f4`. The reproduction builds and prints `first`
(`MFB_TEST_EXE=… cargo test --test rt_resource_review_805
a_nested_list_of_records_returns_a_live_handle`: ok; FAILED against the base
compiler `/tmp/b805-head-target/release/mfb`, built from `bug-staging-7` at
`692e7f32d`).

**One walk for "every resource this value reaches".** `src/codegen/resource/walk.rs`
(`CodeBuilder::emit_for_each_resource`) descends a value by its type — list and map
elements (a map skips its tombstones), record fields (read exactly as
`lower_field_access` reads them), a data union's active variant (the record it
inlines at `+16`) — and hands every resource leaf (a record, or a resource-union
box) to one visitor. A type that reaches itself (`TYPE Node { h AS RES File, kids AS
List OF Node }`) is expanded once per kind and walked through a run-time worklist of
arena nodes, never by recursion on the native stack. Its consumers:

* `RETURN` releases every reached record the returning frame owns
  (`emit_release_returned_value`), not only a resource, a union or a flat
  `List OF RES` — the reason the fix could not stop at making the program compile:
  with the drop descriptor alone it compiled and then printed `Resource handle is
  already closed`, because `make`'s drain closed `f` at `make`'s exit.
* A binding of a call result adopts every reached resource (`setup_owned_list` +
  `emit_owned_list_seed_from_collection`), and so — found by this audit, broken at
  base for a flat `List OF RES` too — does an ASSIGNMENT of a call result to an
  existing container binding (`call_assigned_owner_containers`): at base those
  resources were owned by nobody and a buffered write through them was lost.
* A discarded call result closes the unowned records it reaches
  (`emit_close_discarded_value`).

**The owned-list drop is the leaf's, not the container's.** `collection_resource_drop`
took one aggregate layer and required a close op there; `resource_leaf_drop` takes
the leaf, and a container reaching several leaf kinds (a record with a `File` field
and a resource-union field — refused at base, "an owned-list carries one close op")
registers one list per kind. `record_resource_field_element` and the free
`record_res_field_types`/`is_resource_owning_container` are gone with that refusal.

**`RETURN` of an owner container keeps its drains.** It used to remove them
(`deactivate_owned_list`). The release already makes every returned record unowned,
so the kept drains close nothing the caller adopts; they free the 16-byte node every
float allocated (16 B leaked per call at base, `a_returned_flat_resource_list_frees_its_float_nodes`)
and close a resource removed from the container before the `RETURN`, which at base
stayed owned by the dead frame and was never closed
(`a_resource_removed_before_the_return_closes_in_the_callee`).
`record_ownership::owning_collections` now also treats a collection of records that
reach a resource as able to own its adopted records, so the adopting drain reclaims
them: 0 bytes live after 50 or 100 calls (`a_returned_nested_aggregate_leaks_nothing_per_call`;
112 B per call before that change).

**Sibling found in the heterogeneous-record audit.** Declaring `UNION Stream {
fs::File, tcp::Socket }` anywhere made `RETURN f` of a plain `File` hand the caller a
16-byte copy of the record's head: `inline_collection_payload_size` answered for the
variant with the union's inline size. A concrete resource is now never an inline
payload (`a_declared_resource_union_does_not_change_a_plain_file_return`; at base
`RES f = fresh(); fs::writeAll(f, "+")` in such a program raised
`7-703-0009 … moved to another thread`).

### Sibling audit (measured)

`tests/runtime/rt_resource_review_805.rs`, 18 tests. Against the base compiler 16
FAIL and the 2 controls pass (`the_flat_resource_list_return_is_unchanged`,
`a_passed_in_handle_returned_inside_a_nested_aggregate_keeps_its_owner`); the
bug-643 control `a_repackaged_…` fails there only because its nested `make()` does
not lower. With the fix all 18 pass.

| Case | Base | Fixed |
| --- | --- | --- |
| `List OF List OF Holder` return (primary, read + buffered + `--debug`) | lowering error | first / closes once at caller scope, `double_free_skips 0` |
| `List OF Holder`, `Map OF String TO Holder`, record holding `List OF Holder` | lowering error | transfer, close once at caller |
| record with `File` + resource-union fields | lowering error (one close op) | both transfer |
| data-union variant carrying a `RES` field | lowering error | transfers |
| recursive record tree | lowering error | every handle transfers, `double_free_skips 0` |
| discarded nested result | lowering error | closed at the call statement |
| error before the `RETURN` | lowering error | closed by the callee's error exit |
| passed-in handle returned nested (bug-755 shape) | works | unchanged |
| `collections::take`/`get` over the caller's nested value (bug-643) | lowering error | adopts nothing |
| flat `List OF RES` return (contrast) | works, 16 B/call node leak | works, no leak |
| assignment of a call result to a container binding | never closed | adopted into the binding's scope |

### Validation

* `cargo test --release --test rt_resource_review_805` (+ the 26 resource/global
  runtime and codegen tests listed in the 806 resolution): ok.
* `cargo test --release --bin mfb -- resource owned escape module_owner spec record_ownership`: ok.
* `scripts/test-accept.sh` over the resource/thread/fs/tcp/net/native/scope/memory/
  process/package fixtures: see the 806 resolution.
* Cross-target: the primary and nine sibling programs cross-built with
  `mfb build --target linux-aarch64|linux-riscv64|windows-x86_64` and run on box 2226
  (Debian aarch64 glibc), 2229 (Alpine riscv64 musl) and 192.168.1.206 (Win11 x86_64):
  identical output and file contents on all three.

Not run here (coordinator's batch gate): full `cargo test`, full acceptance, artifact
gate. Expected golden moves are listed in the 806 resolution.
