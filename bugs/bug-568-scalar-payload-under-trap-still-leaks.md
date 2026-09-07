# bug-568: a SCALAR payload under an inline `TRAP` still leaks ~134 B per call — bug-561's residual

Last updated: 2026-09-07
Effort: medium
Severity: **HIGH** (unbounded leak; `TRAP` over an `Integer`-returning function is a very common shape)
Class: Memory / correctness

Status: Open
Regression Test: — (an RSS case beside bug-561's)

## Relationship to bug-561

bug-561 fixed the case where `emit_build_result_inline` **copies a block** into the
`Result` and abandons the producer's own block. That is real and measured:

    strings::mid under TRAP, 400k iterations
      before: 25.6 MB     after: 1.0 MB

**This bug is the part that fix does not reach.** A scalar payload carries no
block, so the block-copy mechanism does not apply — and yet the shape still leaks.

bug-561's report concluded "the leak is **not** type-independent: `Result OF
Integer` never leaked at all." That is wrong for the shape below; it must have
been measured on a different `Integer` producer.

## Failing reproduction

```
IMPORT io
FUNC risky(i AS Integer) AS Integer
  IF i < 0 THEN
    FAIL error(1, "neg")
  END IF
  RETURN i
END FUNC

FUNC main AS Integer
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 400000
    LET n AS Integer = risky(i) TRAP(e)
      RETURN 1
    END TRAP
    total = total + n
    i = i + 1
  END WHILE
  io::print("total=" & toString(total))
  RETURN 0
END FUNC
```

Peak RSS (`/usr/bin/time -l`, macOS arm64), measured at two counts because a
one-shot cannot distinguish a leak from an allocator high-water mark:

| iterations | peak RSS |
| --- | --- |
| 200 000 | 26.8 MB |
| 400 000 | 52.6 MB |

Clean linear doubling ⇒ **≈134 B per call**.

**Before and after bug-561's fix, with two verifiably different compilers**
(sha256 `6a5e5daa…` vs `1d2df0b3…`): **50.2 MB and 50.2 MB.** Unchanged.

**The control isolates it to the `TRAP`.** The identical loop calling a
non-fallible `plain(i)` with no `TRAP` is **1.06 MB flat** at 400 000.

## What is notable about it

- The producer **never fails** in this repro, so no `Error` is ever constructed.
  Whatever is allocated is on the SUCCESS path.
- The payload is a scalar, so there is no payload block to own — which is exactly
  why bug-561's block-provenance fix does not fire.
- The shape is ordinary: `TRAP` over a user function returning `Integer` is one of
  the most common things a program does.

## What a fix must produce

`LET n AS Integer = <fallible user FUNC> TRAP … END TRAP` in a loop runs at
constant RSS, for every scalar payload type (`Integer`, `Float`, `Boolean`,
`Byte`).

**The hazard is the error branch, not the success branch.** On the failure path
the raw success register holds an error code rather than a block; bug-561's fix
registers its cleanups on the Ok path only for exactly this reason. Any fix here
must do the same, and must be pinned by a fixture where **every** call fails, not
just one where none do.

Follow the fail-closed provenance precedent (bug-536 shapes B and B-2, bug-561):
an unproven case keeps leaking rather than wild-freeing.

## References

- `bugs/bug-561-*` — the block-copy half, and `pending_temp_is_freeable`
- `emit_build_result_inline`
- The measurement above was taken on `d477c5c5d` (base) vs the bug-560/561 branch.
