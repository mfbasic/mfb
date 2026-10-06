# bug-983: a function that slices a `MUT String` and `fs::pathJoin`s it onto an owned root dies with SIGBUS in the string drop (`http::respondPath("//etc/hosts")` crashes the server)

Last updated: 2026-10-06
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: (proposed) tests/rt-behavior/fs/pathjoin-absolute-rel-owned-root-rt, plus a `func_http_respondPath_valid` case for a `//`-prefixed `*` capture

A function that (1) reassigns a `MUT rel AS String` from a parameter, (2) re-slices it with
`strings::mid`, (3) joins it onto a root with `fs::pathJoin([root, rel])`, and (4) is called
with `root` an owned `String` (`fs::tempDirectory()`, not a literal) and `rel` starting with
`//` dies with SIGBUS (exit 138) when the frame drops its strings. The fault is at
`_mfb_arena_fill_random` under `_mfb_arena_free` from `_mfb_rt_drop_owned_string_cap`, writing
past the end of a mapped arena chunk. That is a free with the wrong size, or a free of the
wrong block. It happens at -O0 through -O3, and whichever way the function returns.

This is reachable remotely. `http::respondPath` has exactly this shape: it reassigns
`MUT rel`, slices off one leading `/`, then calls `fs::pathJoin([root, rel])`. A request whose
`*` capture is `//etc/hosts` (an absolute `rel` after the slice) kills the server process.

**Correct behavior:** each `String` is freed exactly once, with its own size. The function
returns its value. `http::respondPath` answers 403 for an absolute path that escapes `root`
(or 404 if it does not exist), and the server keeps running.

## Failing Reproduction

Measured by the coordinator on 2026-10-06 with main at c6dd68acc
(`/tmp/main-dbg-target/release/mfb`), macOS aarch64.

```
mfb init u   # src/main.mfb:
IMPORT fs
IMPORT io
IMPORT strings

FUNC check(s AS String, root AS String) AS Integer
  MUT rel AS String = ""
  rel = s
  IF strings::startsWith(rel, "/") THEN
    rel = strings::mid(rel, 1, len(rel) - 1)
  END IF
  LET candidate AS String = fs::pathJoin([root, rel])
  IF fs::fileExists(candidate) = FALSE THEN
    RETURN 404
  END IF
  MUT within AS Boolean = FALSE
  within = fs::isWithin(root, candidate) TRAP(e)
    RECOVER FALSE
  END TRAP
  IF within = FALSE THEN
    RETURN 403
  END IF
  RETURN 200
END FUNC

FUNC main AS Integer
  io::print(toString(check("//etc/hosts", fs::tempDirectory())))
  RETURN 0
END FUNC

mfb build -q -O <0|1|2|3> u && ./u/build/u.out; echo rc=$?
```

- Observed: no output, `rc=138` at each of `-O 0`, `-O 1`, `-O 2`, `-O 3`. It is the same
  with `"//etc/nonexistent_zz"`, which takes the `RETURN 404` path.
- Expected: `403`, rc 0.
- Controls (all rc 0):
  - `"/etc/hosts"` (only one slash, so `rel` is relative after the slice) prints `404`.
  - The same program with root `"/tmp"` (a literal) prints `403`.
  - With the `MUT rel` / `strings::mid` steps removed (`rel` passed straight in), it prints `403`.
- `mfb dbg` on a `--debug -O0` build:
  `--[XC] … F: _mfb_arena_fill_random+0x2e0 X: a bad memory access`, with backtrace
  `mfb_arena_fill_random ← mfb_arena_free ← mfb_rt_drop_owned_string_cap ← check ← main`.

The http form: a `respondPath` probe with `*` = `"//etc/hosts"` and root
`fs::pathJoin([fs::tempDirectory(), "x"])` dies the same way, with backtrace
`mfb_arena_fill_random ← mfb_arena_free ← mfb_rt_drop_owned_string ← http::respondPath`.
It is the same at main and with bs16-agent-b's bug-862 fix, which does not touch this path.

## Suspected root cause (guess, not verified)

`fs::pathJoin` restarts at an absolute component, so for an absolute `rel` its result may be
`rel`'s own block (or a view into it) rather than a fresh copy. The frame would then free that
block twice, or free it with the other string's capacity. Start by checking what
`fs::pathJoin([root, "/abs"])` returns and how `candidate`'s ownership is recorded.

## Related

- Found by the coordinator while reviewing bug-862 (`http::respondPath` existence oracle).
