# Constant Pool

The `CONST_POOL` stores immutable literal values.

```text
constCount      u32
ConstEntry[constCount]
```

## Constant entry

```text
kind            u16
flags           u16
payloadLength   u32
payload         byte[payloadLength]
```

Each entry is `kind` (u16), a reserved/`flags` u16 (always `0`), then a `u32` `payloadLength` and the payload bytes.

Constant kinds:

```text
1 = Nothing
2 = Boolean
3 = Integer
4 = Float
5 = Fixed
6 = String
7 = Byte
8 = Error    (reserved; not currently producible — see below)
```

Encoding:

```text
Nothing  payloadLength = 0
Boolean  u8, 0 = FALSE, 1 = TRUE
Integer  i64 little-endian
Float    u64 IEEE-754 binary64 bit pattern (little-endian)
Fixed    i64 raw signed 32/32 fixed-point value (little-endian)
String   stringId as u32
Byte     u8
Error    code i64, message stringId   (reserved layout; see below)
```

The current compiler's `ConstPool::add` produces kinds `1`-`7` only. Kind `8` (`Error`) has a reserved layout but is **not currently emitted** — there is no source form that lowers an `Error` literal into `CONST_POOL`, and `add` returns an error for any non-scalar constant. The reader will carry an unknown-but-well-formed entry through, but no producer writes one today.

`Fixed` constants are parsed from a decimal string into a 32.32 fixed-point value with round-half-up on the fractional part (`fixed_raw_from_decimal`). [[src/binary_repr/writer.rs:fixed_raw_from_decimal]]

Float constants must use canonical quiet NaN representation if NaN constants are ever allowed. Implementations may reject NaN constants in source if deterministic behavior is not yet specified.

## See Also

* ./mfb spec package metadata-encoding — the table and index conventions shared with the pool
* ./mfb spec package type-table — the type IDs constant entries reference
* ./mfb spec language types — the literal value types stored here
