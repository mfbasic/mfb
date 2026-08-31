//! `bits::ctz` — count the trailing zero bits of a 64-bit integer.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTRO: &str = r#"Count the trailing zero bits of a 64-bit integer."#;
const DESC: &str = r#"`ctz` returns the number of zero bits *below* the least significant set (`1`) bit
of `value` — equivalently, the bit index of that lowest set bit — counting up
from bit 0 (the lowest bit) toward bit 63. Bits above the lowest set bit do not
participate: `bits::ctz(40)` is `3` whether the value is `40` (`0b101000`) or
`0b1111_1000`, because both have their lowest set bit at index 3.

`value` is treated as a raw two's-complement 64-bit `Integer` bit pattern; `ctz`
does not interpret sign. Negative values are not special-cased the way they are
for `bits::clz`: `bits::ctz(-1)` is `0` (every bit is set), while
`bits::ctz(-2)` is `1`. When bit 0 is set — that is, whenever `value` is odd —
the result is `0`. When `value` is `0` there is no set bit at all, so all 64 bits
count as trailing zeros and the result is `64`; this zero case is the boundary
that most bit-scan primitives leave architecturally undefined, and `mfb` defines
it on every target. The operation is total: it is defined for every `Integer`,
never raises, and has no side effects.

Because the result is the index of the lowest set bit, `ctz` is the primitive
behind alignment and power-of-two work. For a positive power of two,
`bits::ctz(value)` is exactly its base-2 exponent, so it inverts
`bits::sl(1, n)`. A value is `2^k`-aligned exactly when
`bits::ctz(value) >= k`, which is how to test alignment without a modulo. And `ctz`
composes with the lowest-set-bit idiom `value AND -value`, which clears every
bit but the lowest one: iterating "extract lowest bit, `ctz` it, clear it" walks
a bitmask's set indices in ascending order, one iteration per set bit rather than
one per word bit.

Unlike `bits::clz`, `ctz` scans from the bottom of the word, so it is insensitive
to the width of the whole `Integer` and reports the same answer for a value
whether or not it has been narrowed — `bits::ctz(1)` is `0` for an 8-bit field
and for a 64-bit one alike. That makes it the safer of the two when working with
packed sub-fields. The one place width does intrude is the all-zero input, where
the `64` result reflects the `Integer` width rather than your field's. The mirror
operation, counting zeros from the top, is `bits::clz`; for the count of set bits
anywhere in the word see `bits::popCount`. Note the identity
`bits::ctz(value) = bits::popCount(bits::band(value, -value) - 1)`, which holds
for every `value` including `0`.

`ctz` gives the same answer on every platform, but it is the one function here
whose cost varies noticeably between them: it is cheap on arm64 and x86-64 and
markedly more expensive on RISC-V, which lacks the operations it is built from.
On a hot RISC-V path where all you need is a yes/no alignment test, prefer
`bits::band(value, -value)` and a comparison over calling `ctz`."#;
const EX: &str = r#"Count the trailing zeros of `40` (`0b101000`) — its lowest set bit is at index 3:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::ctz(40)
  io::print(toString(result))
END SUB
```

The all-zero pattern has 64 trailing zeros, while any odd value has none:

```
IMPORT bits
IMPORT io

SUB main()
  io::print(toString(bits::ctz(0)))
  io::print(toString(bits::ctz(255)))
  io::print(toString(bits::ctz(-1)))
END SUB
```

Recover the exponent of a power of two, inverting `bits::sl`:

```
IMPORT bits
IMPORT io

SUB main()
  LET n AS Integer = bits::sl(1, 20)
  io::print(toString(bits::ctz(n)))
END SUB
```

Test whether a value is aligned to a `2^k` boundary without a modulo:

```
IMPORT bits
IMPORT io

FUNC isAligned(value AS Integer, k AS Integer) AS Boolean
  IF value = 0 THEN
    RETURN TRUE
  END IF
  RETURN bits::ctz(value) >= k
END FUNC

SUB main()
  io::print(toString(isAligned(4096, 12)))
  io::print(toString(isAligned(4100, 12)))
END SUB
```

Walk the set bits of a mask in ascending index order, one iteration per set bit:

```
IMPORT bits
IMPORT io

SUB main()
  MUT mask AS Integer = 0b1001_0100
  WHILE mask <> 0
    LET lowest AS Integer = bits::band(mask, -mask)
    io::print(toString(bits::ctz(lowest)))
    mask = bits::bxor(mask, lowest)
  END WHILE
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "ctz",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The 64-bit value to inspect. Any `Integer` is accepted; treated as a raw two's-complement bit pattern, not as a signed magnitude.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_inline(lower_bits_ctz),
        }],
    });
}

/// Target-generic call-site lowering for `bits::ctz`.
pub(crate) fn lower_bits_ctz(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let value = &args[0];
    if value.type_ != ParameterType::Integer {
        return Err(format!("bits.ctz does not accept {}", value.type_));
    }
    let dst = builder.allocate_register();
    // `ctz` reverses the bits (`RBIT`) and then counts leading zeros; both `clz`
    // and `ctz` return `64` for a zero input.
    let reversed = builder.allocate_register();
    builder.emit(abi::reverse_bits(reversed, &value.location));
    builder.emit(abi::count_leading_zeros(dst, reversed));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(dst.render()),
        text: format!("bits.ctz({})", value.text),
    })
}
