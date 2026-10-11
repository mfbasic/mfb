# bug-1068: a module-level `RES g AS fs::File STATE Cur` leaves 128 bytes live at exit

Last updated: 2026-10-11
Effort: small (<1h)
Severity: LOW
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` test asserting `live_bytes 0`

A program whose only resource is a top-level `RES g AS fs::File STATE Cur = …` exits with two allocations not freed, 128 bytes. The amount is constant (it does not grow with `N`).

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036). **Open question for the owner:** whether a constant amount held by a module-level binding until process exit counts.

**Correct behavior:** `live_bytes 0` at exit, if the owner's rule covers module-level bindings.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and with the batch 22 staging compiler (8592e4505): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters the program prints to stderr at exit (`N=<n> SRC=<dir> /tmp/bs22/bin/probe.sh <mfb> <name>`). `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The result is the same on both compilers unless a line says otherwise.

`N=10` and `N=40`: exit 0, stdout `1`, `alloc_calls 7`, `free_calls 5`, `live_bytes 128`.

`n9_module_stateful`:

```
IMPORT io
IMPORT fs
TYPE Cur
  n AS Integer
END TYPE
RES g AS fs::File STATE Cur = fs::createTempFile()
FUNC p(x AS fs::File STATE Cur) AS Integer
  x.state.n = x.state.n + 1
  RETURN x.state.n
END FUNC
FUNC main AS Integer
  io::print(toString(p(g)))
  RETURN 0
END FUNC
```

## What is known

Not traced. Which two blocks stay (the record, the `STATE` block, an I/O buffer) was not identified.

Found during batch 22 (reported by bs22-agent-b, reproduced by the coordinator).
