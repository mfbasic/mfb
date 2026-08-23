//! `fs::close` — descriptor + docs.
//!
//! Native syscall member: its `Body::abi_function` body delegates to the shared
//! family-generic OS-seam dispatcher `native::lower_fs_os_seam`.

use super::native::lower_fs_os_seam;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::close` — the shared OS-seam dispatcher
/// [`super::native::lower_fs_os_seam`], selected by runtime-call name (crypto/io's
/// clean-room shape); the `abi_function` wrapper finalizes it.
pub(crate) fn lower_fs_close(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_fs_os_seam(builder, ctx, "fs.close")
}

const INTRO: &str = r#"Close an open `File` resource and release its operating-system handle"#;
const DESC: &str = r#"`fs::close` releases the operating-system file descriptor behind an open `File`,
then returns nothing. Before releasing the descriptor it drains any output held in
the handle's per-handle buffer (see `fs::setBuffered`) so buffered on-disk data is
never stranded; the drain is a no-op on an unbuffered handle, which is the default.

The `File` is marked closed regardless of the outcome of the underlying `close`. On
some platforms a failing `close` (for example `EINTR` or `EIO`) has still released
the descriptor, so leaving the handle usable would let a later call drain and close
the same descriptor number — which by then may name an unrelated open file. Setting
the closed flag first means any later `fs::` call that takes the same `File` is
refused rather than touching a stale or reused descriptor, and a re-close raises an
error instead of repeating the release.

Closing is otherwise automatic. Every `File` returned by `fs::open`, `fs::openFile`,
`fs::openFileNoFollow`, `fs::openWithin`, or `fs::createTempFile` is closed by
lexical drop when the `RES` binding that holds it leaves scope, and that drop drains
the buffer the same way. Call `fs::close` only when the descriptor must be released
earlier than scope exit — for example to reopen the same path, to let another process
observe writes, or to bound how many descriptors a long-running program holds open at
once. Closing a `File` and then letting it drop is safe: the drop sees the closed
flag and does nothing.

Beyond the pre-close flush, `fs::close` reads and writes no file contents of its own.
It is an error to close a `File` that is already closed, including one closed by a
previous `fs::close` on the same value or by a prior scope-drop. It is likewise an
error to close a handle that `thread::transfer` has moved to another thread: such a
handle is not closed but no longer belongs to this thread, so the call reports that
distinctly."#;
const EX: &str = r#"Open a file and release its handle explicitly:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("data.txt")
  LET line AS String = fs::readLine(f)
  fs::close(f)
END SUB
```

Write a file, then close it before reopening the same path:

```
IMPORT fs
IMPORT io

SUB main()
  RES w = fs::open("out.txt", "write")
  fs::writeAll(w, "hello")
  fs::close(w)
  RES r = fs::open("out.txt", "read")
  io::print(fs::readAll(r))
  fs::close(r)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "close",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "file",
                desc: "The open `File` resource to close, as returned by `fs::open`, \
                       `fs::openFile`, `fs::openFileNoFollow`, `fs::openWithin`, or \
                       `fs::createTempFile`. Must not already be closed or moved.",
                aliases: &[],
                ty: ParameterType::Named(super::FILE_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_fs_close),
        }],
    });
}
