# bug-401: inline-`TRAP` handler continuation-distribution is exponential in handler branch count → super-linear compile time / codegen overflow on a legal program

Last updated: 2026-07-28
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness / compile-time resource exhaustion (code-size explosion)

Status: FIXED (60a012916)
Regression Test: `src/ir/tests.rs::inline_trap_fallthrough_handler_lowers_linearly`
(RED-verified: pre-fix the handler op count grew by 98208 for 10 extra
fall-through `IF`s vs the ≤400 linear bound) plus the runtime fixture
`tests/rt-behavior/trap/inline-trap-fallthrough-recover-rt` locking in the
conditional-`RECOVER` + fall-through semantics.

`distribute_continuation` (`src/ir/lower.rs:1312`), driven by `treeify_handler`
(`:1239`), normalizes an inline-`TRAP` handler so a `RECOVER` falls through
correctly. When a handler statement is a branching statement (`IF`/`MATCH`) whose
branches fall through, the entire remaining continuation (`tail`) is cloned into
**both** the `then` and `else` branch (and, for `MATCH`, into every case body plus
a synthesized `ELSE`), then each copy is recursively treeified. A handler that is
N sequential non-terminating `IF` statements followed by a `RECOVER` therefore
duplicates the continuation 2^N times — an IR body exponential in the source size.

Because the blow-up is in sibling **breadth**, not nesting depth, the bug-183 /
bug-289 parser recursion caps (which bound depth) do not limit it.

References:

- `src/ir/lower.rs:1312` (`distribute_continuation`), `:1239` (`treeify_handler`).
- Distinct from bug-34/156/174/194/286 (prior lower.rs findings) and bug-315
  (regex ReDoS). Found during goal-07.

## Failing Reproduction

```
FUNC g() AS Integer  RETURN 1  END FUNC
FUNC main() AS Integer
  MUT y AS Integer = 0
  LET x = g() TRAP(e)
    IF y > 1 THEN  y = 1  END IF
    IF y > 2 THEN  y = 2  END IF
    ...                     (N such IFs)
    RECOVER 0
  END TRAP
  RETURN x + y
END FUNC
```

Measured `target/debug/mfb build` wall times (verified 2026-07-28):

| N  | time / result |
|----|---------------|
| 8  | 0.22 s ✓ |
| 11 | ~1.3 s ✓ |
| 14 | ~10 s, and fails codegen: `error: AArch64 branch 'b.eq' displacement 1180216 to 'if_else_64' exceeds ±1 MiB` |

- Observed: super-linear (≈2× per +1 in N) compile time; at N≈14 the exploded
  function body overflows the AArch64 ±1 MiB branch range, so a **legal** program
  stops compiling; at higher N the compiler does not terminate in minutes.
- Expected: compile time/size linear (or near-linear) in handler size; the handler
  continuation is shared/joined rather than duplicated per branch.

Contrast (immune): the same handler with terminating branches (each `IF` body ends
in `RECOVER`/`RETURN`) does not duplicate the tail.

## Root Cause

`distribute_continuation` duplicates the full continuation into every fall-through
branch and recurses, so K sequential fall-through branch statements yield 2^K
copies of the tail. There is no join/merge (e.g. lowering the continuation once to
a shared label the branches jump to) and no size budget.

## Goal

- An inline-`TRAP` handler compiles in time and code size linear in its statement
  count: the post-handler continuation is emitted once and shared (a join point /
  label) rather than cloned into every fall-through branch.

### Non-goals (must NOT change)

- The runtime fall-through semantics of `RECOVER` and handler branches must be
  preserved exactly; this is purely a lowering-shape fix.

## Blast Radius

- `src/ir/lower.rs:1312` (`distribute_continuation`) and its `treeify_handler`
  caller (`:1239`) — the duplication site.
- Codegen branch-range check (`src/arch/aarch64/…`) surfaces the symptom but is
  not the bug; do not "fix" by widening branch range.

## STATUS: FIXED (60a012916)

Reproduced exactly as documented (debug `mfb build --ir`, macOS-aarch64): IR body
grew 2858 → 22570 → 180266 lines for N = 8/11/14 (2^N), and N = 14 failed codegen
with `AArch64 branch 'b.eq' displacement 1180216 to 'if_else_64' exceeds ±1 MiB`.

Fix (`src/ir/lower.rs`): `treeify_handler` only distributes the handler
continuation *into* an `IF`/`MATCH` branch when that branch **can reach a
terminator** (`RECOVER`/`RETURN`/`FAIL`/`?`/loop `EXIT`/`CONTINUE`) — the case
where a recovered path must not fall through into the shared continuation. When
no branch can terminate, the branch always falls through, so the continuation is
kept as a single shared **sibling** after the branch instead of being cloned into
every arm. New `block_can_terminate`/`statement_can_terminate` predicates (a
"some path" analysis, distinct from the existing all-paths `block_terminates`)
drive the decision; being conservative only forces distribution, which is always
semantically safe.

Post-fix the same handler lowers **linearly** (≈10 IR lines per `IF`): N = 8/11/14/20/100
→ 133/163/193/253/1053 lines, flat ≈0.01 s compile; N = 14 and N = 100 now build
cleanly (0.07 s / 0.43 s).

Verification:
- Semantics preserved: a fired `RECOVER` still stops later sibling statements
  from running (fixture prints 50/111/1300/1320/1700 — the `flag=1` case recovers
  early and skips the fall-through additions).
- No golden shift: all 17 existing inline-`TRAP`-with-branch fixtures produce
  byte-identical `.ir` (their handler branches all terminate, so the new hoist
  path is never taken for them).
- Full suite green: `cargo test --release` 4198 passed / 0 failed across 39
  binaries; the new acceptance fixture passes `test-accept.sh`.

Deviation from the doc: the "Failing Reproduction" snippet is pseudocode
(single-line `FUNC`/`IF … END IF` don't parse; `str()` is `toString`). The
mechanism and the measured 2^N blow-up / ±1 MiB overflow reproduced exactly once
transcribed to valid multi-line MFBASIC.
