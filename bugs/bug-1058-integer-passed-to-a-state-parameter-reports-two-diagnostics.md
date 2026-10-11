# bug-1058: passing an Integer to a `fs::File STATE Cur` parameter reports two diagnostics for one mistake

Last updated: 2026-10-11
Effort: small (<1h)
Severity: LOW
Class: Diagnostics quality

Status: Open
Regression Test: (proposed) a syntax test over the program below expecting exactly one diagnostic

`wantsCur(1)`, where `wantsCur` declares `x AS fs::File STATE Cur`, reports
`TYPE_CALL_ARGUMENT_MISMATCH` (an Integer is not a `fs::File`) and then
`TYPE_STATE_MISMATCH` claiming the argument "carries no STATE". The second repeats the first
in a misleading form: an Integer is not a resource and has no STATE to carry.

**Correct behavior:** one diagnostic, `TYPE_CALL_ARGUMENT_MISMATCH`.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello
").

`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb wc1`: `BUILD FAIL`,

```
/tmp/bs21/p/wc1/src/main.mfb:12 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: function call argument type does not match parameter type
               Argument 1 for `wantsCur` has type Integer, expected fs.File.
/tmp/bs21/p/wc1/src/main.mfb:12 error[2-203-0129 TYPE_STATE_MISMATCH]: a resource's STATE type is fixed at its owning binding and every other declaration of it must agree
               Argument 1 for `wantsCur` is declared `STATE Cur` but carries no STATE; a parameter cannot attach one — declare `STATE Cur` on the owning binding.
```

```
IMPORT io
IMPORT fs
TYPE Cur
  n AS Integer
END TYPE

FUNC wantsCur(x AS fs::File STATE Cur) AS Integer
  RETURN x.state.n
END FUNC

FUNC main AS Integer
  io::print(toString(wantsCur(1)))
  RETURN 0
END FUNC
```

## What is known

The first diagnostic also prints the expected type as `fs.File`, without the `STATE Cur`
the parameter declares. Not traced: the second diagnostic looks like the agreement check
running on an argument that already failed the type check. No existing bug document covers
it (searched `bugs/` and `bugs/completed/` for `carries no STATE`; only bug-998, completed,
about a `STATE` on a non-resource parameter).

Found during batch 21.
