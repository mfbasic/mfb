# bug-804: successful thread transfer does not invalidate the sender in static move checking

Last updated: 2026-10-03
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: `tests/runtime/rt_resource_review_804.rs`; fixtures `tests/syntax/threads/thread-transfer-use-after-move-invalid`, `tests/syntax/threads/thread-transfer-failure-handler-valid`, `tests/syntax/resources/branch-close-sibling-arm-valid`.

A direct `thread::transfer(t, f)` followed by a read of `f` builds successfully;
the read is refused only at runtime. The single correct behavior is that a read
on the successful continuation is diagnosed as `TYPE_USE_AFTER_MOVE`, while
inline failure recovery retains permission to use the untransferred resource.
This is a diagnostic gap, not evidence that the moved runtime guard is missing.

References:

- `src/docs/spec/language/14_memory-semantics.md` — binding validity and lexical cleanup.
- `src/docs/spec/language/15_resource-management.md` — resource views, cleanup scope, aggregate routing.
- Found while addressing memory/resource specification review items 4.1–4.5.

## Failing Reproduction

From the repository root, after `cargo build`:

```sh
mkdir -p /tmp/mfb-bug-804/src
printf 'first\nsecond\n' > /tmp/mfb-resource-review-data.txt
cat > /tmp/mfb-bug-804/project.json <<'JSON'
{"name": "post_transfer", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "entry": "main", "targets": ["native"]}
JSON
cat > /tmp/mfb-bug-804/src/main.mfb <<'BASIC'
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
ISOLATED FUNC worker(t AS ThreadWorker OF String RES fs::File TO Integer, seed AS String) AS Integer
  LET signal = thread::receive(t)
  RETURN 0
END FUNC
FUNC main() AS Nothing
  RES t = thread::start(worker, "", 1, 1)
  RES f = fresh()
  thread::transfer(t, f)
  io::print(read(f))
  thread::send(t, "done")
END FUNC
BASIC
target/debug/mfb build /tmp/mfb-bug-804
/tmp/mfb-bug-804/build/post_transfer.out
```

- Observed: Build exits 0. Runtime exits 0 because the probe catches the error and prints:
``Resource handle was moved to another thread by `thread::transfer` and is no longer usable by the sender.``
- Expected: Build rejects the `read(f)` after the successful transfer with
`TYPE_USE_AFTER_MOVE`. The inline-failure control must still build and print `first`.

Reproduced on macOS AArch64 with a freshly built debug compiler, and again at
unchanged HEAD `ecd062151` using `git worktree add --detach
/tmp/mfb-resource-review-head HEAD` followed by
`CARGO_TARGET_DIR=/Users/justinzaun/Development/mfb/target cargo build
--manifest-path /tmp/mfb-resource-review-head/Cargo.toml` and the same programs.
No other native target has been executed for this report. The source mechanism is in shared
verification/lowering; cross-target behavior still needs the validation below.

## Contrast: inline failure retains the resource

After the primary reproduction's setup, replace only the source with this
full-queue control. It must build and print `first`; the current compiler does.
The worker waits on the data plane, so the first resource fills the capacity-one
resource queue and the second transfer with timeout zero necessarily fails.

```sh
cat > /tmp/mfb-bug-804/src/main.mfb <<'BASIC'
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
ISOLATED FUNC worker(t AS ThreadWorker OF String RES fs::File TO Integer, seed AS String) AS Integer
  LET signal = thread::receive(t)
  RETURN 0
END FUNC
FUNC main() AS Nothing
  RES t = thread::start(worker, "", 1, 1)
  RES fill = fresh()
  thread::transfer(t, fill)
  RES f = fresh()
  thread::transfer(t, f, 0) TRAP(e)
    io::print(read(f))
    RECOVER
  END TRAP
  thread::send(t, "done")
END FUNC
BASIC
target/debug/mfb build /tmp/mfb-bug-804
/tmp/mfb-bug-804/build/post_transfer.out
```

## Root Cause

`src/ir/verify/link.rs:TypeEnv::consumed_resource` explicitly excludes transfer:
it merges `Call` and `CallResult` for close checking, but has no success/failure
move effect for transfer. Its comment refers to a former source checker restoring
the failure state; the current verifier does not supply that effect.
`src/ir/verify/resources.rs:TypeEnv::check_resource_moves` uses this helper and
merges moved sets from falling-through branches. Simply adding transfer to the
existing unconditional helper would reject legal inline handlers, since
`CallResult` is evaluated before the result-test branch.
`src/codegen/cleanup/thread/builder_thread_cleanup.rs:emit_thread_send_runtime_helper_call`
and `src/codegen/runtime/thread/runtime_helpers_thread.rs:thread_queue_write_helper`
already preserve the runtime success/failure distinction.

## Goal

Build rejects the `read(f)` after the successful transfer with
`TYPE_USE_AFTER_MOVE`. The inline-failure control must still build and print `first`.

### Non-goals (must NOT change)

Do not reject inline failure reuse, change runtime transfer/timeout behavior,
remove runtime checks, or weaken existing tests. Do not fix this by redefining
successful transfer as non-consuming.

## Blast Radius

- `TypeEnv::consumed_resource` / `check_resource_moves`: fix direct transfer effects.
- Parent `thread.transferResource` and worker `thread.emitResource`: same effect;
  both must be tested, including decoded package IR.
