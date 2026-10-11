# bug-1018: a `List OF RES fs::File` given where a `List OF RES Handle` (a resource union) is expected is accepted unconverted, so `MATCH` takes no arm

Last updated: 2026-10-10
Effort: small (<1h) once bug-999 and bug-1012 are fixed
Severity: MEDIUM
Class: Correctness

Status: Open (blocked on bug-999 and bug-1012)
Regression Test: (proposed) a resource case in tests/rt-behavior/types/union-container-conversion-rt, plus a `--debug` alloc == free loop

bug-989 (owner decision: a collection of union variants is converted where the union's collection
is expected) covers data unions. The same position with `RES` elements is not converted: the list
is passed as it is, its elements carry no union tag, and `MATCH` matches nothing.

**Correct behavior:** the bug-989 rule applies here too. The list is converted, each handle is
wrapped as the union, `MATCH` takes the `fs::File` arm, the source handle stays open and is
closed once.

**Why it is blocked:** bs18-agent-j implemented the conversion and it was functionally right
(correct arm, source handle open, no double close), but its temporaries were never freed: 102
conversions left 315 allocs / 110 frees / 16,560 B live (`/tmp/bs18j/resl.mfb`, agent-j's
measurement). The causes are bug-999 (an aggregate of resources returned inline is not a temporary)
and bug-1012 (a resource-union box has no owner outside a binding). Land those first, then enable
this conversion and prove `alloc == free`.

## Failing Reproduction

Measured by the coordinator on 2026-10-10, on main's code (`/tmp/main-dbg-target/release/mfb`) and
on bs18-agent-j (bug-989 fixed): the same result on both.

```
IMPORT io
IMPORT fs
IMPORT tcp
IMPORT collections

UNION Handle
  fs::File
  tcp::Socket
END UNION

FUNC describe(hs AS List OF RES Handle) AS Nothing
  FOR EACH h IN hs
    MATCH h
      CASE fs::File(f)
        io::print("file")
      CASE tcp::Socket(s)
        io::print("socket")
      CASE ELSE
        io::print("NO ARM MATCHED")
    END MATCH
  NEXT
END FUNC

FUNC main AS Integer
  MUT files AS List OF RES fs::File = []
  RES f AS fs::File = fs::createTempFile()
  files = collections::append(files, f)
  describe(files)
  RETURN 0
END FUNC
```

- Observed: `NO ARM MATCHED`, exit 0.
- Expected: `file`, exit 0.

## Related

- bug-989 (the data-union form, fixed), bug-999, bug-1012.
