# bug-1061: a generic FUNC with a concrete `STATE` parameter beside its `T` parameter cannot be called: template arguments are not inferred

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (templates)

Status: Open
Regression Test: (proposed) a runtime fixture over the program expecting a successful build and its output

`FUNC bump OF T(x AS fs::File STATE Cur, v AS T)` called as `bump(f, 5)` with `f` a `RES fs::File STATE Cur` fails to build. The `STATE` clause is on a concrete parameter, not on `T` (that case is bug-1032 and waits for an owner decision).

**Correct behavior:** the call builds with `T = Integer`.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and with the batch 22 staging compiler (8592e4505): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters the program prints to stderr at exit (`N=<n> SRC=<dir> /tmp/bs22/bin/probe.sh <mfb> <name>`). `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The result is the same on both compilers unless a line says otherwise.

Build fails:

```
main.mfb:12 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: function call argument type does not match parameter type
               Call to `bump` cannot infer template arguments from `fs.File STATE Cur`.
```

`n2_generic_state`:

```
IMPORT io
IMPORT fs
TYPE Cur
  n AS Integer
END TYPE
FUNC bump OF T(x AS fs::File STATE Cur, v AS T) AS T
  x.state.n = x.state.n + 1
  RETURN v
END FUNC
FUNC main AS Integer
  RES f AS fs::File STATE Cur = fs::createTempFile()
  io::print(toString(bump(f, 5)))
  RETURN 0
END FUNC
```

## What is known

Not traced. Near bug-1032 (a `STATE` clause on a generic parameter is instantiated twice).

Found during batch 22 (reported by bs22-agent-b, reproduced by the coordinator).
