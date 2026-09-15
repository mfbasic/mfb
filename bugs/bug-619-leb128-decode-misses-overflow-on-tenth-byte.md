# bug-619: `encoding::sleb128Decode` (and likely `uleb128Decode`) silently accepts an overlong tenth byte instead of raising overflow

Last updated: 2026-09-13
Effort: small
Severity: MED
Class: Correctness (silent wrong result)

Status: Open
Regression Test: none yet — see Phase 1

`mfb man encoding sleb128Decode` promises: "The accumulated shift may not exceed
`63` bits; a sequence encoding more than 64 significant bits overflows." It does
not. Nine continuation bytes followed by a tenth byte whose payload exceeds the
single bit that position can hold decode silently to a wrong value:

```
sleb128Decode([0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x02]) -> 0
```

The tenth byte starts at bit 63, so of its 7-bit payload only bit 0 fits in an
`Integer`. `0x02` sets bit 64, which is lost, and the result is `0` with no error.

**The single correct behavior a fix produces:** a LEB128 sequence whose value does
not fit in a 64-bit `Integer` raises `ErrInvalidFormat` ("leb128 overflow"). For
`sleb128Decode`, that includes a tenth byte whose payload is not a valid sign
extension of bit 63. For `uleb128Decode`, it includes any tenth byte whose payload
is above `1`. Every in-range sequence decodes exactly as today.

Found by plan-125-C Phase 2's Codex page review of `sleb128Decode`
(`planning/plan-125-findings/C-phase2/man-page-encoding-sleb128Decode.md`,
finding 3), and confirmed with the release binary (`/tmp/p125-ex/encpuny`).
**Filed, not fixed**, by user instruction during a documentation-only plan
("file all bugs, make no fixes"). Until the fix, the page no longer promises
overflow detection.

## Reproduction

```basic
IMPORT io
IMPORT encoding

SUB main()
  LET over AS List OF Byte = [toByte(128), toByte(128), toByte(128), toByte(128), toByte(128), toByte(128), toByte(128), toByte(128), toByte(128), toByte(2)]
  LET v = toString(encoding::sleb128Decode(over)) TRAP(e)
    RECOVER "raised " & toString(e.code)
  END TRAP
  io::print(v)
END SUB
```

Observed (macos-aarch64, `worktree-P-125`): `0`. Expected: `raised 77050003`.

## Observed on the sibling decoders (2026-09-14, `/tmp/p125-ex/encutf`)

The predicted blast radius is confirmed by probe, not just by the shared check:

| Call | Result | Expected |
|---|---|---|
| `uleb128Decode([0x80 x9, 0x02])` | `0` | `raised 77050003` |
| `varintDecode([0x80 x9, 0x02])` | `0` | `raised 77050003` |
| `uleb128Decode([0xFF x9, 0x01])` | **`-1`** | `raised 77050003`: the value is 2^64-1, which no `Integer` holds |

The last row is a second symptom of the same missing check: a tenth byte that sets
bit 63 turns an unsigned decode negative, contradicting the page's "the result is
always non-negative". Until the fix, `mfb man encoding uleb128Decode` and
`varintDecode` no longer promise overflow detection or a non-negative result.

## Root cause

`src/codegen/builtins/encoding/func_sleb128_decode.rs:BODY` checks
`IF shift > 63 THEN FAIL error(77050003, "leb128 overflow")` **before** reading
each byte. The tenth byte is therefore read at `shift = 63`, and its payload is
shifted left by 63, silently discarding every bit above the lowest.
`func_uleb128_decode.rs:BODY` has the identical pre-read check, so it is expected
to show the same defect; confirm in Phase 1. `varintDecode` decodes through the
`uleb128` helper and inherits whatever `uleb128Decode` does.

## Non-goals

- Changing results for any in-range sequence.
- Changing the truncated or empty-input errors, which work (`[]` and `[0x80]` →
  `77050003`).

## Blast-radius audit

- `uleb128Decode`: same check, expected same defect.
- `varintDecode`: decodes via `__encoding_uleb128Decode`, so it inherits the fix.
- Any other hand-written LEB128 reader in the tree (`grep -rn 'leb128' src/`).

## Fix

Phase 1 — RED tests: `sleb128Decode` and `uleb128Decode` of nine `0x80` plus
`0x02` raise `77050003`; the maximum and minimum valid 64-bit values still decode.
Commit:

Phase 2 — at `shift = 63`, validate the tenth byte's payload (and require it to
terminate) (GREEN); full suite. Commit:
