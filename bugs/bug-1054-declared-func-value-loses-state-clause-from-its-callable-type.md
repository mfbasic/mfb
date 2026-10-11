# bug-1054: a first-class reference to a FUNC with a `STATE` parameter loses the clause from its callable type

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory safety)

Status: Open
Regression Test: (proposed) a runtime test over `bb_hole` asserting a compile-time refusal, and over `bb_fref` asserting exit 0 and stdout `1`, `2`, `2`

`LET j = p`, where `p` is a declared `FUNC` whose parameter is `fs::File STATE Cur`, gives `j`
a callable type without the `STATE Cur` clause. Calling `j` with a bare file then builds and
crashes with SIGSEGV; calling `j` with a file that carries `STATE Cur` is refused with an
unlocated `TYPE_CALL_ARGUMENT_MISMATCH`.

**Correct behavior:** `j` has the type `FUNC(fs::File STATE Cur) AS Integer`, the same as `p`.
`j(bareFile)` is refused at compile time with the `TYPE_STATE_MISMATCH` that `p(bareFile)`
gets. `j(statefulFile)` builds and runs: in `bb_fref`, the program prints `1`, `2`, `2` (by
hand, not measured: `j(f)` makes `n` 1, `p(f)` makes it 2, `f.state.n` reads 2).

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello
").

`bb_hole` (`j` called with a bare file):
`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb bb_hole`: the build succeeds,
the program dies with `Segmentation fault: 11`, exit 139, empty stdout. `N=40` was not run (the
program does not read `N`).

```
IMPORT io
IMPORT fs
TYPE Cur
  n AS Integer
END TYPE

FUNC p(x AS fs::File STATE Cur) AS Integer
  x.state.n = x.state.n + 1
  RETURN x.state.n
END FUNC

FUNC main AS Integer
  LET j = p
  RES f AS fs::File = fs::createTempFile()
  io::print(toString(j(f)))
  RETURN 0
END FUNC
```

`bb_fref` (`j` called with a file that carries `STATE Cur`):
`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb bb_fref`: `BUILD FAIL`, with one
diagnostic that has no file and line:

```
error: TYPE_CALL_ARGUMENT_MISMATCH: Argument 1 for `j` has type fs.File STATE Cur, expected fs.File.
```

```
IMPORT io
IMPORT fs
TYPE Cur
  n AS Integer
END TYPE

FUNC p(x AS fs::File STATE Cur) AS Integer
  x.state.n = x.state.n + 1
  RETURN x.state.n
END FUNC

FUNC main AS Integer
  LET j = p
  RES f AS fs::File STATE Cur = fs::createTempFile()
  io::print(toString(j(f)))
  io::print(toString(p(f)))
  io::print(toString(f.state.n))
  RETURN 0
END FUNC
```

## What is known

Reported cause, not verified: `function_types` drops the `STATE` clause from a declared FUNC's
callable type when the FUNC is named as a value. bug-1030 (fixed in the batch 21 tree, `bugs/completed/bug-1030-*.md`; not in main at
dcf790e9a, so these measurements do not include it) fixed the `LAMBDA` form only (its accept test `tests/syntax/resources/bug1030_state_agreement_invalid` builds
`LET j = LAMBDA(x AS fs::File STATE Cur) -> x.state.n` and refuses a bare argument to `j`
with `TYPE_STATE_MISMATCH`); the declared-FUNC form is the same defect through a different
path. With the clause dropped, the compiler treats `j`'s parameter as a bare `fs::File`, so a
bare file reaches a body that reads `x.state` and takes the state block from memory that does
not hold one (guess for the crash; not traced).

Found during batch 21.
