//! `fs::close` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::close` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_close(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_handle::lower_fs_close_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        true,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Close an open `File` resource and release its operating-system handle"#;
const DESC: &str = r#"`fs::close` closes an open `File` and releases the operating-system resources behind it,
then returns nothing. Before it closes, it drains any output held in
the handle's per-handle buffer (see `fs::setBuffered`) so buffered on-disk data is
never stranded; the drain is a no-op on an unbuffered handle, which is the default.

**The `File` is marked closed even when the close itself fails.** That is
deliberate: on some hosts a failed close has still let the file go, and a handle
left usable could send a later write to whatever the host has since opened in
its place. So any later `fs::` call on the same `File` is refused, and a second
`fs::close` raises rather than closing something else.

Closing is otherwise automatic. Every `File` returned by `fs::open`, `fs::openFile`,
`fs::openFileNoFollow`, `fs::openWithin`, or `fs::createTempFile` is closed by
itself when the `RES` binding that holds it goes out of scope, and that drains
the buffer the same way. Call `fs::close` only when the file must be closed
earlier than that — to reopen the same path, to let another process see the
writes, or to keep a long-running program from holding too many files open at
once. Closing a `File` and then letting its binding go out of scope is safe: the
second close does nothing.

Beyond the pre-close flush, `fs::close` reads and writes no file contents of its own.
It is an error to close a `File` that is already closed, including one closed by a
previous `fs::close` on the same value, or by its binding having gone out of
scope. It is likewise an
error to close a handle that `thread::transfer` has moved to another thread: such a
handle is not closed but no longer belongs to this thread, so the call reports that
distinctly."#;
const EX: &str = r#"Open a file and release its handle explicitly:

```
IMPORT fs

SUB main()
  fs::writeText("data.txt", "first line\nsecond line\n")
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
                ty: ParameterType::named(super::FILE_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_fs_close),
        }],
    });
}
