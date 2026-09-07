# bug-567: a user function that RETURNS a `&` concatenation leaks its block, 64 B per call

Last updated: 2026-09-07
Effort: small–medium
Severity: **HIGH** (unbounded leak on the commonest String-returning function shape)
Class: Memory / correctness

Status: **FIXED**
Regression Test: `tests/runtime/rt_scope_drop_leaks.rs` —
`returning_a_nested_concat_runs_at_constant_rss`,
`returning_a_concat_of_a_call_result_runs_at_constant_rss`,
`returning_a_collection_built_from_interior_concats_runs_at_constant_rss`,
`returning_an_interior_temp_beside_a_live_local_runs_at_constant_rss`, the
POSITIVE pins `returning_a_two_operand_concat_still_runs_at_constant_rss` and
`a_failing_trap_does_not_leak_its_interior_temp`, the value pin
`every_returned_concat_shape_still_produces_the_right_value`, and the five
owner counts in `tests/codegen/codegen_return_interior_temp_drop.rs`

Found while fixing bug-561. Reproduces unchanged on `19880284452` and after both
bug-560 and bug-561.

## The finding

```
FUNC f(n AS Integer) AS String
  RETURN "v" & toString(n MOD 10)     ' <- a concat, not a bare call
END FUNC
```

Called in a loop, bound or unbound, this leaks **64 B per call**.

| program | 200k | 400k |
| --- | --- | --- |
| `acc = acc + len(f(i))`, `RETURN "v" & toString(n MOD 10)` | **13.3 MB** | **25.6 MB** |
| `LET s AS String = f(i)`, same body | **13.3 MB** | **25.6 MB** |
| contrast: same call shapes, body `RETURN toString(n MOD 10)` | 1.0 MB | 1.0 MB |

(macOS arm64, peak RSS via `/usr/bin/time -l`.)

The ONLY difference between the leaking and the flat program is the callee's
return expression: a `Binary { & }` versus a bare `Call`.

## Why bug-536 shape B-2 does not cover it

B-2's `tests/rt_scope_drop_leaks.rs` cases all return either a bare call
(`SHAPE_B2_TRANSITIVE`: `RETURN toString(i)` / `RETURN leaf(i)`), a `MUT` local
(`SHAPE_B2_APPEND_ARGUMENT`), or a literal (`SHAPE_B2_RETURNED_LITERAL`). None
returns a concatenation, so the corpus has a hole exactly where the commonest
real body sits. `SHAPE_B_CONCAT` covers `len("x" & toString(i) & "y")` but
*inline in `main`*, where it is flat — so the concat producer itself is fine and
the defect is in what happens to that block at the RETURN/caller seam.

## Where to look

`lower_returned_value`'s four return shapes (bug-536 B-2's table): a concat
result is "a claimed pending temp" — the producer's own `arena_alloc`, marked by
`mark_fresh_string`. Check whether the claim actually matches at a `RETURN`
whose operand is a `Binary`, and whether `function_returns_fresh_string` then
licenses the caller's free (it should: `f` returns a value, is not a param
borrow, is not callback-referenced). One of those two links is not connecting.

## What a fix must produce

`RETURN <concat>` runs at constant RSS at both counts, bound and unbound, and the
existing B-2 shapes stay flat with no second free (`arena_free` on a block a
caller still owns is a use-after-free, not a leak).

Measure as peak RSS at N and 2N, never a one-shot.


## Root cause — from bug-570, which was filed for the same defect and folded in here

An agent working bug-562 reproduced this shape independently and filed it as
bug-570 before bug-567 had landed. Same defect; bug-570 is withdrawn and its
analysis is kept here, because it is sharper than what this document had.

**`clear_pending_temps_to` truncates *every* pending temp above the watermark at
a control transfer.** Its justification is two clauses, and the second is false:

* *"the returned temp is moved to the caller"* — true, but only of the ONE temp
  `claim_pending_temp` has already popped.
