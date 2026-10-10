# bug-999: a `List OF RES` (or record of resources) returned by a call nested in an expression is never closed — `len(mkList())` leaks every resource it holds

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (resource leak)

Status: Fixed (2026-10-10)
Regression Test: `tests/runtime/rt_resource_temporary_close.rs` (`an_inline_aggregate_of_resources_does_not_leak_descriptors`, `an_inline_aggregate_closes_only_what_nobody_took`: descriptors under `ulimit -n 64`) and `tests/runtime/rt_resource_drop_reclaims.rs` (the `b999_*` cases: `--debug` counters)

bug-842 made a resource produced inline in an expression (a call argument, a member read, a
list or record literal argument) a statement-scoped temporary that is closed at the end of its
statement. It deliberately left out aggregates: a call nested in an expression that RETURNS a
`List OF RES T` (or a record or union holding resources) is not registered. Its resources are
never closed. bug-842's resolution gives the reason: a builtin aggregate can be a view of
resources someone else owns (`canvas::groupItems`) with no owner word to say so, and
registering every aggregate doubled the canvas renderer helpers (`_mfb_ifn_canvas_5FdrawGroup`
477 → 1635 instructions).

**Correct behavior:** language spec 15: "A resource is closed exactly once." An aggregate of
resources that a user call returns and that nothing claims should have its resources closed by
the end of the statement, as the bound form `LET ys = mkList()` does at scope exit. Views
returned by builtins that do not transfer ownership (`canvas::groupItems`) must stay untouched.
The fix probably needs an ownership distinction between "fresh owned aggregate" and "view",
which bug-842 found missing.

## Failing Reproduction

Measured by the coordinator on 2026-10-06, macOS aarch64, with main 468ff0f94
(`/tmp/main-dbg-target/release/mfb`) and with bs17-agent-f (bug-842 fixed). The result is the
same on both.

```
IMPORT fs
IMPORT io
IMPORT collections

FUNC mkList() AS List OF RES fs::File
  MUT xs AS List OF RES fs::File = []
  RES f AS fs::File = fs::createTempFile()
  xs = collections::append(xs, f)
  RETURN xs
END FUNC

FUNC main AS Integer
  MUT n AS Integer = 0
  FOR i = 1 TO 2000
    n = n + len(mkList())
  NEXT
  io::print("n=" & toString(n))
  RETURN 0
END FUNC

mfb build -q agg && (ulimit -n 64; ./agg/build/agg.out)
```

- Observed: `Error: 7-702-0002` / `Write or flush operation failed.`, exit 255. The descriptors
  run out.
- Control: the same loop with the list bound first
  (`LET ys AS List OF RES fs::File = mkList()` then `n = n + len(ys)`) prints `n=2000`, exit 0
  (bs17-agent-f).
- Expected: `n=2000`, exit 0.

## Related

- bug-842 (resource temporaries; this is the aggregate case it scoped out).

## The distinction between a handed-over aggregate and a view

bug-842 left aggregates out because it had no way to tell a fresh aggregate from a
view. The language already draws that line, and no new rule was needed:

- `mfb spec language resource-management` §15: a function returning a value that
  reaches resources "releases each referenced resource that one of its scopes owns
  … and the caller's binding scope takes each released resource, and no other … a
  call whose result is discarded closes the resources nobody took". The owner word
  (`mfb spec memory heap-values`, "The owner word") is that fact at run time: `0`
  only for a record the callee produced or released and nothing has taken.
- That statement is about a function that leaves through `RETURN`. A natively
  lowered builtin has no `RETURN` and makes no such statement: `canvas::groupItems`
  hands back the group store's own items, which are held and unowned at once.

So the producer rule of bug-842 extends to an aggregate exactly when the call runs
a function **body** (`self.functions` has it), under the same `adopts_from_call`
exclusion the bound form uses for a `collections` member that only re-packages its
arguments. A natively lowered call's aggregate stays a view. `canvas::groupItems`
is internal-only, so user code cannot wrap it in a body.

One more view is told apart statically: a function whose every `RETURN` hands back
a parameter it never reassigns, or a module-level binding
(`builder_values.rs:function_returns_only_views`). It transfers nothing — a
parameter's resources are the caller's and a module binding's are the module
instance's — so a call to it is not a producer. This keeps the canvas renderer's
`__canvas_hashScene(__canvas_frameItems(), [])` (which returns the frame's item
list, a module binding) exactly as it was.

## Resolution

Two tiers. Both close every resource the result reaches that is still unowned at
the end of the statement.

