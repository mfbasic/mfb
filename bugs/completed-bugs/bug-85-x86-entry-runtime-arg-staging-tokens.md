<!-- Bug document: correctness bug. Root cause FIXED (revert); follow-up open. -->

# bug-85: x86 entry stub & runtime helpers stage call args via result-accessors — plan-34-B Phase 4 exposed it, breaking every x86 program

Last updated: 2026-07-10
Effort: large (the follow-up token audit)

## Symptom (FIXED)

At commit `c098504f` (plan-34-B Phase 4) through `77d290c8`, **every** linux-x86-64
program segfaulted at startup — hello-world included, plus closures, scope-drop,
collections, and TLS. Fixed by reverting Phase 4 (`a23aee06`): x86 hello-world,
closures, scope-drop, collections, control-flow, crypto, and the register-pressure
closure test all run correctly again on the 2227 box.

## Root cause

The shared entry stub (`entry_and_arena.rs`) and the runtime-helper **bodies**
stage outgoing call/syscall arguments using *result*-accessors — `abi::return_register()`
(= `%ret0`), `string_data_register()`/`string_length_register()` (= `x1`/`x2`), and
bare `x0`/`x1`/`x2`. On AArch64 and riscv64 the argument and result registers are
the same file (`x0`==arg0==ret0), so this is byte-identical and correct. On x86-64
they are DISJOINT: `%ret0` → `rax`, but a call's first argument must be in `rdi`.

plan-34-B Phase 4 (`c098504f`) deleted `remap_x86_abi`'s CFG role-inference and
mapped each role token straight to its SysV home. For the package-generating
builders that was correct (Phase 3b tokenized their boundary properly), but the
entry stub and helper bodies were never made token-correct — the inference was the
only thing resolving their `%ret0`-as-argument uses to `rdi` by call/`svc`/`ret`
context. With the inference gone, e.g. the entry stub handed the `getrandom`
buffer, the arena-base pointer, and the RNG seed to their helpers in `rax` instead
of `rdi` → immediate crash before `main`.

## Why it escaped plan-34-B's verification

Phase 4's x86 gate cross-emitted the per-package `.nobj` for 447 tests and found 0
diffs. But `-nobj` is the **user package** object only; the entry stub and runtime
helpers are emitted per-executable and linked in separately, so they were never
byte-checked. Bisection with the FULL executable: `41578ef3` (pre-plan-34) and
`cdc4129f` (Phase 3b: tokens + inference via the seam) both run x86 hello-world;
`c098504f` (Phase 4) segfaults. The package `.nobj` is byte-identical across all
three — the divergence is entirely in the linked-in entry/runtime code.

## The single correct end state a fix produces (follow-up, OPEN)

Re-delete the x86 inference (the plan-34-B Phase 4 goal) *after* auditing every
entry-stub and runtime-helper arg-staging site to use `%arg`/`%sysarg` tokens
instead of result-accessors. The completeness gate is **byte-identity of the full
x86 executable** (not just `.nobj`) against the inference oracle (`cdc4129f`) across
the whole test corpus — a missed site is a silent x86 crash the byte-gate on
packages cannot catch. Until that audit is done, the CFG inference stays as the
mechanism that makes x86 correct.

Partial progress toward the audit (already landed, byte-identical on aarch64):
`entry_and_arena.rs` staged the getrandom buffer / arena_base to RNG_SEED and
ARENA_FILL_SEED via `return_register()`; those four sites now use `abi::ARG[0]`
(commit on the plan-34-C branch).

## Verify the fix

```
target/debug/mfb build -target linux-x86_64 tests/syntax/lexical/parser-hello-world
scp -P 2227 .../parser_hello_world-musl.out test@127.0.0.1:/tmp/hw
ssh -p 2227 test@127.0.0.1 'chmod +x /tmp/hw && /tmp/hw'   # -> Hello World
```

## Related

- Reverted: `c098504f` (plan-34-B Phase 4). Spec `architecture/15_x86_64-instruction-set.md`
  restored to describe the inference.
- `plan-34-B` (`planning/old-plans/`) — its Phase 4 "delete the inference" acceptance
  ("byte-identical on all four targets, including linux-x86_64") was met only for
  package `.nobj`, not the full executable; that gap is the lesson here.
