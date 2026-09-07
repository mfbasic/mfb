# bug-565: the inline-`TRAP` ERROR path leaks ~780 B per trapped error

Last updated: 2026-09-06
Effort: medium
Severity: **HIGH** (unbounded leak on any loop whose fallible call fails)
Class: Memory / correctness

Status: Open
Regression Test: — (a `tests/rt_scope_drop_leaks.rs` RSS case, to be added)

Found while fixing bug-561, which fixed the *success* half of the same lowering.
Measured byte-identical on `19880284452` and after bug-560 + bug-561.

## The finding

```
FUNC always(n AS Integer) AS String
  IF n >= 0 THEN
    FAIL error(7, "always")
  END IF
  RETURN toString(n)
END FUNC
SUB main()
  … WHILE i < N
    LET s AS String = always(i) TRAP(e)
      RECOVER "fallback"
    END TRAP
  … END WHILE
END SUB
```

| N | peak RSS |
| --- | --- |
| 200 000 | **149.8 MB** |
| 400 000 | **298.6 MB** |

Identical on the base compiler and after bug-561, so it is neither caused nor
fixed by that change. ~780 B per trapped error.

## Root cause (stated, not confirmed by a fix)

The `CallResult` error branch in `builder_values.rs` does:

```
let error_register = self.emit_build_error_inline(value_slot, message_slot, source_slot)?;
store error_register -> payload_slot
emit_build_result_inline(tag_slot, "Error", payload_slot)     ' COPIES the Error block in
store -> result_slot
```

and never frees `payload_slot`. This is the exact shape bug-561 fixed on the Ok
branch, and the shape `materialize_current_result`'s adopt path already gets
right — it calls `emit_free_error_block_from_slot(payload_slot)` after the inline
build. `_mfb_build_error_loc`'s block is a second candidate on the same path.

## What a fix must produce

A `TRAP` whose call fails every iteration runs at constant RSS. The existing
positive pin
`a_trap_whose_call_always_fails_still_produces_the_right_value`
(`tests/rt_scope_drop_leaks.rs`) already asserts the recovery VALUE on this exact
program and must stay green; add the RSS half beside it.

**The failure mode to design against is a double free**, exactly as in bug-561:
`emit_build_result_inline` copies the Error block, so freeing the source is
sound — but the adopt path in `materialize_current_result` already frees its own,
and an `ERR_BLOCK` error is a block parked in the current-error slot with another
owner. Follow the fail-closed precedent: free only where the block is provably
this frame's own fresh allocation.
