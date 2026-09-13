# bug-576: an UNBOUND runtime-helper `String` result is never freed

Last updated: 2026-09-12
Effort: small–medium (the free is a statement-scope temp; the audit is which call positions already own one)
Severity: MEDIUM (unbounded leak in any loop that uses a helper's result without binding it)
Class: Memory / correctness

Status: **FIXED** (2026-09-12, `fbddba488`, goldens `a8305b07d`)
Regression Test:
`tests/runtime/rt_scope_drop_leaks.rs` — nine RSS cases (`b576_unbound_hostname`,
`b576_unbound_arch`, `b576_unbound_tempdir`, `b576_unbound_concat`,
`b576_unbound_argument`, `b576_returned`, `b576_into_collection`, and the two
positive pins `b576_bound_arch` / `b576_bound_cwd`) plus
`every_unbound_helper_result_position_still_produces_the_right_value`;
`src/codegen/registry/mod.rs::every_string_returning_runtime_helper_is_marked_fresh`
— the catalog-wide enumeration.

Found while fixing bug-574, and measured to be a **different defect**: it is
byte-identical before and after that change, and it is present for helpers that
allocate nothing but their result.

## The finding

```
n = n + len(os::hostName())        ' the result is never bound
```
in a loop leaks per call. Binding it does not:

| shape | 20 000 → 40 000, per call |
| --- | --- |
| `n = n + len(os::hostName())` | **128 B** |
| `n = n + len(os::arch())` | **63 B** |
| `n = n + len(fs::tempDirectory())` | **265 B** |
| `LET s AS String = os::arch()` then `len(s)` | 0 B |
| `LET s AS String = fs::currentDirectory()` then `len(s)` | 0 B |

Measured on `4ef3bce0c` and after bug-574 — identical in both, to the byte.

Re-measured at 200 000 → 400 000 on the pre-fix release binary while fixing it:
129 B/call, 64 B/call, 260 B/call, and 32 KB *total* movement for the bound rows.
The attribution in the original report held exactly.

That contrast is the whole report. `owns_freeable_value` registers a scope-drop
`arena_free` for the block a `Bind` of a runtime call yields, which is why the
bound rows are flat. An unbound one — the call's result consumed by `len(...)`,
by `&`, or by another call's argument list and then discarded — yields a
`ValueResult` no binding claims and no statement-scope temp list holds.

## Two more leaking positions the report did not list

Measured RED with the same root cause, and fixed by the same change:

* **`RETURN os::arch()`** — 12 MB / 200k. `lower_returned_value` found no pending
  temp to claim (there was none to register), fell through to the
  `current_returns_fresh_string` arm, and `copy_flat_block`ed the block to deliver
  the promise — leaving the ORIGINAL with no owner. With the mark the result is a
  claimable temp, so the block is moved to the caller and the redundant copy
  disappears with the leak (the same arm bug-536 shape A added for a constructor).
* **`collections::append(names, os::arch())`** — 24 MB / 200k. `append` copies the
  element's bytes into the container's data region, so the helper's block is dead
  the moment the call returns.

## The fix

`emit_runtime_helper_call`'s non-`raw` tail now calls
`mark_runtime_helper_result_fresh(target, result_type, <result register>)`
(`src/codegen/engine/value/builder_values.rs`), which marks the result with
bug-536 shape B's freshness provenance when — and only when — both hold:

* `runtime_result_needs_fresh_string_mark(result_type)` — a **wildcard-free**
  `match` over `ParameterType` that answers `true` for `String` alone. Every other
  freeable-flat result (a `List OF Byte`, a `net.Address`, a `Result OF T`) is
  already freed unmarked by `register_pending_temp`, and everything else is not
  freeable-flat at all. A new `ParameterType` variant is a build error here.
* `runtime_result_is_caller_owned(target, result_type)` — bug-566's predicate,
  renamed from `raw_runtime_result_is_caller_owned` because it now serves both the
  raw and the non-raw path. It is `!runtime_call_result_is_foreign_arena(target)
  && is_freeable_flat_value(type)`: the SAME gate the `Bind` path
  (`owns_freeable_value`) already applies to the same call, so no new licence is
  created — `thread.*` keeps its exemption (`x19` is per-thread; a
  `thread::waitFor` result is the WORKER's block).

The change only ADDS a free. No lifetime moves: a bound, reassigned, returned or
moved-into-a-collection result claims the temp through the existing
`claim_pending_temp` path, exactly as it does for any other marked native producer.

## Why it is not a one-liner

The registration has to be exactly as narrow as the `Bind` gate already is:
`!runtime_call_result_is_foreign_arena(target) && is_freeable_flat_value(type)`
(bug-566's audit). `x19` is per-thread, so a `thread::waitFor` result registered
as a statement-scope temp is a cross-arena free, and the result may also be
MOVED into a collection or returned rather than dropped — the same "who else
claims this block" question bug-567 answered for `RETURN`'s interior temps.

## What a fix must produce

Each of the three rows above flat at N and 2N, `LET`-bound forms still flat (no
double free), and the value probes in `rt_scope_drop_leaks.rs` unchanged —
including `thread::waitFor`, whose result is another arena's.
