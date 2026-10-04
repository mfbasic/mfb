# bug-806: module-level aggregate retains a resource after its local owner is reclaimed

Last updated: 2026-10-03
Effort: large (3h–1d)
Severity: HIGH
Class: Memory-safety

Status: Fixed (2026-10-03, bug-staging-7, f4ee7b96f)
Regression Test: `tests/runtime/rt_resource_review_806.rs` (17 runtime tests)

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

- [x] Add `tests/runtime/rt_resource_review_806.rs` reproducing the input above,
      asserting the expected result rather than the observed failure.
- [x] Add runtime tests for success, failure, alias, and nesting siblings in the
      blast-radius list; RED against the base compiler (see Resolution). No
      `tests/rt-behavior` fixture was added: its goldens belong to the batch gate.
- [x] Complete the listed sibling audit, recording which cases fail and why.

Acceptance: the primary regression fails for the documented reason, and each
sibling has a measured outcome or an explicit untested status.
Commit: f4ee7b96f

### Phase 2 — the fix

- [x] Implement the design above and all confirmed in-scope sibling fixes.
- [x] Preserve the non-goals (controls and non-goal cases in the runtime tests).
- [x] Update the owning language/memory/architecture topics to the implemented
      contract; remove the as-is limitation when the corresponding proof passes.

Acceptance: primary and sibling regressions pass, including observable cleanup
and the existing contrast cases.
Commit: f4ee7b96f

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

- Choose the persistent module-instance owner representation and its ordering relative to frame tokens; do not encode a dead stack address as the owner.

## Summary

The repair crosses ownership analysis and observable resource behavior. This
report preserves a runnable reproduction and a test-first implementation plan;
the accompanying specification change describes current behavior, not a shipped
compiler fix.

## Resolution (2026-10-03, bug-staging-7)

Fixed in `f4ee7b96f` (on top of bug-805's `5814f14f4`, whose aggregate walk it
reuses). The reproduction prints `first` (`cargo test --test rt_resource_review_806
a_resource_appended_to_a_global_stays_readable`: ok; FAILED against the base
compiler built from `bug-staging-7` at `692e7f32d`).

**Design decision (the Open Decision).** A module-level binding is scope depth `-1`,
the outermost scope there is; per instance it lives until instance teardown.

* **Static routing.** `ir::resource_escape` places an assignment to a module-level
  binding at `MODULE_DEPTH` (`-1`) instead of the statement's depth, so a resource the
  function produces and stores into a global floats to it like any outer container
  (`ResOwner::Float(global)`). `ir::lower` hands the analyzer the module's global
  names. A `RES` parameter is not floated there: its owner is a caller frame.
* **Owner representation.** `MODULE_OWNER_TOKEN` is all ones. A frame token is
  `((sp + slot) << 12) - depth`, `depth` in `1..4096`; it equals all ones only for
  `depth == 1` at a stack address of `0` mod `2^52`, so no frame token can equal it
  (unit test `the_module_token_is_no_frame_token`). It is above every frame token, so
  a float never lowers it; the `RETURN` frame test reads `2^52` for it, no stack
  address, so a return never releases it. Every scope-exit close, owned-list drain,
  discarded-result close and claim is owner-gated already, so the storing function's
  cleanup now skips the record instead of closing and freeing it.
* **Per-instance list.** One writable global slot, `$module_resources`, reserved just
  past the program's own globals only when some global's type can reach a resource,
  heads a list of `{record, next, kind, tag}` arena nodes. The globals region is per
  arena, so the main program and every worker instance have their own list. A union
  leaf is listed as its active variant's record plus tag, never the box (the box is
  freed by the binding that wrapped it).
* **Dynamic raise at the store.** `StoreGlobal` raises every resource it adds to the
  module (`emit_module_store_float`): before the store, by walking only the stored
  operands that are named values (a local, a parameter, a capture, a field chain over
  one) through constructors, literals, `WITH` and `collections` insertions — so
  `g = collections::append(g, Holder[f])` costs O(item), not O(len(g)) — and after
  the store over the whole global when an operand is a call whose resources have no
  name. A record already module-owned is skipped, so each is listed once.
* **Teardown.** `_mfb_module_resource_drain` closes every listed record still owned
  by the module (an explicit close or transfer leaves the flag, and the re-close is
  the defined no-op). The program entry calls it once on its single exit label —
  where `main` returning, `EXIT PROGRAM`, an untrapped error and a failed initializer
  all converge — the worker trampoline after the worker body returns (before the
  result is published), and the canvas graphics thread after its render loop. It
  closes only the kinds some store listed, so a program whose global can hold a
  `tcp::Socket` but never opens one links no `tcp` close; a built-in close it does
  call is emitted with the other runtime helpers. It reclaims nothing (the arena goes
  with the instance).

