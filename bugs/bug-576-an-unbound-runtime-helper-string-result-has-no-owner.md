# bug-576: an UNBOUND runtime-helper `String` result is never freed

Last updated: 2026-09-07
Effort: small–medium (the free is a statement-scope temp; the audit is which call positions already own one)
Severity: MEDIUM (unbounded leak in any loop that uses a helper's result without binding it)
Class: Memory / correctness

Status: Open
Regression Test: — (an RSS case in `tests/runtime/rt_scope_drop_leaks.rs`, to be added)

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

That contrast is the whole report. `owns_freeable_value` registers a scope-drop
`arena_free` for the block a `Bind` of a runtime call yields, which is why the
bound rows are flat. An unbound one — the call's result consumed by `len(...)`,
by `&`, or by another call's argument list and then discarded — yields a
`ValueResult` no binding claims and no statement-scope temp list holds.

## Where to look

`emit_runtime_helper_call` in
`src/codegen/engine/builder/builder_emit_helpers.rs` returns the result register
as a `ValueResult` with `origin: None`. The `Bind` path registers the drop; the
expression paths do not. bug-536 shape B fixed the analogous hole for INLINE
builtins (`register_fresh_string_temp`), and `unbound_native_string_producers_run_at_constant_rss`
in `tests/runtime/rt_scope_drop_leaks.rs` is that pin — it covers `strings::`,
`toString` and `&`, none of which is a runtime helper, which is why this half
stayed open.

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
