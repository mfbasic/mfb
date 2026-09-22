# bug-677: `collections::transform` leaks a callback's record, union or collection result

Last updated: 2026-09-21
Effort: small (<1h)
Severity: MEDIUM
Class: Memory (a leak of one block per element per call)

Status: Fixed
Regression Test: `tests/runtime/rt_transform_block_result_frees.rs`

**STATUS: FIXED.** Found by plan-145-D's differential probe, whose `--debug` report
showed 2304 bytes live after the `transform` cases over a `List OF P`. The same
program leaked the same bytes with the pre-plan-145-D compiler: the leak comes from
`transform` itself, not from the field arms.

## Failing Reproduction

```basic
IMPORT collections
IMPORT io

TYPE P
  k AS Integer
  s AS String
END TYPE

FUNC mkP(n AS Integer) AS P
  RETURN P[k := n, s := "made"]
END FUNC

FUNC main() AS Integer
  LET ys AS List OF P = collections::transform([1, 2, 3], mkP)
  io::print(toString(len(ys)))
  RETURN 0
END FUNC
```

- Observed (built `--debug` with main's compiler at `4e0c50a8b`): prints 3, and the
  report shows `arena.0.live_bytes 96`: three 32-byte `P` blocks never freed.
- Expected: `live_bytes 0`.

The leak is the same for a fixed-width record result (48 B for three), a
`List OF Integer` result (192 B), a data union result (96 B), a record-to-record
callback, and the in-place self-update `xs = collections::transform(xs, upP)` over a
`List OF P` (its parked results). The in-place `m = collections::mapValues(m, f)`
over a `Map OF String TO P` leaked its parked results too (128 B for two entries, two
calls), and both in-place arms leaked them on a failing callback's unwind. A
`String` result has not leaked since bug-569. A
record *argument* is an alias into the source list and was never leaked
(`transform(ps, keyP)` → 0 B).

## Root Cause

The `FunctionRef` ABI owns the callback's result (bug-569). `lower_transform`
(`src/codegen/builtins/collections/func_transform.rs`) appends it to the output list,
and the append byte-copies the payload, so the returned block has no reader after
that. bug-569 freed it only when the result type was `String`, because
`free_collection_loop_item` frees only a `String`. Every other flat block result — a
record, a data union, a collection, a flat `Result` — was never freed. The in-place
arm `try_inplace_transform_assign` (`builder_inplace_rewrite.rs`) had the same
`String`-only free in its write pass and its failure unwind.

## Fix

`free_callback_result` (`src/codegen/collection/collection_loop.rs`) frees a `String`
as before, and any other `is_freeable_flat_value` result through the ordinary
owned-value drop (`emit_owned_value_drop`, which sizes the block from its type).
Scalars are inline, and non-flat values (graphs, resources) are not byte-copied by
the append, so neither is freed. It is used:

- by `lower_transform` after the append. When the element type equals the result
  type, the free keeps the existing identity guard: a callback that returns its
  borrowed argument hands back an alias into the source list, which must not be
  freed.
- by `try_inplace_transform_assign` and `try_inplace_map_values_assign`
  (`builder_inplace_setmap.rs`) in their write pass and their failure unwind.

A FUNC and a LAMBDA identity callback over a `List OF P`, copying and in place,
print the right values and leave 0 B live (the C-state compiler: 9600 B for 50
rounds).

## Verification

- `MFB_TEST_EXE=<the plan-145-C compiler, 062c50c4c> cargo test --test rt_transform_block_result_frees`
  → FAILED: all 9 cases leak (96, 48, 192, 96, 96, 192, 288, 128 and 96 B live).
- `MFB_TEST_EXE=target/release/mfb cargo test --test rt_transform_block_result_frees`
  → ok.
- plan-145-D's differential probe: 164 of 164 cases `ok`, `live_bytes 0` (2304 B
  before).
- The one golden that moved: `byte-identity/http` — `__http_invokeHandler` applies
  the route handler with a singleton `collections::transform`, whose `Response`
  result is now freed (`ncode_fn_diff` of the linux-x86_64 dump: 1 of 157 functions
  changed, `_mfb_ifn_http_5FinvokeHandler`, 3703 → 3825 instructions). The http
  runtime tests pass on the fix (`rt_http_handle_request_serves`,
  `rt_http_server_dos`, `rt_http_async_stream`, `rt_http_chunked_frame_completion`
  → ok). Regenerated with `bash scripts/regen-native-goldens.sh target/release/mfb
  tests/byte-identity/http` → "5 golden(s) rewritten, 0 failure(s)". The rest of
  `scripts/artifact-gate.sh target/release/mfb all` → 0 diffs.
