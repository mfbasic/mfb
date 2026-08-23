//! `fs::tempDirectory` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::tempDirectory` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_temp_directory(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_directory::lower_fs_temp_directory_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Return the host temporary directory path"#;
const DESC: &str = r#"`fs::tempDirectory` returns the path of the host's temporary directory as a
UTF-8 `String`. This is the same location `fs::createTempFile` uses when it is
called without a `directory` argument; that zero-argument form is lowered to
supply `fs::tempDirectory()` as the directory automatically.

The directory path is queried from the operating system on every call rather
than cached, so the result reflects the host environment at the moment of the
call. The returned `String` holds only the path bytes, with the trailing NUL
that the host query produces stripped off; no trailing path separator is added.

The source of the path is platform specific:

- On macOS the per-process Darwin user temporary directory is read with
  `confstr(_CS_DARWIN_USER_TEMP_DIR, ...)`, a user-private location under the
  system temporary area. The reported length is the returned size minus one, to
  drop the terminating NUL.
- On Linux the value of the `TMPDIR` environment variable is used when it is set,
  non-empty, and fits within the internal buffer; otherwise the path falls back
  to `/tmp`.

The function takes no arguments and has no filesystem side effects: it neither
creates the directory nor verifies that it exists, it only reports the
configured path. Internally it reads into a fixed 4096-byte buffer before
copying the result into an arena-backed `String`."#;
const EX: &str = r#"Read and print the host temporary directory:

```
IMPORT fs
IMPORT io

SUB main()
  LET dir AS String = fs::tempDirectory()
  io::print(dir)
END SUB
```

Create a temporary file under the host temporary directory:

```
IMPORT fs

SUB main()
  RES f = fs::createTempFile()
  ' f is created under fs::tempDirectory() and closed by lexical drop
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "tempDirectory",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_fs_temp_directory),
        }],
    });
}
