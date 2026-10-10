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
