# bug-566: a runtime-helper call under an inline `TRAP` leaks the block the helper returned

Last updated: 2026-09-07
Effort: medium (needs a per-helper ownership audit)
Severity: MEDIUM–HIGH (unbounded leak on `fs::`/`net::`/`http::` reads in a loop)
Class: Memory / correctness

Status: **FIXED** (2026-09-07, `be8ed99ff`)
Regression Test: `tests/runtime/rt_scope_drop_leaks.rs` —
`a_trapped_runtime_helper_string_result_grows_no_faster_than_the_plain_call`,
`a_trapped_runtime_helper_collection_result_grows_no_faster_than_the_plain_call`,
the POSITIVE pin `a_trapped_runtime_helper_scalar_result_was_never_the_leak`,
the value pin `every_trapped_runtime_helper_result_still_produces_the_right_value`;
the five owner counts in `tests/codegen/codegen_raw_helper_result_drop.rs`;
and the audit `codegen::registry::raw_result_block_ownership` —
`every_block_returning_runtime_helper_is_classified`,
`no_thread_family_result_is_ever_freed_by_the_calling_thread`

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


## The fix (2026-09-07)

### The audit, and where its answer lives

`materialize_current_result` gained a `RawSuccessBlock` parameter — a two-valued
enum answering one question, *who owns the block the raw success value points at*
— and **all seven call sites state it**. There is no default:

| site | answer | why |
| --- | --- | --- |
| `emit_runtime_helper_call(raw)` | audited per call | the leak; see below |
| `t.result` / `thread.waitFor` | `OwnedElsewhere` | the WORKER's arena |
| `emit_thread_send_runtime_helper_call` | `OwnedElsewhere` | `thread.*`, and every one returns `Nothing` |
| `lower_checked_value` | `OwnedElsewhere` | `lower_value` registered its own pending temp |
| `lower_inline_conversion_raw` | `OwnedElsewhere` | every conversion yields a scalar |
| `lower_inline_builtin_raw` | `OwnedElsewhere` | bug-561's `register_raw_member_result_temp` owns it |
| `lower_inline_infallible_raw` | `OwnedElsewhere` | likewise |

The runtime-helper site's predicate is
`raw_runtime_result_is_caller_owned(target, result_type)` =
`!runtime_call_result_is_foreign_arena(target) && is_freeable_flat_value(result_type)`.

**That is not a new licence.** It is the gate a `Bind` of the same call already
applies: `owns_freeable_value` in `lower_ops_inner` registers a scope-drop
`arena_free` for exactly `!runtime_managed && is_freeable_flat_value(T)`, which is
why `LET s AS String = fs::readText(p)` has always freed the helper's block. A
helper whose result were not a fresh caller-arena allocation would already be a
wild free at every `LET`. bug-566 asks the same question at the site the `TRAP`
desugar bypassed.

The exclusion is `runtime_call_result_is_foreign_arena`, which
`value_is_runtime_managed` now calls so the two cannot drift — the seat this
document predicted.

### The enumeration is total, and asserted

`raw_result_block_ownership` partitions the WHOLE runtime-call catalog
(`registry::runtime_specs()`, 200+ entries) into three classes and asserts each by
set equality:

