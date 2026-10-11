# bug-1054: a first-class reference to a FUNC with a `STATE` parameter loses the clause from its callable type

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory safety)

Status: Fixed (bs22-agent-b)
Regression Test: `tests/rt-behavior/resources/bug1054_func_ref_state_rt` (`bb_fref` and its siblings), `tests/syntax/resources/bug1054_func_ref_state_invalid` (`bb_hole` and its siblings)

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

## Resolution (bs22-agent-b, 2026-10-11)

The reported cause was right. `function_types` (`src/ir/lower.rs`) built a declared FUNC's
callable type from `HirParam::type_` alone, while the FUNC's own parameters (`lower_param`)
and a lambda's callable type (`lambda_param_type`, bug-1030) fold the parameter's
`state_type` in. It now maps every parameter through `lambda_param_type`, so `LET j = p` is a
`FUNC(fs.File STATE Cur) AS Integer`. Nothing else changed: the two diagnostics the doc asks
for are the existing checks (`check_argument_state_agreement`, the callable-type
compatibility rule) seeing the right type.

The crash, traced with the debugger on the main-compiler build of `bb_hole`
(`printf 'run\nbt\nquit\n' | /tmp/main-dbg-target/release/mfb dbg hole.out`): `a bad memory
access at 0x0000000000000000` in `p+0x14`, called from `main`. `p` loads the state block
pointer of a record whose state slot is null and reads `n` through it, which is the guess in
"What is known".

### Measurements (fixed compiler `target/release/mfb`, macOS aarch64, 2026-10-11)

| program | main's compiler | fixed |
| --- | --- | --- |
| `bb_hole` | builds, exit 139 (SIGSEGV), empty stdout | `BUILD FAIL`, `main.mfb:15 error[2-203-0129 TYPE_STATE_MISMATCH]` "Argument 1 for `j` is declared `STATE Cur` but carries no STATE" |
| `bb_fref` | `BUILD FAIL`, unlocated `TYPE_CALL_ARGUMENT_MISMATCH` | exit 0, stdout `1`, `2`, `2`; `alloc_calls 8`, `free_calls 8`, `live_bytes 0`, `double_free_skips 0` (default level and `-O0`) |

Commands: `P=/tmp/bs22-agent-b/p /tmp/bs22/bin/probe.sh <mfb> bb_hole` and `... bb_fref [-O0]`.

Sibling forms, each a line of one of the two fixtures unless a program is named
(`scripts/test-accept.sh target/release/mfb <out> '*bug1054*'` passes, 2 tests):

| form | main's compiler | fixed |
| --- | --- | --- |
| `apply(p, bare)` where `apply` takes `FUNC(fs::File) AS Integer` (`/tmp/bs22-agent-b/src/s2.mfb`) | builds, exit 139 | `TYPE_CALL_ARGUMENT_MISMATCH`, located |
| the same through the binding, `apply(j, bare)` | accepted | `TYPE_CALL_ARGUMENT_MISMATCH`, located |
| `j(other)`, a file carrying another STATE | accepted | `TYPE_STATE_MISMATCH`, located |
| `LET k AS FUNC(fs::File) AS Integer = p` | accepted | `TYPE_BINDING_MISMATCH` |
| `Holder[p]`, the field a `FUNC(fs::File) AS Integer` | accepted | `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH` |
| `[opaque, p]`, typed and inferred | accepted | `TYPE_LIST_ELEMENT_MISMATCH` |
| `RETURN p` from a `FUNC ... AS FUNC(fs::File) AS Integer` | accepted | `TYPE_RETURN_MISMATCH` |
| inferred list `[p, j]`, `FOR EACH fn IN fns` / `fn(g)` | build fails | runs (`list 42`, `list 43`) |
| `LET again = j`; a second parameter beside the stateful one; a call in a loop | build fails | runs |
| `MUT j = p`, `j = p`, `j = LAMBDA(y AS fs::File STATE Cur) -> ...` (`t2.mfb`) | build fails | exit 0, `1`, `101`, `1`, `8`/`8`/`0` at both levels |
| module-level `LET gj = p`, called from a function and from a lambda (`t3.mfb`) | build fails | exit 0, `3`, `3`, `3`, `14` |
| a STATE on the RETURN type (`LET mk = openTagged`, `RES g AS fs::File STATE Cur = mk()`) | already right (`function_types` kept the return's clause, plan-52-D) | unchanged; a bare or `STATE Other` binding of `mk()` is `TYPE_STATE_MISMATCH` on both (`s7.mfb`) |
| an imported FUNC, `LET j = stpkg::bump` (`/tmp/bs22-agent-b/pk`) | already right: the stateful call runs (`1`, `2`, `2`), the bare call is refused with `TYPE_STATE_MISMATCH` (the export's parameter type carries the clause) | unchanged |

A function type written in source cannot spell a STATE clause (`FUNC(fs::File STATE Cur) AS
Integer` is a parse error, "Function type must close with `)`", on main as well), so a
reference to such a FUNC can be held only where the type is inferred. That is the same limit
the lambda form has since bug-1030, and the spec now says so.

### Golden impact

Four `.ir` goldens change, each by one type string and nothing else: the `functionRef` a
`thread::start(work, ...)` call passes now spells the worker parameter's clause, e.g.
`ISOLATED FUNC(ThreadWorker OF Integer TO Integer, Integer) AS Integer` becomes
`ISOLATED FUNC(ThreadWorker OF Integer TO Integer STATE W, Integer) AS Integer`. `work` is
declared `worker AS ThreadWorker OF Integer TO Integer STATE W`, so this is the fix doing
what it says (the clause of a declared FUNC's parameter rides in its callable type), on the
one existing shape that references such a FUNC as a value.

* `tests/rt-behavior/threads/bug693-worker-state/golden/bug693_worker_state.ir`
* `tests/rt-behavior/threads/thread-view-state-independent/golden/thread_view_state_independent.ir`
* `tests/rt-behavior/threads/thread-worker-state-send-rt/golden/thread_worker_state_send_rt.ir`
* `tests/rt-behavior/threads/threadworker-res-binding-rt/golden/threadworker_res_binding_rt.ir`

`scripts/artifact-gate.sh target/release/mfb all`: 2348 goldens checked, 4 diffs, these four.
`scripts/test-accept.sh target/release/mfb <out> '*thread*' '*worker*' '*state*'` (200 tests):
the same four `.ir` files and nothing else of this bug's, so each of the four still builds,
runs and prints what its `build.log` golden says. The goldens are not edited here (the
batch rule); they are for the coordinator's regeneration.

### Spec

`mfb spec language resource-management` §15.5 (a reference to a declared FUNC carries its
parameters' clauses) and `mfb spec language functions` (the clauses are part of the callable
type).
