# bug-1106: the native encoder accepts a floating-point register as the source of an integer store and stores another register's bits

Last updated: 2026-10-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (codegen robustness)

Status: Open
Regression Test: none yet (an encoder unit test that `str_u64` with a `d` source is refused)

Found by bs24-agent-f while fixing bug-845, reproduced by the bug-staging-24 coordinator,
and placed in a later phase.

An ncode `str_u64` whose `src` is a floating-point register (`d0`…) encodes without error,
as the general-purpose register with the same number, so the store writes some other
value. bug-845's fix corrected the one caller found (a Float `MATCH` scrutinee held in a
floating-point register was spilled with `str_u64`); the encoder still accepts the shape,
so the next caller that does the same is silently wrong too.

**The single correct behavior a fix produces:** a general-purpose memory instruction given
a floating-point register (or the reverse) fails to encode with an internal error naming
the instruction, instead of encoding a different register.

## Reproduction (main `c60f81037`; the symptom bug-845 fixes on staging)

```
IMPORT io

FUNC main() AS Nothing
  LET z AS Float = 0.25 + 1.25
  MATCH z
    CASE 1.5
      io::print("one-point-five")
    CASE ELSE
      io::print("else")
  END MATCH
END FUNC
```

`mfb build -q . && ./build/<name>.out` prints `else`.
