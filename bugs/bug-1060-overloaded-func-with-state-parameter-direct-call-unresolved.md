# bug-1060: a direct call of an overloaded FUNC whose matching overload takes a `STATE` parameter is reported as an unknown identifier

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (overload resolution)

Status: Open
Regression Test: (proposed) a runtime fixture over the program expecting stdout `7`, `1`

`FUNC p(x AS fs::File STATE Cur)` and `FUNC p(x AS Integer)` are one overload set. `p(f)`, with `f` a `RES fs::File STATE Cur`, fails to build; `apply(p)` with a `FUNC(Integer) AS Integer` parameter is accepted.

**Correct behavior:** the program builds; `p(f)` selects the `STATE Cur` overload and the program prints `7` then `1`.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and with the batch 22 staging compiler (8592e4505): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters the program prints to stderr at exit (`N=<n> SRC=<dir> /tmp/bs22/bin/probe.sh <mfb> <name>`). `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The result is the same on both compilers unless a line says otherwise.

Build fails:

```
main.mfb:20 error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: identifier could not be resolved
               Callable `p` is not a top-level function.
```

`n1_overload_state`:

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
FUNC p(x AS Integer) AS Integer
  RETURN x
END FUNC
FUNC apply(fn AS FUNC(Integer) AS Integer) AS Integer
  RETURN fn(7)
END FUNC
FUNC main AS Integer
  RES f AS fs::File STATE Cur = fs::createTempFile()
  io::print(toString(apply(p)))
  io::print(toString(p(f)))
  RETURN 0
END FUNC
```

## What is known

Not traced. The message is the one bug-836 describes for a call that matches no overload; here an overload does match, so the `STATE` clause on the parameter is probably what the overload match does not accept (guess).

Found during batch 22 (reported by bs22-agent-b, reproduced by the coordinator).
