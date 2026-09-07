# bug-568: a SCALAR payload under an inline `TRAP` still leaks ~134 B per call — bug-561's residual

Last updated: 2026-09-07
Effort: medium
Severity: **HIGH** (unbounded leak; `TRAP` over an `Integer`-returning function is a very common shape)
Class: Memory / correctness

Status: **FIXED** (2026-09-07, `1bf2a4a94`)
Regression Test: `tests/runtime/rt_scope_drop_leaks.rs` —
`a_trap_over_a_param_returning_callee_runs_at_constant_rss`,
`a_trap_over_every_scalar_payload_runs_at_constant_rss`,
`a_trap_over_a_param_returning_string_callee_runs_at_constant_rss`, the POSITIVE
pin `a_param_borrow_without_a_trap_still_runs_at_constant_rss`, and the 25-run
behaviour pin `every_param_borrow_shape_still_produces_the_right_value`;
`tests/codegen/codegen_trap_result_wrapper.rs` — four counts, one of them
negative, plus the totality assertion
`the_result_wrapper_has_exactly_one_constructor`.

## The correction to the report

The report's title is right about the numbers and wrong about the cause: **it is
not about scalars.** The discriminator is the CALLEE's `RETURN` shape.

| callee body | leaks under `TRAP`? |
| --- | --- |
| `RETURN i` (a parameter) | **yes** |
| `RETURN i + 0` (folds to the parameter) | **yes** |
| `RETURN i / 2` | no |
| `LET r AS Integer = i` then `RETURN r` | no |

That is why bug-561 recorded "`Result OF Integer` never leaked at all" and why
this report says it always does: bug-561's contrast case is `RETURN n / 2`, and
this report's producer is `RETURN i`. Both measurements are correct.
A `String` payload leaks on the same rule, harder — 195 B per call rather than
134 B, because the wrapper inlines the whole string.

## Root cause

The inline-`TRAP` bind is `$trap_resN : Result OF T = CallResult(risky(i))`, and
`lower_value_owned` decides whether an owning store must deep-copy by asking
`value_needs_owning_copy`. For a call that is
`call_returns_param_borrow(target)` — "the block this callee returns is the
CALLER's own argument, so an owner must copy it before it can free it."

**That is a statement about the callee's SUCCESS value, and on a `CallResult` the
lowered value is not the callee's success value.** Every inline-`TRAP` lowering —
the direct user/`.mfb` callee, the indirect `FUNC` value, and
`materialize_current_result` for an inline builtin or a runtime helper — ends
holding the `{tag @0, size @8, payload @16}` block `emit_build_result_inline`
just allocated, with the callee's value COPIED into it. The wrapper is this
frame's own fresh block, aliasing nothing.

So the bind deep-copied a block that needed no copy, and dropped the original on
the floor. One `_mfb_arena_alloc` and one `flat_copy` per call, forever.

## Measured (macOS arm64, `/usr/bin/time -l` peak RSS, release)

| program (loop body) | N=200 000 | N=400 000 | per call |
| --- | --- | --- | --- |
| `LET n AS Integer = risky(i) TRAP …`, `risky` = `RETURN i` | **26.8 MB** | **52.6 MB** | 134 B |
| all four scalar payloads (`Integer`, `Float`, `Boolean`, `Byte`), four traps per iteration | **99.5 MB** | **197.8 MB** | 134 B x 4 |
| `LET n AS String = pick(base, i) TRAP …`, `pick` = `RETURN s` | **38.2 MB** | **75.4 MB** | 195 B |
| **after, all three** | **1.0-1.1 MB** | **1.0-1.1 MB** | 0 |
| CONTROL: the same param-borrow callee with **no `TRAP`** | 1.06 MB | 1.06 MB | 0, before and after |

`/usr/bin/time -l` peak RSS, macOS arm64, release, at two counts because a
one-shot cannot tell a leak from an allocator high-water mark. The control is
what makes the numbers attributable: the identical callee, the identical loop,
the identical borrowed argument — only the `TRAP` removed — is flat both before
and after, so the leak is the trapped BIND and not the borrow.

The three leaking rows are **byte-identical (26 820 608 / 52 625 408 bytes) on
plain `main` and on the bug-565 compiler**, which is expected: the producer here
never fails, so nothing bug-565 touches is on this path.

## The fix

