# bug-1002: `TYPE_COLLECTION_OWNERSHIP_VIOLATION` is reported twice for one collection declaration that holds a thread worker

Last updated: 2026-10-06
Effort: small (<1h)
Severity: LOW
Class: Other (duplicated diagnostic)

Status: Open
Regression Test: (proposed) a syntax fixture with the declaration below whose `build.log` golden carries the error exactly once

A single declaration of a collection whose element is a `ThreadWorker` produces the same
`TYPE_COLLECTION_OWNERSHIP_VIOLATION` twice.

**Correct behavior:** one diagnostic per offending declaration.

Cause (from bs17-agent-h's reading; not traced): two checks both fire. The declared-type check in
the bind arm (`check_map_key_comparable`, `src/ir/verify/ops.rs`) and the `ListLiteral` arm
(`src/ir/verify/values.rs`) each report it, and `check_map_key_comparable` can also report a
nested worker more than once on its own. A fix must decide which check owns the report.

## Failing Reproduction

Measured by the coordinator on 2026-10-06 with main 468ff0f94 (`/tmp/main-dbg-target/release/mfb`).

```
IMPORT io
IMPORT thread
ISOLATED FUNC spin(t AS ThreadWorker OF String TO Integer, n AS Integer) AS Integer
  LET xs AS List OF RES ThreadWorker OF String TO Integer = []
  RETURN n * 2
END FUNC
FUNC main AS Integer
  RES t = thread::start(spin, 21)
  io::print("thread=" & toString(thread::waitFor(t)))
  RETURN 0
END FUNC
```

`mfb build -q p11 2>&1 | grep -c TYPE_COLLECTION_OWNERSHIP_VIOLATION` → `2`. Expected `1`.

Found by bs17-agent-h while resolving the bug-975/bug-843 capture-rule integration.
