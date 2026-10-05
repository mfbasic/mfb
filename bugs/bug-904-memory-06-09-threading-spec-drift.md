# bug-904: memory 06–09 and threading spec drift — stale `.result` helper rule, wrong `MOUSE_STATE_SLOTS` count, closure x28 save and thread-entry wording

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (spec/doc)

Status: Open
Regression Test: none (doc-only; verify by re-rendering `mfb spec memory …` / `mfb spec threading …`)

Several statements in the memory 06–09 and threading topics no longer match the
code. Each item gives the current spec text, the evidence, and the correction.
Spec errors that come with a code bug (the § Lifetime sentence in threading 07, the
trampoline's missing program-exit arm in threading 06, the stdin subscription order
in memory 08) are written up in their own B09 bug drafts and are not repeated here.

References:

- Found during spec hardening batch B09.

## Items

1. **`src/docs/spec/memory/07_runtime-helper-abi.md` § Required-Helper Analysis:**
   "**`.result` member access.** Reading the `result` member of a value pulls in
   the `Thread` family, because thread-result materialization is a thread helper.
   [[src/target/shared/runtime/usage.rs:push_value_helpers]]". This rule is stale. In
   `push_value_helpers`, the `IrValue::MemberAccess` arm says: "No `.result`
   heuristic: `Thread.result` was removed from the language
   (TYPE_THREAD_RESULT_REMOVED) … a pure false positive that rejected valid programs
   (bug-119)". Threading 11 also says "there is no `t.result` field". **Fix:** delete
   the bullet and say there is only one non-obvious source (resource-union binds).
   The heading's "two non-obvious sources" also changes.

2. **`src/docs/spec/memory/08_program-startup.md` § The conditionally-reserved region
   chain:** "**Mouse state** — `MOUSE_STATE_SLOTS` (8 words: the decoded-event ring
   pointer, its head and tail cursors, and the stdin decoder's partial-sequence
   buffer)". In code,
   `src/codegen/error/constants/error_constants.rs:MOUSE_STATE_SLOTS` is
   `(MOUSE_STATE_PARSE_BUF_OFFSET + MOUSE_STATE_PARSE_BUF_BYTES) / 8` = `(40 + 32) / 8`
   = **9** words. The listed fields also leave out the replay cursor
   `MOUSE_STATE_DRAIN_POS_OFFSET` (32). Evidence: `grep -n "MOUSE_STATE_" .../error_constants.rs`.
   **Fix:** "9 words", and list the drain cursor.

3. **`src/docs/spec/memory/09_closures.md` § The Environment Register:** "the caller
   **saves its own x28 to a stack slot before the call and restores it afterward**",
   and the pseudo-code shows `save x28 -> [sp + saved_env_slot]`. The generated code
   (`otool -tV` of a `--debug` build, `_mfb_fn_apply` calling a `FUNC` parameter)
   keeps the saved env in an allocator-chosen register instead:
   `mov x23, x28 ; ldr x9,[x8] ; ldr x10,[x8,#8] ; mov x28, x10 ; blr x9 ; mov x28, x23`.
   x23 is callee-saved and is saved in the prologue. **Fix:** "saves its own x28 in a
   temporary that survives the call (a callee-saved register or a spill slot, chosen
   by the allocator)".

4. **`src/docs/spec/memory/09_closures.md` § Closures Across Threads:** "When a
   closure is dispatched as a worker thread entry, the thread trampoline restores the
   closure environment …". A closure can never be a thread entry. Threading 01 says
   "The compiler rejects: Lambdas and closures as thread entry points (only a function
   identifier is accepted)". B09 confirmed this: a `LAMBDA` passed to `thread::start`
   gets `thread.start entry point must name an ISOLATED FUNC.`. So the entry
   descriptor is always a static `FunctionRef` with `env = 0`. The trampoline's
   load sequence is as described (`ldr x13,[x20,#0x98] ; ldr x28,[x13,#8] ; ldr x13,[x13]`).
   **Fix:** say the entry is always a `FunctionRef` descriptor, and that x28 is loaded
   from its env word (always 0) so the entry is called the same way as any function
   value.

5. **`src/docs/spec/memory/06_native-calling-convention.md` § Float and Fixed
   arguments:** "An operand's bits move out of its `x` register with `fmov d, x`
   (`fmov_d_from_x`), the `fadd`/`fmul`/etc. runs". In `_mfb_fn_many` (B09 `abi`
   probe, `-O1`), the incoming `Float` parameter is spilled from `x1` to its slot and
   then read with `ldr d0, [sp, #0x28]` straight into the FP register. There is no
   `fmov`. This is an imprecision, not an ABI break. **Fix:** "moves into a `d`
   register (`fmov d, x` from a GPR, or a direct `ldr d` from its slot)".

6. **`src/docs/spec/memory/08_program-startup.md` (sequence completeness):** the entry
   sets the main arena's stack-limit word right after publishing
   `_mfb_rt_main_arena` and before signal setup. Disassembly of `_main` shows
   `pthread_self` → `pthread_get_stackaddr_np` → `pthread_get_stacksize_np` → store
   at arena `+0x690`. The topic never mentions this step, even though threading 09
   says "every platform's trampoline does the same". **Fix:** add a "Stack limit" step
   that links to `./mfb spec language functions` (recursion).

7. **`src/docs/spec/threading/07_control-block.md` § The block arena / 06 § thread::start**
   (minor): queue rings are allocated eagerly at `capacity * 8` bytes and filled in
   full (entropy fill), so a large valid limit costs memory up front. Measured with
   `/usr/bin/time -l`: limit 1e8 gave about 1.6 GB max RSS for an idle worker, and
   limit 2^40 drove RSS past 20 GB before the process was killed, instead of
   returning `ErrOutOfMemory`. The spec only says "the limit is upper-bounded so the
   product cannot wrap". **Fix (doc):** state that the ring is committed eagerly and
   the cost scales with the limit. Whether to cap the limit or allocate lazily is an
   owner decision.