Non-goals kept: nothing closes when the last global view is removed or replaced
(`removing_the_last_global_view_does_not_close`), records are not retained beyond
their instance, no guard is disabled, and resource-free globals and thread
isolation are unchanged (a worker's stored resource is its own instance's;
`a_workers_global_resource_closes_at_worker_exit` sees the parent's copy empty).

### Sibling audit (measured)

`tests/runtime/rt_resource_review_806.rs`, 17 tests; against the base compiler all
17 FAIL, all pass with the fix:

| Case | Base | Fixed |
| --- | --- | --- |
| doc reproduction | freed-record read (`… moved …`) | `first` |
| buffered handle in a global, `--debug` | closed at `put`'s exit | open while running, flushed at exit, `double_free_skips 0` |
| `RES` parameter / record parameter / resource-union parameter stored from a caller scope that exits first | closed by the caller's scope | open, closed at exit |
| flat `List OF RES`, nested `List OF List OF Holder`, `Map OF String TO Holder` globals | closed early / lowering error | open, closed at exit |
| call result stored into a global (post-store walk) | lowering error / leak | open, closed at exit |
| last view removed (`saved = []`) | closed at `put`'s exit | stays open, closed at exit |
| explicit close through the global | — | honoured; teardown re-close is the no-op |
| `EXIT PROGRAM 3` from a nested call / untrapped `FAIL` | file never flushed | flushed, exit 3 / 255 |
| worker storing into its own copy of a global | closed at the storing function's exit | open in the worker, closed at worker exit |
| global initializer storing a resource | not closed at exit | closed at exit |
| 40 stores in a loop, `--debug` | closed early | all 40 open, each closed once, `double_free_skips 0` |
| local outer `List OF Holder` float (local twin) | lowering error | closes at the container's exit |

### Validation

* `MFB_TEST_EXE=/tmp/b805-target/release/mfb cargo test --release --no-fail-fast`
  over `rt_resource_review_805/806`, `rt_owned_argument`, `rt_inline_trap_union_bind`,
  `rt_res_rebind_alias`, `rt_res_state_inplace_mutation`, `rt_res_list_from_call_leak`,
  `rt_res_list_kept_by_call`, `rt_resource_owner_scope`, `rt_scope_drop_leaks`,
  `rt_audio_alias_trap_resource_result`, `rt_imported_resource_scope_drop`,
  `rt_native_resource_scope_drop`, `rt_recursive_map_transfer`,
  `rt_recursive_thread_transfer`, `rt_inplace_global_record`, `rt_global_self_update`,
  `rt_user_global_vs_builtin_locals`, `rt_global_init_order_fold`,
  `rt_for_each_over_reassigned_global`, `rt_global_argument_reassigned_by_callee`,
  `rt_imported_package_global`, `rt_top_level_initializer_globals`,
  `codegen_thread_exit_arena_teardown`, `codegen_owned_drop_free_and_null`,
  `cli_thread_accept_res_bind`, `rt_inline_trap_borrowed_resource`,
  `rt_thread_accept_res_drop_closes`, `cli_canvas_image_resource`: all 29 test binaries ok (`rt_scope_drop_leaks` 154, `rt_resource_review_805` 18, `rt_resource_review_806` 17).
* `cargo test --release --bin mfb -- resource owned escape module_owner spec record_ownership`: 290 passed.
* `scripts/test-accept.sh /tmp/b805-target/release/mfb /tmp/b805-acc` over
  `rt-behavior/{resources,thread,threads,scope,memory,fs,tcp,native,udp,tls,net,process,packages}/*`,
  `syntax/resources/*`, `rt-error/{threads,net,tcp}/*`: `acceptance tests passed (338 test(s) ran)`.
* Cross-target: the reproduction and eleven siblings, cross-built for
  `linux-aarch64`, `linux-riscv64` and `windows-x86_64`, run on box 2226 (Debian
  aarch64 glibc), 2229 (Alpine riscv64 musl) and 192.168.1.206 (Win11 x86_64): every
  program's output and every file's content after exit matched macOS.

### Expected golden moves (for the batch gate)

* `.ncode`/`.ncodesum` of fixtures that RETURN, discard, bind or assign from a call a
  value reaching a resource (a flat `List OF RES` included: the release/seed now go
  through the walk's labels and slots, and the drain is kept on `RETURN`), and of
  programs with an owner container. A plain resource or resource-union return/discard
  emits the same instructions as before.
* `.ncode`/`.ncodesum` of fixtures with a global whose type reaches a resource: the
  extra global slot shifts the `LINK`/`term`/app regions and the entry frame, the
  entry calls `_mfb_module_resource_drain`, and stores into such globals raise.
* `.ncode` of fixtures declaring a resource union with a `File` variant that return a
  plain `File` (the 16-byte copy is gone).
* `.nir` `resourceOwners` only where a resource floats to a global (`Float(global)`);
  `.ir` does not render `resource_owners`, so no `.ir` moves. A `.mfp` of a package
  whose function stores a resource into a global encodes `Float(global)` instead of
  `Local`.
