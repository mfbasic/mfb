# bug-498: `thread-transfer-bidirectional-rt` intermittently fails with "Resource handle is already closed" under full-suite load

Filed: 2026-09-03
Severity: medium (a flaky acceptance fixture — but the symptom is a resource
lifetime error in the bidirectional thread-transfer path, so the defect may be in
the runtime rather than the test)
Status: OPEN — reproduced once, not yet root-caused

## Symptom

Under a **full** `scripts/test-accept.sh` run, `rt-behavior/threads/thread-transfer-bidirectional-rt`
occasionally fails its `build.log` golden. The fixture's program exits 255 with a
raised error instead of printing its two sizes:

```
 $ tests/rt-behavior/threads/thread-transfer-bidirectional-rt/build/thread_transfer_bidirectional_rt.out
-6
-12
-[exit 0]
+Error: 7-703-0004
+Resource handle is already closed.
+[exit 255]
```

`7-703-0004` is the already-closed resource-handle error.

## How it was found

Observed during plan-122-A Phase 2, in a full acceptance run that was otherwise
green (`acceptance tests failed: 2 mismatch(es) (1378 test(s) ran)` — the other
mismatch was an expected, attributed `color` `.ir` churn).

**It is not attributable to that plan's changes**, which add a new `color` builtin
package that this fixture does not import:

- `scripts/artifact-gate.sh <mfb> all` → `1878 golden(s) checked, 0 diff(s)`, so
  the change is byte-neutral for every program that does not import `color`.
- The same fixture was green in the immediately preceding full run at
  plan-122-A Phase 1, with the same package registered.

So this is a pre-existing intermittent failure that a differently-timed run
surfaced, not a regression.

## Reproduction

Not reproducible in isolation. Measured:

| Condition | Result |
|---|---|
| `scripts/test-accept.sh <mfb> <scratch> 'thread-transfer-bidirectional-rt'`, 10 consecutive runs | 10/10 **pass** |
| Full `scripts/test-accept.sh <mfb> <scratch>` (1378 fixtures, parallel) | 1 failure observed in 3 full runs |

The trigger is therefore machine load / scheduling, not fixture content. Any
repro attempt must run the fixture under concurrent load rather than alone —
`test-accept.sh` filtered to one test will never show it.

## What the fixture does

`tests/rt-behavior/threads/thread-transfer-bidirectional-rt/src/main.mfb` moves a
`RES fs::File` in both directions across a thread boundary:

```
LET t AS Thread OF RES fs::File TO Integer = thread::start(xfer_bidi_worker::exchange, "seed")
RES pf AS fs::File = fs::openFile(".../data/parent.txt")
thread::transfer(t, pf)            ' parent -> worker (inbound queue)
RES wf AS fs::File = thread::accept(t)   ' worker -> parent (outbound queue)
LET parentReceived AS Integer = len(fs::readAll(wf))
fs::close(wf)
LET workerReceived AS Integer = thread::waitFor(t)
```

The worker half is a **compiled** package,
`packages/xfer_bidi_worker.mfp` — note `committed-mfp-goes-stale-on-resource-requalification`
when investigating; confirm the `.mfp` still matches the current resource
qualification before blaming the runtime.

## Lines of enquiry, cheapest first

1. **Which handle is already closed** — `pf` (transferred away, so the parent must
   no longer own it) or `wf` (accepted). Instrument or run under a debugger to
   name it. The transfer-out path is the more likely one: if `thread::transfer`
   can return before the receiving side has taken ownership, a racing cleanup on
   the parent's scope exit would close a handle the worker still holds, and the
   symptom lands wherever it is next touched.
2. **Whether the raise comes from the parent or the worker.** The golden captures
   only the process's merged output, so this is currently unknown and is the
   fact that most narrows the search.
3. `.ai/resources-packages.md` (RES ownership across a transfer) and
   `.ai/canvas-threading.md`'s arena rule — `arena-state-is-per-thread` — for
   whether the handle table is per-thread here.

## Not yet done

- Not root-caused; no RED test yet. A RED test must reproduce **under load**, so
  the affordance to build first is a way to run one fixture with concurrent
  pressure rather than alone.
- Not verified at an older commit. Attribution above rests on byte-neutrality of
  the current change plus the preceding green run, not on a bisect. If a bisect is
  wanted, use `attribution-binary-via-git-archive`, not a sibling worktree.
