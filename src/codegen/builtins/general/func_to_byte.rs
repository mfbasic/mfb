//! `toByte` — convert a value to a `Byte`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, TO_BYTE};

const INTRO: &str = "Convert a value to a Byte.";

const DESC: &str = r#"`toByte` converts a value to a `Byte`, an unsigned 8-bit integer holding 0
through 255. It is written as a bare name, with no `IMPORT` and no package
prefix.

**It does not wrap.** A value outside 0 through 255 raises `ErrOverflow` —
`toByte(256)` and `toByte(-1)` both fail rather than silently becoming `0` and
`255`. If you want wrapping, mask first with `bits::band(value, 255)`.

That strictness is the point: a byte that came from arithmetic you did not
bound is usually a bug, and this is where you find out.

`toByte` is also how you produce a `Byte` where a literal will not do — there is
no `Byte` suffix, so a call taking a `Byte` parameter needs `toByte(2)` rather
than a bare `2`. `toString`'s `precision` argument is the one you will meet
first."#;

const EX: &str = r#"Convert in range, and see the out-of-range failure:

```
IMPORT io

SUB main()
  io::print(toString(toByte(255)))
  io::print(toString(toByte(256)))
  EXIT SUB
TRAP(err)
  io::print("toByte raised " & toString(err.code) & " — " & err.message)
  EXIT SUB
END TRAP
END SUB
```

prints:

```
255
toByte raised 77050010 — Arithmetic overflow or numeric conversion outside the destination range.
```

Wrap deliberately, when that is what you want:

```
IMPORT io
IMPORT bits

SUB main()
  LET wide AS Integer = 300
  io::print(toString(toByte(bits::band(wide, 255))))
END SUB
```

prints:

```
44
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        TO_BYTE,
        (INTRO, DESC, EX),
        ParameterType::Byte,
        vec!["ErrOverflow"],
        vec![req(
            "value",
            ParameterType::Integer,
            "The number to convert. Must be 0 through 255 — anything else raises `ErrOverflow` rather than wrapping.",
        )],
    ));
}
