# bug-570: `RETURN <nested concat>` drops an interior temp unfreed — 64 B per call

Last updated: 2026-09-07
Effort: small-to-medium
Severity: **HIGH** (unbounded leak; a two-part concat in a `RETURN` is everywhere)
Class: Memory / correctness

Status: Open
Regression Test: — (an RSS case in `tests/runtime/rt_scope_drop_leaks.rs`)

## Failing reproduction

```
IMPORT io

FUNC deco(s AS String) AS String
  RETURN "<" & s & ">"
END FUNC

SUB main()
  LET arg AS String = "x"
  MUT i AS Integer = 0
  MUT acc AS Integer = 0
  WHILE i < 400000
    acc = acc + len(deco(arg))
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
```

Peak RSS (`/usr/bin/time -l`), macos-aarch64, release: **25.6 MB at 400 000,
50.2 MB at 800 000.** 64 B per call.

## It is the SECOND concat, not the concat

Same program, one operator fewer in the `RETURN`:

| `deco`'s body | 400 000 | 800 000 |
|---|---|---|
| `RETURN "<" & s & ">"` | 25.6 MB | 50.2 MB |
| `RETURN s & ">"` | 1.0 MB | 1.0 MB |
| `RETURN toString(s)` | 1.0 MB | 1.0 MB |
| `MUT o = "<"` / `o = o & s` / `RETURN o` | 1.0 MB | 1.0 MB |
| `RETURN "lit"` | 1.0 MB | 1.0 MB |

A single concat is flat. A nested one leaks exactly one block. Splitting the same
expression across two statements is flat. So the leak is the block the INNER
concat allocates.

## Root cause

`"<" & s & ">"` lowers as two concats, each registering a statement-scope pending
temp: the inner (`"<" & s`) then the outer (the whole thing).

At the `RETURN`, `lower_returned_value` claims the tail temp — the outer one —
and moves it to the caller (`claim_pending_temp`, `builder_exits.rs`). Then
`lower_ops` reaches its per-statement epilogue:

```
    // A control-transfer statement branches away, so any interior-temp free
    // would be unreachable and a returned/moved temp belongs to the target;
    // just forget them. Every other statement frees its interior temps here.
    if Self::op_transfers_control(op) {
        self.clear_pending_temps_to(temp_watermark);   // <- truncate, no free
    } else {
        self.drop_pending_temps_to(temp_watermark)?;
    }
```
(`src/codegen/engine/control/builder_control.rs`)

`clear_pending_temps_to` truncates the pending list **without emitting a free**.
Its comment justifies that two ways, and only the first is true:

* "a returned/moved temp belongs to the target" — true, but only of the ONE temp
  the return claimed, and claiming already popped it;
* "any interior-temp free would be unreachable" — not true. The interior temps are
  still live at the point the return value is computed; their frees are
  unreachable only because they would be emitted *after* the branch. Emitted
  *before* it, they are on the path.

So every pending temp a `RETURN`/`EXIT`/`CONTINUE`/`Fail` statement's expression
allocated other than the returned one is leaked. A single concat has exactly one
temp, which the return claims — hence flat. A nested concat has two.

## The fix, and its hazard

At a control-transfer statement, free the *unclaimed* interior temps before
emitting the branch, instead of truncating them. The returned temp is already off
the list by then (`claim_pending_temp` pops it), so "everything still on the list
above the watermark" is exactly the set to free.

The hazard is the usual one and it points the other way from the leak: this ADDS
frees, so a temp that is still reachable from the returned value — an inner block
whose bytes the outer concat copied *by pointer* rather than by value — would
become a use-after-free. Concat copies bytes, so it is safe there; the change
needs the same enumeration over every node that registers a pending temp.

Note also `EXIT SUB` / `CONTINUE` / `FAIL`, which take the same branch and have
the same leak with no returned temp at all.

## Relationship to bug-562

bug-562 fixed a *different* 64 B/call leak with the same surface symptom: a
`String`-returning function that is used as a callback ANYWHERE in the module lost
its callers' statement-scope free, because `function_returns_fresh_string`
excluded callback-referenced names on both the callee and the caller side. That
one is now flat (25.6/50.2 MB -> 1.0/1.0 MB;
`a_direct_call_to_a_callback_referenced_string_callee_runs_at_constant_rss`).

This bug is independent of callback status — it reproduces on a function no
callback ever references — and bug-562's change does not move its numbers at all
(25.6/50.2 before and after).
