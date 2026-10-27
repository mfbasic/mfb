//! `fs::appendBytes` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::appendBytes` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_append_bytes(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_atomic_write::lower_fs_write_path_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
            true,
            true,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str =
    r#"Append a `List OF Byte` to the end of a file, preserving its existing contents"#;
const DESC: &str = r#"`fs::appendBytes` opens the file named by `path`, creating it with no contents
if it does not already exist, writes the complete contents of `bytes` after
whatever the file already held, flushes the file to disk, closes it, and
returns nothing. Any existing contents are preserved; to replace a file's
contents instead of extending them, use `fs::writeBytes`.

Every append lands at the file's current end, even when another program has
extended the file since it was opened. The bytes are written exactly as the
list holds them, with no encoding, decoding, or newline translation, so the
function suits binary data as well as text. Either every byte of `bytes` is
written, or the call raises `ErrWriteFailed`. An empty list leaves the file's
length unchanged, creating it as an empty file if it did not exist.

When the file is created it gets permissions `0600` — readable and writable only by the user running the program, before the process umask is applied — not the world-readable `0666`. An
existing file keeps its current mode. Symbolic links anywhere in `path` are
followed, so appending through a link appends to the file it points to; use
`fs::openFileNoFollow` when that must be refused.

`path` may be absolute or relative to the current working directory and may
contain Unicode characters when the host filesystem accepts those names. It
must not be empty and must not contain a NUL character.

The file is closed before the function returns, whether the write succeeded or
failed. The append is not atomic: a reader observing the file while the write
is in progress may see only part of the appended bytes, and a failure partway
through leaves the file extended by only the bytes written so far.

An empty `path`, or one containing a NUL character, raises
`ErrInvalidArgument`. When the file cannot be opened the error says why:
`ErrPathNotFound` if a directory in `path` does not exist, `ErrAccessDenied` if
permission is refused, and `ErrInvalidPath` if `path` cannot name a file — it
names a directory, a name in it is too long, a file is used as a directory, a
symbolic link in it loops back on itself, or the host filesystem rejects the
name. Any other refusal, and a failed
write, flush to disk or close, raises `ErrWriteFailed`. `ErrOutOfMemory` means
the program ran out of memory."#;
const EX: &str = r#"Append a single newline byte to a log file in a scratch directory under the
temporary directory, then clean up:

```
IMPORT fs

FUNC main() AS Nothing
  LET dir AS String = fs::pathJoin([fs::tempDirectory(), "mfb-appendBytes-1"])
  fs::createDirectories(dir)
  LET log AS String = fs::pathJoin([dir, "log.bin"])
  LET bytes AS List OF Byte = [10]
  fs::appendBytes(log, bytes)
  fs::deleteFile(log)
  fs::deleteDirectory(dir)
END FUNC
```

Append the contents of one file to the end of another:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET dir AS String = fs::pathJoin([fs::tempDirectory(), "mfb-appendBytes-2"])
  fs::createDirectories(dir)
  LET source AS String = fs::pathJoin([dir, "source.bin"])
  LET combined AS String = fs::pathJoin([dir, "combined.bin"])
  fs::writeText(source, "first line\nsecond line\n")
  fs::writeText(combined, "header\n")
  fs::appendBytes(combined, fs::readBytes(source))
  io::print(fs::readText(combined))
  fs::deleteFile(source)
  fs::deleteFile(combined)
  fs::deleteDirectory(dir)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "appendBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, List OF Byte"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "path",
                    desc: "The filesystem path of the file to append to; absolute or relative to the current working directory. Must be non-empty and contain no NUL character. The file is created if it does not exist.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "bytes",
                    desc: "The bytes to append, written in list order after the file's existing contents. An empty list adds nothing; the file is still created if it did not exist.",
                    aliases: &["value"],
                    ty: ParameterType::list_of(ParameterType::Byte),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![
                "ErrOutOfMemory",
                "ErrWriteFailed",
                "ErrAccessDenied",
                "ErrInvalidArgument",
                "ErrInvalidPath",
                "ErrPathNotFound",
            ],
            body: Body::abi_function(lower_fs_append_bytes),
        }],
    });
}
