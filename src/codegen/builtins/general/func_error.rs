//! `error` — construct an `Error` value.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, ERROR};

const INTRO: &str = "Construct an `Error` value from a numeric code and a message.";

const DESC: &str = r#"`error` builds an `Error` from a numeric `code` and a human-readable `message`.
It is written as a bare name, with no `IMPORT` and no package prefix.

Building an `Error` does not fail anything by itself — it just makes the value.
`FAIL` is what raises it:

```
FAIL error(77050002, "the port must be between 1 and 65535")
```

The `code` is what a `TRAP` handler can compare against, so use a name from the
`errorCode` package rather than a bare number wherever one fits:
`errorCode::ErrInvalidArgument` is the same value as `77050002` and says what it
means. The `message` is what a person reads, so write it for them — what was
wrong, and ideally what would have been right.

An `Error` you catch carries the same two fields: `err.code` and `err.message`.

See `mfb man errors` for the error model as a whole, and `mfb man errorCode` for
the code names."#;

const EX: &str = r#"Build an error and read its fields back:

```
IMPORT io

SUB main()
  LET e AS Error = error(77050002, "made up")
  io::print(toString(e.code))
  io::print(e.message)
END SUB
```

prints:

```
77050002
made up
```

Raise one with `FAIL`, and catch it:

```
IMPORT io
IMPORT errorCode

FUNC checkPort(port AS Integer) AS Integer
  IF port < 1 OR port > 65535 THEN
    FAIL error(errorCode::ErrInvalidArgument, "port must be 1 through 65535")
  END IF
  RETURN port
END FUNC

FUNC main AS Integer
  LET p AS Integer = checkPort(70000)
  io::print("accepted " & toString(p))
  RETURN 0
TRAP(err)
  io::print("rejected: " & err.message)
  RETURN 0
END TRAP
END FUNC
```

prints:

```
rejected: port must be 1 through 65535
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    // Reserved primitive: legacy `error` had param names but EMPTY overloads → `None`
    // arity. The registry forbids an implementation-less function, so it carries one
    // illustrative implementation; `arity` special-cases `error` back to `None`.
    pkg.add_function(member(
        ERROR,
        (INTRO, DESC, EX),
        ParameterType::named("Error"),
        vec![],
        vec![
            req(
                "code",
                ParameterType::Integer,
                "The numeric code a `TRAP` handler can compare against. Prefer a name from `errorCode` over a literal.",
            ),
            req(
                "message",
                ParameterType::String,
                "The human-readable explanation. Say what was wrong, and ideally what would have been right.",
            ),
        ],
    ));
}
