# bug-999: a `List OF RES` (or record of resources) returned by a call nested in an expression is never closed — `len(mkList())` leaks every resource it holds

Last updated: 2026-10-06
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (resource leak)

Status: Open
Regression Test: (proposed) a `rt_resource_temporary_close`-style test that runs 2000 × `len(mkList())` under `ulimit -n 64`

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

## Attempt in bug-staging-18 (bs18-agent-k, 2026-10-10): reverted, still open

A fix was committed (`ee8fa14cb`) and reverted (`8982edd27`) in the same batch. What
it found:

- **Who produced it is settled.** A call that runs a function BODY leaves through
  `RETURN`, which releases exactly the records its own scopes owned (spec §15), so an
  unowned record in its result is one it handed over. A natively lowered builtin
  (`canvas::groupItems`) makes no such statement, and neither does a body whose every
  `RETURN` hands back a parameter or a module-level binding.
- **Who keeps it is NOT settled, and that is what broke.** The fix closed, at the end
  of the statement, everything an aggregate from a function body reached unowned.
  That is wrong whenever a value that outlives the statement keeps the aggregate
  without owning what it reaches: `LET back = Layer[items := mkItems()]`,
  `LET items = mkLayer().items`, `FOR EACH h IN mkList()`. Only a call-rooted bind
  adopts, and only a single resource in an element position is hoisted to a name.
  `tests/canvas/rt_canvas_geo_native.rs`
  (`the_native_frame_pass_matches_the_mfbasic_walk_on_a_mixed_moving_scene`) died of
  it with SIGSEGV on `LET back AS canvas::DrawLayer = canvas::DrawLayer[items :=
  shapes(20)]`: a constructor frees its argument's block once it has copied it, so
  the statement-end walk read a freed list (`mfb dbg`: a bad memory access in
  `main+0x5dd4`, on the `ldr x9, [x8]` that loads an element's tag). Passing with
  the base compiler, failing with the fix, three runs each.
- **One class is safe and was measured flat**, but was not verified after being
  separated from the rest, so it is not in the tree: a call to a function taking only
  plain scalars, written straight into an argument of a call that keeps nothing of it,
  takes no keeping callback and can only answer with a plain scalar (`len(mkList())`).
  Nothing can keep such a result, so the statement can adopt what it reaches as a
  binding would (claim at once, drain with the owning frees at the end). With that in
  place the report's program measured `163 / 163 / 0` (alloc / free / live, N=20)
  against the bound control's `163 / 163 / 0` and the base's `143 / 103 / 7040`, and
  printed `n=2000` under `ulimit -n 64`. A result reaching a resource-union box must
  stay out of it (bug-1012).
- **The other positions need a language decision**: either every bind, assignment,
  global store or `RETURN` whose value contains such a call adopts what it handed
  over (then the statement may close the rest), or such a call must be bound first.