**1. Closed, tombstone kept (the bug-842 rule, extended).**
`builder_values.rs:register_resource_temp` registers a `Call`/`CallResult` to a
function with a body whose result reaches a resource leaf
(`resource_leaf_types`), as it already registered a call whose result is a
resource. At the end of the statement `emit_resource_temp_close` walks the result
and closes each unowned, not yet closed record, and reclaims its buffers and
`STATE`. The record stays as a closed tombstone and the aggregate's own block is
not freed: a reference stored without a float
(`keep = collections::append(keep, collections::get(mkList(), 0))`) must go on
reading a closed handle (§15, "other records remain as tombstones").

**2. Freed whole, when the result is the statement's alone.** `len(mkList())` is
`LET t = mkList()` + `len(t)` shrunk to one statement, and the bound form frees
everything. `record_ownership.rs` (`CollectionUses::exclusive_temps`) proves the
same exclusivity for a nested call from three facts:

- it is written straight into an argument of a call that does not keep that
  argument (a natively lowered builtin with no consuming parameter there, or a
  body read and found not to keep it) and that reaches no unread code (no callback
  that keeps, no call through a function value);
- that receiving call can only answer with a plain scalar
  (`call_returns_plain_scalar`), so nothing of the argument comes back out of it;
- the nested call runs a body all of whose parameters are plain scalars
  (`call_takes_only_plain_scalars`), so an unowned record in its result came from
  its own frame — never an argument's holder, never a closed tombstone a
  longer-lived list still names.

For such a call `adopt_exclusive_temp` does what a binding's adoption does, when
the call returns: every unowned record the result reaches is claimed for the
current scope and listed (one owned-list per leaf kind, the adopting push of
bug-1000, so a record reached twice is listed once and a record that already has
an owner is left to it). At the end of the statement `emit_exclusive_temp_drop`
drains the lists with the owning frees (close, buffers, `STATE`, record, node) and
frees the result's own block when the callee built it
(`user_call_returns_fresh_block`; `len(keep(xs))` hands back the caller's block).

Not in tier 2, so tier 1: a result that reaches a resource-union box (bug-1012: the
box is freed by the callee's binding, so a drain that frees boxes frees it twice)
or a function value; a call in any other position (a member read `mkHolder().tag`,
a `FOR EACH` iterable, an argument of a call that answers with a collection).

### Runtime proof

- Descriptors: 2000 × `n = n + len(mkList())` under `ulimit -n 64` prints `n=2000`
  (base: `Error: 7-702-0002`, exit 255).
- `mfb build -q --debug`, arena 0, the report's program with `N` from the
  environment (`/tmp/k/probe.sh <mfb> g1 10 20`), against the bound control `g1c`:

| program | base N=20 | fixed N=10 | fixed N=20 |
|---|---|---|---|
| `n = n + len(mkList())` | 143 / 103 / 7040 | 83 / 83 / 0 | 163 / 163 / 0 |
| control: `LET ys = mkList()` then `len(ys)` | 163 / 163 / 0 | 83 / 83 / 0 | 163 / 163 / 0 |

  The fixed nested form allocates and frees exactly what the control does (it also
  allocates the adoption's list node, as the control does). `double_free_skips 0`.
- What is not the statement's stays: `rt_resource_drop_reclaims.rs`
  (`an_inline_aggregate_that_is_not_the_statements_alone_is_left_alone`) uses the
  caller's list after `len(keep(xs))` and the caller's handle after
  `len(wrap(a))` (both `open`), and reads an element copied out of `mkList()`
  without a float after its statement (`closed`, the defined `ErrResourceClosed`).
  `rt_resource_temporary_close.rs` reads through the elements while the statement
  runs (a callee, a `FOR EACH` body) and through an aggregate a binding or a
  `RETURN` took (`open` to the end of that scope).

Residual, by design: a tier-1 temporary leaves one 112 B tombstone per resource and
its aggregate block, as every resource temporary has since bug-842.

## Golden impact

Measured with `scripts/artifact-gate.sh <mfb> all` on this branch and on its base
commit `821806278` (built in a detached worktree): the base already reports 33
diffs it inherited (goldens not regenerated in this batch), and this branch
reports 39. The per-function comparison below is `mfb build -ncode` from the two
compilers.

**No golden moves for this bug.** No fixture with a native golden nests a call to
a function with a body whose result reaches a resource. The one candidate in the
builtin sources, the canvas renderer's `__canvas_hashScene(__canvas_frameItems(),
[])`, returns a module-level binding and is a view (`function_returns_only_views`):
`_mfb_ifn_canvas_5FsceneOffsets` differs from the base commit's only by bug-1000's
22 instructions.