`is_fresh_trapped_result_wrapper(value, result_type)` — a `NirValue::CallResult`
whose lowered type is a `ResultOf` — skips the callee-keyed predicates in
`lower_value_owned`. Nothing else changes: the plain-`Call` path still copies,
`register_pending_temp` still answers the same way for every non-`CallResult`
value, and no free is added anywhere.

### Spec

`mfb spec language memory-semantics` §14.1: "Copy creates an independent value
with no shared mutable state", and the §14 preamble: "Each live value is owned by
exactly one binding … Values are reclaimed by deterministic drop at the end of
the owning scope." The wrapper already satisfied both — it is an independent
value and the `$trap_res` binding is its one owner, and `ResultOf` is a freeable
flat value whose scope drop reclaims it. The redundant copy created a SECOND
independent value and left the first with no owner at all. Removing it restores
the one-owner invariant; it adds no free and removes one allocation per call.

### Why this is not a use-after-free

This fix REMOVES a copy, which is the direction that can hand a binding an alias
it will later `arena_free`. Two things make it local:

* **Both halves of the gate are load-bearing, and it fails CLOSED.** The
  `CallResult` node says the lowering took a trapped-`Result` path; the `ResultOf`
  TYPE is the runtime-shape witness that it produced a wrapper. A `CallResult`
  that ever lowered to something else keeps the copy — the old, merely wasteful
  behaviour — rather than losing it.
* **The `ResultOf` type is a total witness, and that is asserted rather than
  assumed.** All three lowerings now build their result through the single
  constructor `CodeBuilder::fresh_trapped_result_value` — a byte-identical
  extraction of what each did inline — which is the only place in `src/codegen`
  that builds a `ValueResult` with a `ParameterType::result_of` type.
  `the_result_wrapper_has_exactly_one_constructor` scans `src/codegen` and
  asserts that, so a fourth lowering returning an aliased `Result` reds a test
  instead of defaulting into the no-copy path. The scan is scoped to
  `src/codegen` deliberately: the same field spelling appears on `IrValue` in
  `src/ir`, a different type whose `Result` node is a lowering description and
  not an arena pointer, so widening the walk would flag it and say nothing about
  ownership.

The plain-`Call` path is untouched and must be:
`a_param_borrow_without_a_trap_still_copies` asserts, as a count against the same
callee that computes instead of borrowing, that the copy is still emitted there.
Removing it would `arena_free` the caller's live local — invisible to every leak
test.

## Golden delta

**Zero.** `artifact-gate.sh all`: **1 412 tests, 1 578 builds, 1 973 goldens
checked, 0 diff(s)** — not one golden moved, against the immediately preceding
commit (bug-565) as the baseline.

That is a result, not an absence of one: 0 diffs over 1 973 `.ncode` sha256s
means every function of all 22 byte-identity packages is BYTE-IDENTICAL, which
is the per-function attribution for the whole covered corpus. Empirically, no
`.mfb` builtin body inline-`TRAP`s a callee that returns a bare parameter (nor
one of the three rodata-`String` internals `call_returns_rodata_string` names),
so nothing in the standard library had the shape. The blast radius is exactly
user programs that do.

The positive attribution is on two programs that DO have it, `-ncode` dumps
diffed function by function against the bug-565 compiler:

| program | functions added/removed | `dataObjects` | changed | the change |
| --- | --- | --- | --- | --- |
| `Integer` param borrow under `TRAP` | 0 / 0 | identical | 1 (`_mfb_fn_main`) | `flat_copy` 1 → 0, `_mfb_arena_alloc` 4 → 3 |
| `String` param borrow under `TRAP` | 0 / 0 | identical | 1 (`_mfb_fn_main`) | `flat_copy` 6 → 5, `_mfb_arena_alloc` 9 → 8 |

Exactly one function changes, and it loses exactly one deep copy and exactly one
allocation. **`_mfb_arena_free` and `_mfb_rt_drop_owned_string` counts are
UNCHANGED in both** — that is the semantics proof for a fix in this direction:
no free was added, none was removed, and the binding that used to own the copy
now owns the original.

## Residual

None found for this shape. The `TRAP` machinery's remaining known leak is the
one bug-565 names: an error raised by an inline builtin's own domain check
orphans the `ErrorLoc` that `_mfb_make_error_result` allocated, on the RAISE side.
