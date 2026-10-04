//! `fs::openData` — descriptor + docs (bug-809).
//!
//! Native syscall member with two overloads that cannot share one helper body:
//! a `String` keeps its bytes at `+8`, a `List OF Byte` past its collection
//! header. The `List OF Byte` overload therefore arrives under the
//! `fs.openDataBytes` code form (`builder_values` selects it by the argument's
//! static type), and the one `abi_function` body picks its arm off `ctx.call`.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// The code form the `List OF Byte` overload lowers under.
pub(crate) const OPEN_DATA_BYTES: &str = "fs.openDataBytes";

/// `abi_function` body for `fs::openData` — both overloads; the byte-list one
/// arrives as [`OPEN_DATA_BYTES`].
pub(crate) fn lower_fs_open_data(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_open_data::lower_fs_open_data_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        ctx.call == OPEN_DATA_BYTES,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Open a `File` whose contents are a copy of a value held in memory"#;
const DESC: &str = r#"`fs::openData` returns a `File` whose contents are a copy of `value` — the
UTF-8 bytes of a `String`, or the bytes of a `List OF Byte` — so any code written
against a `File` (`fs::readLine`, `fs::readAll`, `csv::readRow`, …) can read text
the program already has. The data lives in memory, not in any directory: no other
program can see it, nothing is left behind when the `File` is closed, and nothing
needs cleaning up after a crash.

It is an ordinary `File`. Every `fs::` handle operation works on it exactly as on
a file opened from disk — `fs::readLine`, `fs::readAll`, `fs::readAllBytes`,
`fs::readBytesAt`, `fs::writeAll`, `fs::writeAllBytes`, `fs::seek`, `fs::size`,
`fs::eof`, `fs::setBuffered`, `fs::flush` — and it closes the same way: with
`fs::close`, or by itself when its binding goes out of scope. Closing it releases
the memory.

`value` is copied when the `File` is opened. Nothing written to the `File`
afterwards changes `value`, and changing the variable that held `value` does not
change the `File`; read the result back from the `File` itself.

`mode` is one of the `fs::open` modes and means the same thing here, applied to
the copied contents:

* `"read"` or `"r"` — read the data from the start. Writing raises
  `ErrWriteFailed`.
* `"write"` or `"w"` — start empty; `value` is ignored. The `File` is write-only:
  reading raises `ErrReadFailed`.
* `"readWrite"` or `"rw"` — read and write the data, starting at the beginning.
  Writes overwrite from the current position and may grow the data. To read back
  what was written, `fs::seek` to where it starts first.
* `"append"` or `"a"` — keep the data; every write is added at the end. The
  `File` is write-only, as with `"write"`.

The mode string is matched exactly and is case sensitive; any other value raises
`ErrInvalidArgument` and opens nothing. `ErrWriteFailed` means the system could
not create or fill the in-memory file, and `ErrOutOfMemory` that the program ran
out of memory. An empty `value` is fine: it gives an empty file."#;
const EX: &str = r#"Read text that is already in memory one line at a time:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  RES f = fs::openData("alpha\nbeta\n", "r")
  WHILE NOT fs::eof(f)
    io::print(fs::readLine(f))
  END WHILE
END FUNC
```

Build data in a read-write file, then read it back:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  RES f = fs::openData("", "rw")
  fs::writeAll(f, "one\n")
  fs::writeAll(f, "two\n")
  fs::seek(f, 0)
  io::print(fs::readAll(f))
END FUNC
```

Open bytes, including ones that are not text:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET raw AS List OF Byte = [0, 255, 10]
  RES f = fs::openData(raw, "r")
  io::print(toString(fs::size(f)))
END FUNC
```"#;

const MODE_DESC: &str = "The access mode, as for `fs::open`: \"read\"/\"r\" (read the data), \
                         \"write\"/\"w\" (start empty, write only), \"readWrite\"/\"rw\" (read \
                         and write the data, starting at the beginning), or \"append\"/\"a\" \
                         (keep the data, write only, every write added at the end). Matched \
                         exactly and case sensitively.";

const ERRORS: &[&str] = &["ErrInvalidArgument", "ErrOutOfMemory", "ErrWriteFailed"];

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "openData",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String or List OF Byte, String"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "The text the `File` starts with, copied as its UTF-8 bytes. May be \
                               empty.",
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "mode",
                        desc: MODE_DESC,
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::named(super::FILE_TYPE_ID),
                errors: ERRORS.to_vec(),
                body: Body::abi_function_aliased(lower_fs_open_data, &[]),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "The bytes the `File` starts with, copied exactly. May be empty.",
                        aliases: &[],
                        ty: ParameterType::list_of(ParameterType::Byte),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "mode",
                        desc: MODE_DESC,
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::named(super::FILE_TYPE_ID),
                errors: ERRORS.to_vec(),
                body: Body::abi_function_aliased(lower_fs_open_data, &["openDataBytes"]),
            },
        ],
    });
}
