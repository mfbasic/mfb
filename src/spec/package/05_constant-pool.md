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

Constant kinds:

```text
1 = Nothing
2 = Boolean
3 = Integer
4 = Float
5 = Fixed
6 = String
7 = Byte
8 = Error
```

Encoding:

```text
Nothing  payloadLength = 0
Boolean  u8, 0 = FALSE, 1 = TRUE
Integer  i64
Float    u64 IEEE-754 binary64 bit pattern
Fixed    i64 raw signed 32/32 fixed-point value
String   stringId as u32
Byte     u8
Error    code i64, message stringId
```

Float constants must use canonical quiet NaN representation if NaN constants are ever allowed. Implementations may reject NaN constants in source if deterministic behavior is not yet specified.
