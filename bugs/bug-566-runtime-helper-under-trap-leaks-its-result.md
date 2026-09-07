# bug-566: a runtime-helper call under an inline `TRAP` leaks the block the helper returned

Last updated: 2026-09-06
Effort: medium (needs a per-helper ownership audit)
Severity: MEDIUM–HIGH (unbounded leak on `fs::`/`net::`/`http::` reads in a loop)
Class: Memory / correctness

Status: Open
Regression Test: — (a `tests/rt_scope_drop_leaks.rs` RSS case, to be added)

Found while fixing bug-561. It is the third of the three lowering paths that
build a `Result` for an inline `TRAP`; bug-561 fixed the other two and left this
one deliberately, fail-closed, because it needs an audit this change did not do.

## The finding

```
LET s AS String = fs::readText("probe.txt") TRAP(e)
  RECOVER "x"
END TRAP
```

in a loop over an 12-byte file:

| N | base `19880284452` | after bug-560 + bug-561 |
| --- | --- | --- |
| 20 000 | 4.7 MB | 4.7 MB |
| 40 000 | 8.4 MB | 8.4 MB |

Unchanged by bug-561, ~190 B per call.

## Root cause

`lower_runtime_helper_call(.., raw = true)` hands the helper's raw result
registers to `materialize_current_result`, which

1. `copy_value_to_current_arena`s the success value into an intermediate block,
2. `emit_build_result_inline`s that intermediate INTO the `Result` block, and
3. frees the intermediate (bug-379).

The HELPER's own original block — step 1's source — has no owner and is never
freed. Outside a `TRAP` the same call is flat, because the binding takes the
helper's block directly and its scope drop frees it.

## Why bug-561 did not fix it

bug-561 reuses `pending_temp_is_freeable` — the audited predicate the plain-call
path already asks — at each raw `Result` site. For a `RuntimeCall` that predicate
answers **false** for a `String` (no `mark_fresh_string` provenance can cross the
helper boundary, and `call_returns_fresh_string` only knows about `.mfb`/user
functions), so the site keeps leaking rather than freeing a block it cannot prove
it owns. That is the intended fail-closed behaviour, not an oversight — but it
leaves this leak live.

## What a fix must produce

A runtime-helper call under `TRAP` runs at constant RSS for every payload type.

The fix needs the missing half of the provenance: an audited statement, per
runtime helper, of whether its returned block is a fresh allocation in the
CALLER's arena. `thread.waitFor` is the counter-example that makes this an audit
and not a blanket rule — its result lives in the worker's arena
(`materialize_current_result`'s `worker_error_source` flag) and freeing it from
the caller would be a cross-arena wild free. `value_is_runtime_managed` already
excludes every `thread.*` target, which is the natural seat for the answer.