* **`FOREIGN_ARENA_RESULTS`** — the seven calls whose declared return type is a
  type VARIABLE (`thread.waitFor` is `Out`, `thread.receive` is `Msg`,
  `thread.accept` is `Res`, plus `thread.start`'s handle). `assert_eq!` says these
  are exactly the generic-result calls in the catalog, which is the safety-critical
  half: at the call site the substituted type is an ordinary `String` and nothing
  in the TYPE says the block belongs to another arena — only the call's identity
  does. A NON-thread helper appearing in that set reds, because it would otherwise
  be freed on the strength of a concrete type the catalog never declared.
* **`CALLER_ARENA_BLOCK_RESULTS`** — the 63 calls with a concrete block-carrying
  return (`fs.readText`, `os.environ`, `net.lookup`, `crypto.sign`, …). A new
  block-returning helper reds and must be added deliberately.
* the complement — scalars, `Nothing`, and resource HANDLES (`is_freeable_flat_value`
  is false for a handle; its lifetime is the §15 close obligation) — asserted
  disjoint from both, and the three lengths asserted to cover the catalog.

`no_thread_family_result_is_ever_freed_by_the_calling_thread` additionally
declines the whole `thread` family, not just the seven, so a member that returns
`Nothing` today and a `String` tomorrow does not become freeable the moment its
signature changes.

### The runtime guard

The free is not emitted bare. It compares the producer's pointer against the
intermediate's (`raw_helper_result_kept`) and skips when they are equal, so if any
producer or copy path ever made `copy_value_to_current_arena` an identity, the
bug-379 free above has already released the block and this one declines. Soundness
is local to four instructions rather than resting on the copy's behaviour.

### Measured — and the report's "flat outside a TRAP" did not survive

macOS arm64, peak RSS via `/usr/bin/time -l`, 12-byte file, N = 20 000 / 40 000.
Reproduced byte-identically on `ac421788a` and on `38b855905` (after
bug-561/565/568 landed), so it is neither caused nor fixed by any of them:

| program | base `ac421788a` | after |
| --- | --- | --- |
| `fs::readText(p)` under `TRAP`, bound `String` | 6.2 → 11.4 MB (**260 B/call**) | 3.6 → 6.3 MB (**132 B/call**) |
| the same call bound with **no** `TRAP` | 3.6 → 6.2 MB (129 B/call) | 3.7 → 6.2 MB (unchanged) |
| `fs::exists(p)` under `TRAP`, `Boolean` payload | 3.6 → 6.2 MB (129 B/call) | 3.6 → 6.2 MB (unchanged) |

bug-566's own leak is the DIFFERENCE — 131 B per call — and it is gone: the
trapped form now grows at exactly the rate of the untrapped one.

**This document said "Outside a `TRAP` the same call is flat". It is not.** The
untrapped `LET s AS String = fs::readText(p)` grows 129 B per call on the base
compiler and still does, and so does `fs::exists(p)` with no `TRAP` and no block
result at all. That residual is a **separate defect**: every runtime-helper call
leaks its marshalled `String` ARGUMENT, and the leak scales with the argument's
length (a 10-char path costs ~65 B per call, a 415-char path ~1 819 B; a
zero-argument helper such as `os::arch()` is flat). Filed as **bug-574**.

It is also why the RSS pins here are comparative (`assert_no_extra_growth`, "the
trapped form grows no faster than the plain one") rather than `assert_flat`:
flatness is unavailable while bug-574 is open, and a flatness assertion here would
have been red for a reason that has nothing to do with this fix. When bug-574
lands, all four can be tightened to `assert_flat`.

### Golden attribution

`artifact-gate.sh all`: **5 diffs, all five targets of one fixture**
(`byte-identity/http`). Per function against the base compiler:

**6 functions changed — `http::lingerNet`, `http::lingerTls`, `http::readNet`,
`http::readTls`, `http::readRequestNet`, `http::readRequestTls`. Every one gained
exactly one guard AND exactly one `_mfb_arena_free`. None changed without gaining
an owner. `dataObjects` byte-identical. `_mfb_arena_alloc` unchanged (delta 0).**

All six are the same shape: `tcp::read` / `tls::read` — a runtime helper returning
`List OF Byte` — under an inline `TRAP`. That the whole tree contains exactly six
such sites, and that they are the six `http` needed, is itself the evidence that
the predicate is narrow.

### Gates

Rebased onto `38b855905` (after bug-561/565/568 landed) and re-run there:
`cargo test --release --no-fail-fast` 158 binaries, **5000 passed, 0 failed**;
`artifact-gate.sh all` 1412 tests, 1578 builds, 1973 goldens, **0 diffs** after
`regen-ncodesum.sh` (5 goldens changed against main); `test-accept.sh`
**1434 tests passed**; `cargo fmt --all --check` and `cargo check --all-targets`
clean.
