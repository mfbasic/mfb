# bug-1019: a collection temporary passed through a generic function (`idOf(squares(20.0))`) is never freed

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs19-agent-a)
Regression Test: `tests/runtime/rt_param_borrow_handover_frees.rs` (26 `--debug` programs, `alloc_calls == free_calls`, `live_bytes 0`, `double_free_skips 0`), `codegen::engine::analysis::handover::tests::a_parameter_returned_as_a_borrow_is_never_handed_over`

`LET d AS List OF Square = idOf(squares(20.0))`, with `FUNC idOf OF T(x AS T) AS T`, leaks the
argument temporary. bs18-agent-j reports the same leak through `transform(idOf(temp), f)`
(`/tmp/bs18j/tp_gen4.mfb`; not re-measured by the coordinator).

**Correct behavior:** the temporary produced by the inner call is freed once; `alloc_calls ==
free_calls`, `live_bytes 0`.

## Failing Reproduction

Measured by the coordinator on 2026-10-10 on main's code (`/tmp/main-dbg-target/release/mfb`),
`mfb build -q --debug`: `arena.0.alloc_calls 5`, `free_calls 4`, `live_bytes 144`.

```
IMPORT collections
IMPORT io


TYPE Circle
  r AS Float
END TYPE

TYPE Square
  side AS Float
END TYPE

UNION Shape
  Circle
  Square
END UNION

TYPE Holder
  items AS List OF Shape
END TYPE

FUNC describe(s AS Shape) AS String
  MATCH s
    CASE Circle(c)
      RETURN "circle " & toString(c.r)
    CASE Square(q)
      RETURN "square " & toString(q.side)
  END MATCH
END FUNC

FUNC showAll(label AS String, xs AS List OF Shape) AS Nothing
  MUT line AS String = label & ":"
  FOR EACH s IN xs
    line = line & " [" & describe(s) & "]"
  NEXT
  io::print(line)
END FUNC

FUNC showMap(label AS String, m AS Map OF String TO Shape) AS Nothing
  MUT line AS String = label & ":"
  FOR EACH k IN collections::keys(m)
    line = line & " " & k & "=[" & describe(collections::get(m, k)) & "]"
  NEXT
  io::print(line)
END FUNC

FUNC showNested(label AS String, rows AS List OF List OF Shape) AS Nothing
  MUT n AS Integer = 0
  FOR EACH row IN rows
    showAll(label & " row " & toString(n), row)
    n = n + 1
  NEXT
END FUNC

FUNC squares(first AS Float) AS List OF Square
  RETURN [Square[first], Square[first + 1.0]]
END FUNC
FUNC maybe(n AS Integer) AS List OF Square
  IF n < 0 THEN
    FAIL error(1, "neg")
  END IF
  RETURN squares(toFloat(n))
END FUNC

FUNC idOf OF T(x AS T) AS T
  RETURN x
END FUNC

FUNC main AS Integer
  LET d AS List OF Square = idOf(squares(20.0))
  io::print(toString(len(d)))
  RETURN 0
END FUNC

FUNC returnedShapes AS List OF Shape
  RETURN maybe(40)
END FUNC
```

Found by bs18-agent-j while fixing bug-989; it predates that fix.

## Root Cause

Not generics, and not collections only. Two ownership contracts met at one call and
disagreed about who owns the argument's block.

- **plan-86 K1, the borrow.** A function whose every `RETURN` is a bare parameter it
  never reassigns (`function_returns_param_borrow`,
  `src/codegen/engine/function/function_lowering.rs`) returns the argument's own
  pointer uncopied. Every caller reads the result as a borrow of its argument
  (`call_returns_param_borrow`, `src/codegen/engine/value/builder_values.rs`): it copies
  the result at an owning store and frees nothing, because "the argument's owner frees
  it".
- **plan-147, the hand-over.** `collect_handover_args`
  (`src/codegen/engine/analysis/handover.rs`) gives a fresh temporary, or a local at its
  last use, to the callee's owned variant (`idOf$List$OF$Square$own1`) when the callee
  consumes that parameter, and `consuming_use` counts `RETURN x` as consuming it.

So `LET d = idOf(squares(20.0))` handed the list to `idOf$own1`, which moved it straight
back out as its result. The caller had given the argument away, so it did not free it; it
still read the result as a borrow, so it copied it for `d` and did not free that either.
The block had no owner.

**Seen in the debugger** (`mfb build --debug -O0`, then
`printf 'bp _mfb_arena_alloc bt 2; list reg x0\nbp _mfb_arena_free bt 2; list reg x0\nrun\nquit\n' | mfb dbg build/a_generic.out`):
before the fix `squares + 804` allocates the 136-byte list, `main + 200` allocates the
136-byte copy, and at exit `_mfb_rt_drop_owned_collection` frees one block, the copy
(`…c0d0`); the list `squares` built is never passed to `_mfb_arena_free`. The MIR
(`mfb build --mir`) shows the call target `_mfb_fn_idOf_24List_24OF_24Square$own1` and
no free of the argument's pending-temp slot after it. After the fix the same trace has
a second `_mfb_rt_drop_owned_collection` free, of the list (`…c040`), right after the
copy.

## Resolution

`collect_handover_args` gained one refusal, H7: an argument is not handed over for a
parameter the callee returns as a borrow. `param_borrow_returns` (new, beside
`function_returns_param_borrow`, which is now `param_borrow_returns(..).is_some()`)
names those parameters, decided over the same `callback_referenced` set as the callee's
own lowering and the caller's classification, so the three cannot disagree. The call
then takes the ordinary lending path: the temporary stays on the statement's pending
list and is freed at the end of the statement, a local keeps its scope-drop, and the
result is copied at the owning store exactly as plan-86 K1 describes.

What is unchanged, each pinned by a case in the regression test or the unit test:

- a parameter such a function does **not** return is still handed over
  (`one_returned_one_consumed`: `keepFirst(ints(1), ints(5))`);
- a function that also builds its result (`RETURN xs` on one path, `RETURN
  append(xs, n)` on another) is not a borrow function, and its argument is still handed
  over (`growOrSame`, `consumed_accumulator`);
- a passthrough that is also used as a `FUNC` value returns a fresh block and its
  argument is still handed over (`callback_identity`, `sameRow`).

Rejected: teaching the caller that the result of a hand-over call to a borrow function
is a fresh block (which would save the copy). The caller classifies a call result from
the `NirValue` alone, at several points before and after the call is lowered, and the
lowering can decline a hand-over the analysis approved: `io::print(toString(len(
idOf(squares(20.0)))))` calls the base `idOf` although `idOf$own1` is emitted. A
classification that disagreed with the lowering in that direction would free the
caller's own argument twice. The refusal costs at most the copy plan-86 K1 already
specifies.

### Sibling matrix

`/tmp/bs19a/matrix.sh <mfb>` (25 programs, `mfb build -q --debug`), before → after, as
`alloc_calls / free_calls / live_bytes`; `double_free_skips` is 0 in every row, before
and after:

| Shape | Before | After |
| --- | --- | --- |
| `idOf(squares(20.0))`, generic | 5 / 4 / 144 | 5 / 5 / 0 |
| `idL(squares(20.0))`, not generic | 5 / 4 / 144 | 5 / 5 / 0 |
| `idOf(s)`, `s` at its last use | 5 / 4 / 144 | 5 / 5 / 0 |
| `idOf(s)`, `s` read again | 5 / 5 / 0 | 5 / 5 / 0 |
| `collections::transform(idOf(squares(20.0)), side)` | 5 / 4 / 144 | 5 / 5 / 0 |
| `idOf(Square[3.0])`, a record | 3 / 2 / 16 | 3 / 3 / 0 |
| `idOf("a" & toString(7))`, a `String` | 1 / 1 / 0 | 1 / 1 / 0 |
| `idOf(Map OF String TO Integer { … })` | 3 / 2 / 112 | 3 / 3 / 0 |
| `idOf(Set OF Integer { 1, 2 })` | 4 / 3 / 336 | 4 / 4 / 0 |
| `idOf([1, 2, 3])` | 3 / 2 / 64 | 3 / 3 / 0 |
| `idOf(Bag[ints(1), "bag"])`, a record of a collection | 5 / 4 / 96 | 5 / 5 / 0 |
| `lenOf(squares(20.0))`, a generic that does not return its argument | 4 / 4 / 0 | 4 / 4 / 0 |
| `idOf(idOf(squares(20.0)))`, nested | 5 / 4 / 144 | 5 / 5 / 0 |
| `len(idInts(idInts(ints(3))))` | 2 / 2 / 0 | 2 / 2 / 0 |
| `pick(ints(1), ints(5), TRUE)` / `FALSE` | 4 / 3 / 64 | 4 / 4 / 0 |
| `RETURN idOf(squares(n))` | 5 / 4 / 144 | 5 / 5 / 0 |
| `xs = idOf(xs)` | 3 / 2 / 64 | 3 / 3 / 0 |
| 50 passes of `LET d = idOf(ints(i))` | 101 / 51 / 3200 | 101 / 101 / 0 |
| `idOf(squares(20.0))` as a statement | 3 / 2 / 144 | 3 / 3 / 0 |
| `idOf(collections::append(xs, 9))`, `xs` dead | 4 / 3 / 96 | 5 / 5 / 0 |
| `idOf(maybe(3)) TRAP …`, success | 3 / 2 / 64 | 3 / 3 / 0 |
| `idOf(maybe(-1)) TRAP …`, failure | 4 / 4 / 0 | 4 / 4 / 0 |
| `transform(squares(1.0), idCb)` + `idCb(Square[9.0])` | 8 / 8 / 0 | 8 / 8 / 0 |
| `collections::reduce(ints(1), [0], keepAcc)` | 6 / 6 / 0 | 6 / 6 / 0 |

The one row whose allocation count moved is `idOf(collections::append(xs, 9))`: the
argument is no longer built by updating `xs` in place (that is part of the hand-over),
so `append` copies. It leaked before.

The bug document's own program (`/tmp/bs19-repro/g1019`): 5 / 5 / 0, `double_free_skips
0`, at the default level and at `-O0`, `-O2` and `-O3`.

## Checks run

- RED: `cargo test --release --test rt_param_borrow_handover_frees` at the RED commit
  fails 19 of the 26 programs (`generic_temp: … 5 allocated / 4 freed / 144 B live`);
  the unit test fails with H7 switched off ("a fresh temporary through a passthrough",
  left 1, right 0). Both pass with the fix.
- `cargo test --release --bin mfb -- codegen::engine::analysis:: codegen::engine::function::
  codegen::engine::builder:: every_body_mfb_member_declares`: 47 passed.
- `cargo test --release --test rt_owned_argument --test rt_scope_drop_leaks --test
  codegen_string_return_freshness --test codegen_trap_result_wrapper --test
  rt_returned_transform_element_type --test codegen_helper_scratch_release --test
  rt_reduce_collection_accumulator_frees --test rt_inplace_self_update --test
  rt_self_update_cost --test inplace_self_update_census` and the `tests/guards/*`
  targets: all passed.
- `scripts/artifact-gate.sh target/release/mfb all`: 1664 tests, 1843 builds, 2320
  goldens checked, 0 diffs.
- `scripts/spec-census.sh --render` and `--citations` after the spec edit.

## Golden impact

None. `scripts/artifact-gate.sh target/release/mfb all` reports 0 diffs over 2320
goldens: no committed fixture hands an argument to a function that returns it as a
borrow. No golden was regenerated. Not run: the perf goldens and the full acceptance
suite (batch rule: targeted checks only).
