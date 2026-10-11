# bug-1055: a bare top-level `RES` binding whose initializer carries `STATE` is accepted

Last updated: 2026-10-11
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (diagnostics)

Status: Open
Regression Test: (proposed) a syntax test with the program below expecting `TYPE_STATE_MISMATCH` on `g`

`RES g AS fs::File = openTagged()` at the top level, where `openTagged` returns
`RES fs::File STATE Cur`, builds and runs. The same binding inside a function is refused.

**Correct behavior:** the top-level binding is refused with the diagnostic the local binding
gets (below), because language spec 15 says a binding that wants only the handle must still
restate the `STATE` its initializer carries.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello
").

`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb bb_s4`: the build succeeds,
exit 0, empty stdout, `alloc_calls 7`, `free_calls 5`, `live_bytes 144`.

```
IMPORT io
IMPORT fs
IMPORT tcp
IMPORT thread
TYPE Cur
  n AS Integer
  s AS String
END TYPE
TYPE Other
  k AS Integer
END TYPE
UNION Handle
  fs::File
  tcp::Socket
END UNION
FUNC openTagged() AS RES fs::File STATE Cur
  RES f AS fs::File STATE Cur = fs::createTempFile()
  f.state.n = 42
  f.state.s = "tag"
  RETURN f
END FUNC
FUNC bump(x AS fs::File STATE Cur) AS Integer
  x.state.n = x.state.n + 10
  RETURN x.state.n
END FUNC
FUNC bare(x AS fs::File) AS Integer
  RETURN 1
END FUNC
FUNC other(x AS fs::File STATE Other) AS Integer
  RETURN x.state.k
END FUNC

RES g AS fs::File = openTagged()
FUNC main AS Integer
  RETURN 0
END FUNC
```

The equivalent local binding (`bb_s4l`, the same `Cur` and `openTagged`, with
`RES g AS fs::File = openTagged()` as the first statement of `main`) is refused:
`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb bb_s4l`: `BUILD FAIL`,

```
/tmp/bs21/p/bb_s4l/src/main.mfb:14 error[2-203-0129 TYPE_STATE_MISMATCH]: a resource's STATE type is fixed at its owning binding and every other declaration of it must agree
               binding `g` is bare but its initializer carries STATE `Cur`; a bare binding asserts the resource has no state — declare `STATE Cur`.
```

```
IMPORT io
IMPORT fs
TYPE Cur
  n AS Integer
  s AS String
END TYPE
FUNC openTagged() AS RES fs::File STATE Cur
  RES f AS fs::File STATE Cur = fs::createTempFile()
  f.state.n = 42
  f.state.s = "tag"
  RETURN f
END FUNC
FUNC main AS Integer
  RES g AS fs::File = openTagged()
  RETURN 0
END FUNC
```

## What is known

The local and the top-level binding take different checks; the top-level path does not run
the agreement check that produced `TYPE_STATE_MISMATCH` above. Which function holds each
check was not looked up. bug-1030 (fixed in the batch 21 tree, `bugs/completed/bug-1030-*.md`; not in main at
dcf790e9a, so the measurements above do not include it) fixed the `STATE` clause of a
top-level `RES` binding being dropped. Whether `bb_s4` is still accepted with that fix was not
measured. Its accept test `tests/syntax/resources/bug1030_state_agreement_invalid`
covers `RES topBad AS fs::File STATE Other = openTagged()` (refused) but has no bare top-level
case.

Found during batch 21.
