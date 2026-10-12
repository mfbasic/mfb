# bug-1063: a concrete resource carrying `STATE` given where its resource union with the same `STATE` is expected is not wrapped: two forms crash with SIGSEGV, one fails to build

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory safety)

Status: Fixed (bs22-agent-g)
Regression Test: `rt_resource_model::bug_1063_state_variant_to_union_{bind,arg,append}`

With `UNION Handle { fs::File, tcp::Socket }`, a `fs::File STATE Cur` value reaches a `Handle STATE Cur` position unwrapped in three forms: a binding from a call (`RES h AS Handle STATE Cur = openS()`, `n4a`), an argument (`rd(s)` with `rd(h AS Handle STATE Cur)`, `n4b`), and an append to a `List OF RES Handle STATE Cur` (`n4c`). The same three forms without `STATE` wrap the value.

**Correct behavior:** all three build and run; by hand (not measured): `n4a` prints `50` at `N=10` (`rd(h)` reads 5 per call). `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and with the batch 22 staging compiler (8592e4505): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters the program prints to stderr at exit (`N=<n> SRC=<dir> /tmp/bs22/bin/probe.sh <mfb> <name>`). `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The result is the same on both compilers unless a line says otherwise.

- `n4a`, `N=10`: the build succeeds, `Segmentation fault: 11`, exit 139, empty stdout.
- `n4b`, `N=10`: the same, exit 139.
- `n4c`: the build fails with an unlocated internal error: `error: native collection list item must be Handle STATE Cur, got fs.File STATE Cur while lowering assign hs while lowering for $for_iter5`.

Sources: `/tmp/bs22-agent-b/new/n4{a,b,c}_*.mfb`.

`n4a_state_variant_to_union_bind`:

```
IMPORT io
IMPORT os
IMPORT fs
IMPORT tcp
IMPORT collections

UNION Handle
  fs::File
  tcp::Socket
END UNION

TYPE HB
  h AS RES Handle
  n AS Integer
END TYPE

FUNC openOne AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC

FUNC countH(hs AS List OF RES Handle) AS Integer
  MUT t AS Integer = 0
  FOR EACH h IN hs
    MATCH h
      CASE fs::File(f)
        t = t + len(fs::readAll(f))
      CASE tcp::Socket(s)
        t = t + 100
    END MATCH
  NEXT
  RETURN t
END FUNC

FUNC countM(hs AS Map OF String TO RES Handle) AS Integer
  MUT t AS Integer = 0
  FOR EACH e IN hs
    MATCH e.value
      CASE fs::File(f)
        t = t + len(fs::readAll(f))
      CASE tcp::Socket(s)
        t = t + 100
    END MATCH
  NEXT
  RETURN t
END FUNC

FUNC main AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    t = t + once()
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC
TYPE Cur
  n AS Integer
END TYPE
FUNC rd(h AS Handle STATE Cur) AS Integer
  RETURN h.state.n
END FUNC
FUNC openS() AS RES fs::File STATE Cur
  RES f AS fs::File STATE Cur = openOne()
  f.state.n = 5
  RETURN f
END FUNC
FUNC once() AS Integer
  RES s AS fs::File STATE Cur = openOne()
  s.state.n = 7
  RES h AS Handle STATE Cur = openS()
  RETURN rd(h)
END FUNC
```

`n4b_state_variant_to_union_arg, `once` and its helpers (the prelude is `n4a`'s, up to `main`)`:

```
TYPE Cur
  n AS Integer
END TYPE
FUNC rd(h AS Handle STATE Cur) AS Integer
  RETURN h.state.n
END FUNC
FUNC openS() AS RES fs::File STATE Cur
  RES f AS fs::File STATE Cur = openOne()
  f.state.n = 5
  RETURN f
END FUNC
FUNC once() AS Integer
  RES s AS fs::File STATE Cur = openOne()
  s.state.n = 7
  RETURN rd(s)
END FUNC
```

`n4c_state_variant_to_union_append, `once` and its helpers`:

```
TYPE Cur
  n AS Integer
END TYPE
FUNC sumS(hs AS List OF RES Handle STATE Cur) AS Integer
  MUT t AS Integer = 0
  FOR EACH h IN hs
    t = t + h.state.n
  NEXT
  RETURN t
END FUNC
FUNC once() AS Integer
  MUT hs AS List OF RES Handle STATE Cur = []
  FOR j = 1 TO 3
    RES s AS fs::File STATE Cur = openOne()
    s.state.n = j
    hs = collections::append(hs, s)
  NEXT
  RETURN sumS(hs)
END FUNC
```

## What is known

Not traced. Guess: the variant-to-union conversion compares types with the `STATE` clause attached, so `fs.File STATE Cur` is not recognised as a member of `Handle STATE Cur` and no wrap is emitted; the value is then read as a `{tag, record}` box (the crash bug-1046 had for the stateless float).

Found during batch 22 (reported by bs22-agent-b, reproduced by the coordinator).

## Resolution (bs22-agent-g, 2026-10-11)

The general cause is the union rule of bug-1018: membership compared exact spellings, so a variant carrying a `STATE` clause was no member and was never wrapped. `TypeIndex::variant_belongs_to_union` asks the bare types, the wrap carries the payload's `STATE`, the verifier compares the bare payload. The append form's records are freed by the one record-free rule.

Regression tests: `rt_resource_model::bug_1063_state_variant_to_union_{bind,arg,append}`. Each builds the bug's repro (`bugs/repro/`) with `--debug` and requires, at N=10 and N=40, exit 0, the expected total, `double_free_skips 0` and live bytes that do not grow, and exit 0 at N=300 under `ulimit -n 64`.

## Golden impact

Not regenerated. Expect `.ncode` changes wherever a `RES` binding, a resource-union binding or an owned-list is dropped (the take and the reclaim after the exit's last cleanup), wherever a call returns a value reaching resources (the statement's take and drain), and at assignments that float.
