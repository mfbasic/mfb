# bug-1062: `TYPE_STATE_MISMATCH` for a bare file given to an imported FUNC value has no file and line

Last updated: 2026-10-11
Effort: small (<1h)
Severity: LOW
Class: Diagnostics

Status: Open
Regression Test: (proposed) a syntax fixture with a source dependency expecting the located diagnostic

`LET j = stpkg::bump`, where the imported `bump` takes `fs::File STATE Cur`, then `j(f)` with a bare `f`, is correctly refused, but the diagnostic carries no location. The same call through a reference to a FUNC of the project is located (`main.mfb:15`, bug-1054's fixture).

**Correct behavior:** the diagnostic names `src/main.mfb:7`, the line of `j(f)`.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and with the batch 22 staging compiler (8592e4505): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters the program prints to stderr at exit (`N=<n> SRC=<dir> /tmp/bs22/bin/probe.sh <mfb> <name>`). `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The result is the same on both compilers unless a line says otherwise.

`cd /tmp/bs22-agent-b/pk && <mfb> build -q .` prints, with no file and line:

```
error: TYPE_STATE_MISMATCH: Argument 1 for `j` is declared `STATE stpkg.Cur` but carries no STATE; a parameter cannot attach one — declare `STATE stpkg.Cur` on the owning binding.
```

`project.json`:

```
{ "name": "pk", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "packages": [ { "name": "stpkg", "version": "=0.1.0", "source": "file:packages/stpkg", "direct": true, "requiredBy": [] } ],
  "entry": "main", "targets": ["native"] }
```

`src/main.mfb`:

```
IMPORT io
IMPORT fs
IMPORT stpkg
FUNC main AS Integer
  LET j = stpkg::bump
  RES f AS fs::File = fs::createTempFile()
  io::print(toString(j(f)))
  RETURN 0
END FUNC
```

`packages/stpkg/src/lib.mfb (a `kind: package` project named `stpkg`, version 0.1.0)`:

```
IMPORT fs
EXPORT TYPE Cur
  n AS Integer
END TYPE
EXPORT FUNC bump(x AS fs::File STATE Cur) AS Integer
  x.state.n = x.state.n + 1
  RETURN x.state.n
END FUNC
```

## What is known

Not traced.

Found during batch 22 (reported by bs22-agent-b, reproduced by the coordinator).
