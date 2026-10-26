//! `fs::createTempFile` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes. Returns a `File`
//! resource for a freshly created temp file; `directory` is optional.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::createTempFile` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_create_temp_file(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_temp_file::lower_fs_create_temp_file_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Securely create and open a unique, freshly named temporary file"#;
const DESC: &str = r#"`fs::createTempFile` creates a brand-new file with a unique, unpredictable name
and returns an open `File` resource referring to it. The name has the form
`mfb-<uuid>.tmp`, where `<uuid>` is a freshly generated version 4 UUID rendered
in the canonical 8-4-4-4-12 hexadecimal form. The random bytes that seed the
name are drawn from host entropy before the file is opened, so two calls
effectively never collide and the name cannot be guessed by another process.

The file is opened read/write with exclusive-create semantics and permission
bits `0600` (octal), so the call always yields a freshly created, empty file
readable and writable only by the current user. Exclusive creation means the
call fails rather than reusing or truncating any pre-existing file, which
together with the random name closes the classic temporary-file race and
symlink-redirection attacks. The file is also opened so that a child process started later does not inherit
it.

Without an argument the file is created inside the host temporary directory, the
same location returned by `fs::tempDirectory`; that directory path is supplied
automatically as the `directory` argument for the zero-argument form. With a
`directory` argument the file is created directly inside that directory. The
argument names the containing directory, not the file — no name component of
your own is added. The directory must already exist and be writable, since the
new file is created there.

`directory` is interpreted as UTF-8 bytes and passed to the host filesystem. It
may be absolute or relative to the current working directory and may contain
Unicode characters when the host filesystem accepts those names. It must not be
empty and must not contain an embedded NUL byte, because the host `open` call
requires a NUL-terminated path.

The returned `File` is positioned at the start of the empty file and is yours to use. It is closed when the binding that holds it goes out of
scope, or explicitly with `fs::close`. The file itself is not deleted on close;
removing it is the caller's responsibility. The call does not report the name it
chose, so to remove the file later, create it in a directory of your own (pass
that directory as `directory`) and find it there with `fs::listDirectory` before
calling `fs::deleteFile`; a file made by the zero-argument form in the shared
temporary directory cannot be told apart from other programs' files by name.

An empty `directory`, or one containing a NUL byte, raises `ErrInvalidArgument`.
When the file cannot be created the error says why: `ErrPathNotFound` if
`directory` does not exist, `ErrAccessDenied` if it may not be written,
`ErrInvalidPath` if `directory` cannot hold a file — it is a file itself, or a
name in it is too long — `ErrAlreadyExists` if the generated name is already
taken, which the random name makes practically impossible, and `ErrWriteFailed`
for any other failure. `ErrOutOfMemory` means the program ran out of memory."#;
const EX: &str = r#"Create a temporary file in a directory of your own, write to it, and remove it.
The call does not report the generated name, so creating the file in a scratch
directory is what lets `fs::listDirectory` find it again for `fs::deleteFile`:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET dir AS String = fs::pathJoin([fs::tempDirectory(), "mfb-createTempFile-1"])
  fs::createDirectories(dir)
  RES f = fs::createTempFile(dir)
  fs::writeAll(f, "data")
  fs::close(f)
  FOR EACH name IN fs::listDirectory(dir)
    io::print(fs::pathExtension(name))
    fs::deleteFile(fs::pathJoin([dir, name]))
  NEXT
  fs::deleteDirectory(dir)
END FUNC
```

Use a temporary file as scratch space: write, go back, and read it:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET dir AS String = fs::pathJoin([fs::tempDirectory(), "mfb-createTempFile-2"])
  fs::createDirectories(dir)
  RES g = fs::createTempFile(dir)
  fs::writeAll(g, "scratch data")
  fs::seek(g, 0)
  io::print(fs::readAll(g))
  fs::close(g)
  FOR EACH name IN fs::listDirectory(dir)
    fs::deleteFile(fs::pathJoin([dir, name]))
  NEXT
  fs::deleteDirectory(dir)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "createTempFile",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "directory",
                desc: "The path of an existing, writable directory in which to create the \
                       temporary file, as UTF-8 bytes; absolute or relative to the current \
                       working directory. Must be non-empty and free of embedded NUL bytes. When \
                       omitted, the host temporary directory is used.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::Optional,
            }],
            return_type: ParameterType::named(super::FILE_TYPE_ID),
            errors: vec![
                "ErrOutOfMemory",
                "ErrWriteFailed",
                "ErrAccessDenied",
                "ErrInvalidArgument",
                "ErrAlreadyExists",
                "ErrInvalidPath",
                "ErrPathNotFound",
            ],
            body: Body::abi_function(lower_fs_create_temp_file),
        }],
    });
}
