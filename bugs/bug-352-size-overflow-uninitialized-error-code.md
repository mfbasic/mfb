# bug-352: three size-overflow guards raise an error whose code is a partially-computed allocation size, not an error code

Last updated: 2026-07-18
Effort: small (<1h)
Severity: LOW
Class: Correctness (defective defense-in-depth guard; wrong error code)

Status: Open
Regression Test: tests/ (new) — a codegen assertion that every `*_overflow` label
is followed by `emit_error_code_return`, never `emit_allocation_error_return`

Three size-overflow guards in `replace`, `List OF String` `replace`, and
`strings::join` branch to an `overflow` label that calls
`emit_allocation_error_return()`. That helper is
`emit_error_register_return(RESULT_TAG_REGISTER, …)` — it reads the error **code**
out of the result-tag register (`x0`), which is correct only after a failed
`_mfb_arena_alloc` call has left an error code there. At these labels the
allocation call has **not run**. `x0` instead holds a partially-computed
allocation size, which is then moved into the error-code argument and surfaced to
the user as the error's code.

These are defense-in-depth guards added by bug-60 to stop a wrapped size from
producing an undersized allocation that the copy pass then overruns. The guard's
*control flow* is correct — the overrun is still prevented. Only the error code it
reports is garbage. **Severity is LOW because the guards are not realistically
reachable**: firing one requires a 64-bit unsigned wrap in a byte-length or
element-count computation, i.e. lengths near 2^64. No practical input reaches it,
which is also why it has never been observed. It is filed because a
defense-in-depth guard that fires with a corrupt payload is worth exactly as much
as the payload — and the correct idiom is already used at 18 sibling sites in the
same files, with comments that explain precisely why the register form cannot be
shared.

The single correct behavior a fix produces: a size-overflow guard raises
`ERR_OUT_OF_MEMORY_CODE` with `ERR_ALLOCATION_MESSAGE`, the same catchable
allocation error an impossible allocation raises — matching the 18 sites that
already do this.

References:

- `bugs/completed-bugs/bug-60-*` — added these overflow guards (the "trap a 64-bit
  wrap so the copy pass cannot overrun the (undersized) allocation" comments).
- `src/target/shared/code/builder_strings_builtins.rs:131-133` and `:343-345` — the
  correct sites' comments, which state the exact reason the register form is
  invalid here.
- Found during the cleanup review, agent 02 (string/conversion builders),
  INCIDENTAL item, flagged "REAL BUG".

## Failing Reproduction

**A runtime reproduction is not achievable within practical memory limits**, and
that is an honest part of this bug's severity. Triggering the guard needs
`output_len` to wrap 64 bits — for `strings::join`, a total joined length near
2^64 bytes; for `lower_list_replace`, an element count near 2^58. Neither is
constructible. Per the template's allowance, the defect is demonstrated by
inspection of the **emitted instruction stream**, which is unambiguous.

```
# /tmp/ovf/src/main.mfb
IMPORT io
IMPORT strings
FUNC main AS Integer
  LET parts AS List OF String = ["a", "b", "c"]
  io::print(strings::join(parts, "-"))
  io::print(strings::replace("hello", "l", "L"))
  RETURN 0
END FUNC

$ mfb build --ncode .
Wrote native code plan to ./ovf.ncode
```

Emitted stream for `strings::join` — the branch into the guard, and the guard:

```
443 {"op":"add_imm","dst":"x0","src":"x8","imm":"9"}          # x0 = output_len + 9
444 {"op":"ldr_u64","dst":"x8","base":"sp","offset":"1088"}
445 {"op":"cmp","lhs":"x0","rhs":"x8"}                        # wrap test
446 {"op":"b.lo","target":"strings_join_overflow_22"}         # -> guard, x0 = the size
...
642 {"op":"label","name":"strings_join_overflow_22"}
643 {"op":"mov","dst":"x3","src":"x0"}                        # x3 = error CODE arg
644 {"op":"mov_imm","dst":"x1","type":"Integer","value":"7"}  # line
645 {"op":"mov_imm","dst":"x2","type":"Integer","value":"13"} # column
```

Identical shape for `replace`:

```
1264 {"op":"add_imm","dst":"x0","src":"x8","imm":"9"}
1266 {"op":"cmp","lhs":"x0","rhs":"x8"}
1267 {"op":"b.lo","target":"replace_overflow_72"}
...
1463 {"op":"label","name":"replace_overflow_72"}
1464 {"op":"mov","dst":"x3","src":"x0"}
```

- Observed: instruction 443 writes the computed size into `x0`; the wrap test
  branches to the guard; the guard's first instruction moves `x0` — still the size
  — into `x3`, the error-code argument of `_mfb_make_error_result`. The raised
  error's code is `output_len + 9`.
- Expected: `x3` holds `ERR_OUT_OF_MEMORY_CODE` (`77010001`), as at the 18 correct
  sites.

The other two entry edges into the same label make it worse in a second way:
`strings_join_overflow_22` is also reached from instructions 419 and 431, which are
`b.lo` after a `cmp` that does **not** write `x0` at all. On those edges `x0` holds
whatever the surrounding code last left there — genuinely uninitialized with
respect to an error code, and a different garbage value per edge.

Contrast cases that are correct today — the *same* helper, the *same* file, the
same `size_overflow` label pattern, all 18 using `emit_error_code_return`:
`builder_strings_builtins.rs:135,347,586,873,1752`;
`builder_collection_queries.rs:529,845,1186`;
`builder_collection_mutate.rs:604,996,1377,1712,2498,2819,3182,3625,3912`.

Two of those carry comments that diagnose this bug before it was filed:

- `builder_strings_builtins.rs:131-133` — "A size wrap reports the same 77010001 an
  impossible allocation would (**x0 does not hold an error code before the call**,
  so the register-based return above cannot be shared)."
- `builder_strings_builtins.rs:343-345` — "it cannot share the register-based return
  above (**x0 holds the failed call's tag there, not an error code, before the call
  ever runs**)."

Meanwhile the three broken sites carry a comment claiming the opposite outcome —
"raises the same catchable allocation error as an oversized request" — which is
what the code was *meant* to do and is not what it does.

## Root Cause

`src/target/shared/code/builder_codegen_primitives.rs:298-300`:

```rust
pub(super) fn emit_allocation_error_return(&mut self) -> Result<(), String> {
    self.emit_error_register_return(RESULT_TAG_REGISTER, ERR_ALLOCATION_MESSAGE)
}
```

`RESULT_TAG_REGISTER` is `abi::RET[0]` (`src/target/shared/code/error_constants.rs:25`),
i.e. `x0`. `emit_error_register_return`
(`builder_codegen_primitives.rs:767-778`) emits
`abi::move_register(abi::ARG[3], code_register)` — the error code argument is taken
verbatim from `x0`.

That contract is satisfied only on the *post-call* edge: after
`branch_link(ARENA_ALLOC_SYMBOL)` returns a non-OK tag, `x0` holds the allocator's
error code. The three broken sites place the `overflow` label on a **pre-call**
edge and call the same helper:

- `src/target/shared/code/builder_strings.rs:217-218` (`lower_replace`) — label
  reached from `emit_checked_size_add` at `:184` and
  `emit_checked_size_add_immediate(abi::return_register(), output_len, 9, …)` at
  `:199`.
- `src/target/shared/code/builder_strings.rs:462-463` (`lower_list_replace`) — label
  reached from `:406`, `:409`, and the three checked ops at `:432-444`, of which
  `:433-438` and `:439-444` both write **directly into `abi::return_register()`**.
- `src/target/shared/code/builder_strings_builtins.rs:1485-1486` (`lower_strings_join`)
  — label reached from `:1444`, `:1451`, and
  `emit_checked_size_add_immediate(abi::return_register(), …)` at `:1467`.

The checked-size helpers (`builder_codegen_primitives.rs:335-359`) compute into
`dst` **before** testing and branching (`add_registers`/`add_immediate`, then
`compare`, then `branch_lo`), so when `dst` is the return register the branch is
taken with the size already deposited in it. That is why the emitted stream shows
`add_imm x0, x8, #9` immediately before the `b.lo`.

The correct sites are immune because they emit
`emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)`
(`builder_codegen_primitives.rs:375-383`), which materializes the code into a fresh
register first.

**Correcting the audit trail:** the cleanup reviewer reported the sites as "split
roughly evenly between the two idioms." That is wrong, and the real ratio matters
for the fix. An exhaustive search of overflow-labelled returns finds **3 incorrect
against 18 correct**. This is three missed sites against an established, documented
convention — not a codebase-wide ambiguity about which idiom is right.

## Goal

- All three overflow labels raise `ERR_OUT_OF_MEMORY_CODE` with
  `ERR_ALLOCATION_MESSAGE`.
- A codegen-level guard makes it impossible for a future overflow label to reach
  `emit_allocation_error_return`.

### Non-goals (must NOT change)

- The overflow **detection** and control flow (bug-60's actual protection) — the
  guards correctly prevent the undersized-allocation overrun today and must keep
  doing so.
- The 18 correct sites, and the post-call `emit_allocation_error_return` uses,
  which are correct and must stay register-based.
- `emit_allocation_error_return`'s signature and semantics — it is right for its
  contract; the bug is calling it off-contract.
- Do NOT "fix" this by making `emit_allocation_error_return` take a code parameter
  and updating all 69 call sites; that churns 66 correct sites to fix 3.
- Emitted output must be unchanged on every non-overflow path — the only byte
  delta should be inside the three overflow blocks.

## Blast Radius

Search: every `emit_allocation_error_return` call and every overflow-label return.

- `builder_strings.rs:218` (`lower_replace`) — **broken**, fixed by this bug.
- `builder_strings.rs:463` (`lower_list_replace`) — **broken**, fixed by this bug.
- `builder_strings_builtins.rs:1486` (`lower_strings_join`) — **broken**, fixed by
  this bug.
- The remaining 66 `emit_allocation_error_return` calls across
  `src/target/shared/code/` — all on post-`ARENA_ALLOC_SYMBOL` edges where `x0`
  legitimately holds the allocator's error code. Unaffected; verified by the
  search that isolated the three above.
- The 18 `emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, …)` sites — already
  correct; they are the fix template.
- `emit_checked_size_multiply` / `_add` / `_add_immediate`
  (`builder_codegen_primitives.rs:318-359`) — the branch sources. Not changed, but
  their "computes into `dst` before branching" behavior is what makes a
  return-register `dst` hazardous, and is worth a doc note.

## Fix Design

Replace the three `emit_allocation_error_return()?` calls that follow an overflow
label with
`emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?`, and
replace the three misleading comments with the accurate wording already used at
`builder_strings_builtins.rs:131-133`.

Then add the guard that would have caught this: a codegen-level check asserting
that no instruction sequence places `emit_error_register_return(RESULT_TAG_REGISTER, …)`
on a path from an overflow label that has no intervening `ARENA_ALLOC_SYMBOL` call.
The cheap, robust form is a source-level architecture lint in the existing
`architecture_guards` style — every `abi::label(&…overflow…)` must be followed by
`emit_error_code_return`.

Rejected alternatives:

- **Zero `x0` before branching to the overflow label.** Turns garbage into a
  *wrong-but-stable* code (`0` = OK tag), which is worse: it would raise an error
  claiming success.
- **Have the checked-size helpers avoid the return register as `dst`.** Fixes one
  of the three edges per site and leaves the other edges (419/431 in the dump)
  still carrying stale `x0`. Treats a symptom.
- **Parameterize `emit_allocation_error_return`.** Rejected under Non-goals.

Expected output shift: three overflow blocks gain a `mov_imm` of the code
constant. Every other byte on every target unchanged.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a codegen test that lowers `strings::join`, `replace`, and the
      `List OF String` `replace`, locates each `*_overflow` label in the emitted
      stream, and asserts the first instruction after it is not a bare
      `mov x3, x0`. Confirm all three fail today.
- [ ] Confirm the 3-vs-18 split above by exhaustive search; record the verdict per
      site in this file.

Acceptance: three failing assertions for the documented reason; audit complete.
Commit: —

### Phase 2 — the fix

- [ ] Swap the three calls to
      `emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)`.
- [ ] Replace the three inaccurate comments with the `:131-133` wording.
- [ ] Add the architecture lint forbidding `emit_allocation_error_return` directly
      after an overflow label.

Acceptance: Phase 1 tests pass; the lint fails if any site is reverted.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Re-dump `--ncode` for the reproduction program; confirm each overflow label is
      now followed by a `mov_imm` of `77010001` into the code register.
- [ ] `scripts/artifact-gate.sh`; confirm the only artifact delta is inside the
      three overflow blocks on each target.
- [ ] Full acceptance suite on macOS + Linux (aarch64/x86_64).

Acceptance: full suite green; artifact delta is exactly the three overflow blocks.
Commit: —

## Validation Plan

- Regression test(s): the emitted-stream assertion over the three lowerings, plus
  the architecture lint (which generalizes the guard to future sites).
- Runtime proof: not achievable — the guard needs a 64-bit size wrap. The
  instruction-stream diff at the overflow label is the proof, and this is recorded
  as a known limit of the test rather than papered over.
- Doc sync: the three inaccurate source comments. No spec change — no spec
  documents the overflow code.
- Full suite: `scripts/artifact-gate.sh` then `tests/test-accept.sh` per target.

## Open Decisions

- Enforce via source-level architecture lint (recommended — cheap, catches new
  sites at review time) vs. emitted-stream assertion only (proves behavior but only
  for the three lowerings that have tests).

## Summary

`emit_allocation_error_return` reads the error code from `x0`, which is only valid
after a failed allocator call. Three overflow labels call it *before* the allocator
runs, at a point where `x0` holds a partially-computed size — proven directly in
the emitted stream (`add_imm x0, x8, #9` … `b.lo overflow` … `mov x3, x0`). The
fix is a three-line swap to the idiom 18 sibling sites already use, whose own
comments explain exactly why the register form is invalid here. Severity is LOW
because the guards need a 64-bit size wrap to fire and so are not practically
reachable; the value is in repairing a defense-in-depth path that would otherwise
report nonsense at the one moment it matters, and in adding the lint that stops the
fourth site from happening.