- Inline `CallResult` followed by `ResultIsOk`: fix with separate success/failure states.
- Fall-through recovery joins: a resource moved on a reachable success path must
  remain unusable after the join; a diverging success path must not poison a
  failure-only continuation.
- Function-level `IrOp::Trap`: retain existing cleanup-induced invalidation of
  local resources; it is distinct from transfer consuming on failure.
- Runtime queueing and closed/moved guards: unaffected; retain them for aliases
  that static analysis cannot prove.

## Fix Design

Represent transfer's move effect on the successful edge. Ordinary calls with
propagating failures move on their normal continuation. Inline traps need the
resource effect attached to the result and applied only to its success branch;
merge reachable continuations using the existing conservative rule. Preserve
alias invalidation on success. Do not blindly mark the resource moved when
lowering or verifying the pre-branch `CallResult`.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/runtime/rt_resource_review_804.rs` reproducing the input above,
      asserting the expected result rather than the observed failure.
- [x] Add syntax and runtime fixtures for success, failure, alias, and nesting
      siblings in the blast-radius list; demonstrate RED before editing behavior.
- [x] Complete the listed sibling audit, recording which cases fail and why.

Acceptance: the primary regression fails for the documented reason, and each
sibling has a measured outcome or an explicit untested status.
Commit: —

### Phase 2 — the fix

- [x] Implement the design above and all confirmed in-scope sibling fixes.
- [x] Preserve the non-goals and add matching valid/invalid function fixtures.
- [x] Update the owning language/memory/architecture topics to the implemented
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

- Prefer success-edge effects in the IR verifier; choose how the result-to-effect association survives inline-trap lowering and package decoding.

## Summary

The repair crosses ownership analysis and observable resource behavior. This
report preserves a runnable reproduction and a test-first implementation plan;
the accompanying specification change describes current behavior, not a shipped
compiler fix.

## Resolution (2026-10-03)

Commits: `ded6e1445` (sibling-arm entry state), `d7d273c94` (transfer success edge).

**Fix.** `TypeEnv::transferred_resource` (`src/ir/verify/link.rs`) names the
binding a resource-plane transfer sends (`thread.transferResource`, and
`thread.emitResource`). `check_resource_moves` (`src/ir/verify/resources.rs`)
attaches the move to the success edge: a propagating `Call` moves the binding and
its may-aliases on the op's continuation; an inline `CallResult` is remembered by
its `$trap_res` temp and the sibling `If ResultIsOk` starts only its success arm
with the binding moved. The conservative join then rejects a read after recovery.
For a check chain the failing checks' `$trap_failed` flag is recorded
(`transfer_failure_flags`) so the single handler behind it starts without the
success-only move. `consumed_resource` still excludes transfer (it consumes on
every edge).

**Sibling bug fixed first (`ded6e1445`).** Every `IF`/`MATCH` arm ran against the
outer moved set that the previous arm had already merged into, so a close in
`THEN` was a false `TYPE_USE_AFTER_MOVE` in `ELSE` (verified at unchanged HEAD
`91da29ed1`). The inline-TRAP failure arm is that `ELSE`, so this had to be fixed
for the failure handler to stay legal. Arms now start from the construct's entry
state.

**Sibling audit (measured, macOS AArch64, `rt_resource_review_804` + unit tests).**

| Case | Before | After |
| --- | --- | --- |
| Parent `transfer` then read | built; runtime `ErrResourceMoved` | `TYPE_USE_AFTER_MOVE` |
| Worker `transfer` (→ `emitResource`) then read | built; runtime refusal | `TYPE_USE_AFTER_MOVE` |
| May-alias (`RES g = same(f)`) read after transferring `f` | built; runtime refusal | `TYPE_USE_AFTER_MOVE` |
| Read after inline-TRAP join | built; runtime refusal | `TYPE_USE_AFTER_MOVE` |
| Read after check-chain join | built | `TYPE_USE_AFTER_MOVE` |
| Inline failure handler reads/closes `f` | built, prints `first` | unchanged |
| Check-chain handler reads `f` (argument ok, or argument failed) | built | unchanged; prints `first`, `no wait first` |
| Diverging success arm, read after join | — | accepted (unit test) |
| Decoded package IR | no transfer effect | same IR-shape rule (10 unit tests on raw IR) |
| Function-level `IrOp::Trap` | locals closed | unchanged |
| Callee transferring its `RES` parameter | runtime guard | unchanged (runtime guard; `rt_moved_handle_guards` green) |

**Fixture corrected.** `tests/rt-behavior/native/native-resource-thread-accept-rt`
read its database directly after a successful transfer to observe
`ErrResourceMoved`. It now transfers through a `give(RES db, t)` callee (the
untracked-alias case), keeping `used=2000` / `movedCount=2000`; only its
`.ast`/`.ir` goldens move.

**Verification.** `cargo test --release --test rt_resource_review_804` 7/7;
`cargo test --release --bin mfb -- transfer use_after_move …` 38/38 and
`ir::verify::tests` 472/472; related integration targets (15 binaries, incl.
`rt_moved_handle_guards`, `rt_scope_drop_leaks`, `rt_resource_owner_scope`,
`rt_thread_block_arena`, `rt_recursive_thread_transfer`) all pass;
`scripts/test-accept.sh` over the 113 transfer / resource / use-after-move
fixtures passes. Spec: language memory-semantics §14.9, resource-management §15,
threads §16. Other native targets were not run: the change is confined to the
target-independent IR verifier and emits no code.