* *"an interior free would be unreachable"* — **false.** It is unreachable only
  because it would be emitted *after* the branch. That is a property of where the
  code is placed, not of the program.

So an INTERIOR temp — one that is not the returned value — is dropped without
being freed.

That predicts exactly which spellings leak, and the prediction was measured:

| shape | interior temp? | 200k / 400k |
| --- | --- | --- |
| `RETURN "<" & s & ">"` | yes (the inner concat) | 25.6 MB / 50.2 MB |
| `RETURN "v" & toString(n MOD 10)` | yes (`toString`'s result) | 13.3 MB / 25.6 MB |
| `RETURN s & ">"` | **no** — both operands are already-owned values | 1.0 MB / 1.0 MB |

The flat row is the control: one operator fewer, no interior temp, no leak.

**A fix here ADDS frees**, which is the double-free direction, so it needs its own
enumeration of what is and is not interior — not a relaxation of the truncation.
Neither bug-562's change nor bug-536 shape B-2 moves these numbers at all.


## The fix (2026-09-07)

`clear_pending_temps_to`'s second justification was false, exactly as bug-570
said, and the correction is a matter of PLACEMENT: the interior free is emitted
inside `emit_return_exit_inner`, while the code is still reachable, instead of
after the `ret`.

**The soundness argument is not new.** It is the argument that already licenses
`drop_pending_temps_to` at the end of every ORDINARY statement: a registered
pending temp is by construction a fresh, solely-owned arena block
(`register_pending_temp` admits nothing else), and `LET x AS String = wrap("ab")`
already frees the identical interior block at statement scope. A `RETURN` changed
nothing about ownership — only about where the free would land. So the fix does
not relax the truncation; it moves the free to the last reachable instruction
slot, once the escaping value is standalone (claimed, moved by
`plan_returned_move`, or deep-copied by `lower_returned_value` /
`store_pending_success_result`).

On top of that argument, each interior free carries a **runtime pointer-identity
compare** against the parked escaping block (`return_temp_escaped`), so even a
lowering that handed the return the same pointer as an interior temp cannot have
it freed underneath the caller. Soundness is local to four instructions rather
than a whole-program claim. `drop_interior_temps_before_branch` emits **nothing
at all** when no interior temp is pending, so every `RETURN` in the tree that was
not this bug is byte-identical.

### The enumeration, and why `Fail` is not in it

`op_transfers_control` is gone; a single classifier `transfer_temp_disposition`
answers one question per `NirOp` — *what becomes of the temps this statement
registered* — as an EXHAUSTIVE match with no wildcard arm, so a new statement
kind is a build error until someone decides. `TransferTemps` has five classes:

| class | statements | why |
| --- | --- | --- |
| `StatementScope` | everything that falls through | freed by `drop_pending_temps_to`, as before |
| `FreedBeforeBranch` | `Return` | the fix; **checked** — reaching the end of the statement with temps left is now a codegen error, not a truncation |
| `NoExpression` | `ExitLoop`, `ContinueLoop` | they carry only a `LoopKind`, so they can register nothing; also checked |
| `AdoptedByTheCatcher` | `Fail` | see below |
| `ProcessTerminates` | `ExitProgram` | O(1) leak into a heap about to be torn down |

`Fail` must keep truncating: `emit_direct_error_return` parks the `Error` block in
the per-thread current-error slot *precisely because* the control transfer forgets
it, and the catcher frees it exactly once (design "b"). Freeing it there is a
double free, not a leak fix. `the_transfer_temp_classifier_has_no_wildcard_arm`,
`every_nir_op_is_classified_by_name` and
`the_control_transfer_classes_are_the_measured_ones` hold the vocabulary total.

### The doc's residual-leak prediction did not survive measurement

This document predicted that a `FAIL` with an interior temp would keep leaking it.
**It does not.** `error(...)`'s own constructor lowering already frees the message
temps before the branch, and two independent measurements say so.

First, on the pre-bug-565 compiler, where the error path was still losing ~780 B
per trapped error and any interior leak would have to be read against it:

| shape | 20 000 trapped errors | 40 000 |
| --- | --- | --- |
| `FAIL error(7, repeat("x",4000) & "y")` — one interior 4 KB temp | 626 MB | 1251 MB |
| `FAIL error(7, repeat("x",4000))` — no interior temp | 626 MB | 1251 MB |

Equal to within 16 KB, where an abandoned 4 KB block would have shown as +80 MB.

Then, with bug-565 landed (`38b855905`), the same shape is simply **flat** — 1.0 MB
at 5 000 and at 10 000 trapped errors, where an abandoned 4 KB block would be
40 MB. `a_failing_trap_does_not_leak_its_interior_temp` pins the flatness AND the
equality against the no-interior contrast, because flatness alone cannot say the
block was correctly declined rather than wrongly freed. The half that says
"declined" is `a_fail_is_never_given_an_interior_free`, which reads the decision
off the emitted code: the interior-temp `FAIL` emits ZERO `return_escaping_value`
parks and the same two `_mfb_rt_drop_owned_string` calls the base compiler emitted,
against zero for the no-concat contrast.

### Measured

macOS arm64, peak RSS via `/usr/bin/time -l`. Reproduced byte-identically on
`ac421788a` and on `38b855905` (after bug-561/565/568 landed), so this is
neither caused nor fixed by any of them:

| shape | interior temp? | before 200k / 400k | after |
| --- | --- | --- | --- |
| `RETURN "<" & s & ">"` | yes (inner concat) | 13.3 / 25.6 MB | **1.0 / 1.0 MB** |
| `RETURN "v" & toString(n MOD 10)` | yes (`toString`'s result) | 13.3 / 25.6 MB | **1.0 / 1.0 MB** |
| `RETURN s & ">"` | **no** | 1.0 / 1.0 MB | 1.0 / 1.0 MB |

The report's 25.6/50.2 MB row for the nested shape does not reproduce at 64 B per
call: the shape as written here has ONE interior temp and leaks 64 B, matching
13.3/25.6. `mfb build -ncode` on the pre-fix compiler shows `wrap` calling
`_mfb_rt_string_concat` twice and `_mfb_rt_drop_owned_string` zero times, which is
the mechanism read directly off the emitted code.

The largest shape found is not in the report at all: `RETURN ["a" & toString(n),
"b" & toString(n)]` registered FIVE pending temps and emitted ZERO frees — four
blocks per call, not one.

### Golden attribution

Per function against a build of the base commit, over all 17 fixtures whose
goldens move (identical results against `ac421788a` and against `38b855905`):

**87 functions changed. Every one gained ≥1 free. None gained an allocation
(`_mfb_arena_alloc` delta is 0 in all 134 rows). `dataObjects` byte-identical in
all 17 fixtures. No function changed without gaining an owner.**

59 of the 87 also gained exactly one `return_escaping_value` park (the register
fast path); the other 28 are the cleanup-bearing `RETURN` path, where the escaping
value is already in `pending_result_slots.value` and no park of its own is needed.

The guard-label count equals the free count in every function but one:
`_mfb_fn_$lambda7` (`collections::forEach(ints, LAMBDA(x) -> io::print(toString(x)))`)
gained 1 free and 0 guards, because its implicit `RETURN` carries `Nothing` — there
is no escaping block to guard against.

### Gates

Rebased onto `38b855905` (after bug-561/565/568 landed) and re-run there:
`cargo test --release --no-fail-fast` 158 binaries, **5004 passed, 0 failed**;
`artifact-gate.sh all` 1412 tests, 1578 builds, 1973 goldens, **0 diffs** after
`regen-ncodesum.sh` refreshed 144 (78 of them changed against main);
`test-accept.sh` **1434 tests passed**; `cargo fmt --all --check` and
`cargo check --all-targets` clean.
