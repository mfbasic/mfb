# bug-1100: `WITH r { z := … }` naming a field the record does not have passes the checks and fails in native lowering, with no code and no location

Last updated: 2026-10-17
Effort: small (<1h)
Severity: MEDIUM
Class: Other (internal error leaked to the user)

Status: Open
Regression Test: none yet

Found by bs24-agent-a while working bug-1088, reproduced by the bug-staging-24
coordinator, and placed in a later phase.

**The single correct behavior a fix produces:** the program below is refused by the
front end with a coded, located diagnostic naming `z` and the record type, as an unknown
field in a record constructor or a field access is; `mfb check` reports it too.

## Reproduction (main `7781cd71b`)

```
IMPORT io

TYPE Pt
  x AS Integer
  y AS Integer
END TYPE

FUNC main() AS Nothing
  LET a AS Pt = Pt[x := 1, y := 2]
  LET b AS Pt = WITH a { z := 3 }
  io::print(toString(b.x))
END FUNC
```

```
$ mfb build .
Building withz (executable) for macos-aarch64
error: native code WITH update references unknown field 'z' while lowering bind b AS Pt
```

No rule code, no file or line: the message comes from native lowering, after every
front-end check passed. Related: bug-849 (`WITH` bypasses a private field), the same
`WITH` field resolution.
