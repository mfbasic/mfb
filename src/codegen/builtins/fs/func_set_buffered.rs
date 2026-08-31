//! `fs::setBuffered` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::setBuffered` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_set_buffered(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_handle::lower_fs_set_buffered_helper(&symbol)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Enable or disable opt-in output buffering for an open `File`"#;
const DESC: &str = r#"`fs::setBuffered` turns per-handle output buffering on or off for a single open
`File`, then returns nothing. Buffering is a per-handle flag stored on the `File`
resource itself, so the call affects only `file` and no other open handle; each
`File` carries its own buffer and its own enabled flag.

Buffering is **off by default**: a freshly opened `File` starts with its buffered
flag clear, so every incremental `fs::writeAll` and `fs::writeAllBytes` reaches
the operating system immediately. Calling `fs::setBuffered(file, TRUE)` sets the
flag; from then on incremental writes to `file` are held in a per-handle buffer
and issued in larger blocks, collapsing a loop of small writes into roughly one
host write per full buffer.

When buffering is on, held output is drained automatically when the buffer fills,
on an explicit `fs::flush(file)`, and when the handle is closed — whether by
`fs::close` or by lexical scope exit of its `RES` binding. Calling
`fs::setBuffered(file, FALSE)` drains any pending bytes first, on a best-effort
basis, and then clears the flag, so switching buffering off never strands data in
the buffer.

Only incremental `fs::writeAll` / `fs::writeAllBytes` writes are buffered. The
whole-file operations (`fs::writeText`, `fs::writeBytes`, and the append and
atomic variants) already issue their output in a single write and are unaffected
by this setting.

Because buffered output is held in memory until it is drained, a hard crash
(`SIGSEGV`, `SIGKILL`, or an abort) can lose bytes that were written but not yet
flushed. Flush or close a buffered handle to make its data durable, and leave
buffering off (the default) when partial-output-on-crash durability matters. A
buffered handle should also be flushed before it is transferred to another
thread, which resets it to unbuffered."#;
const EX: &str = r#"Buffer a loop of small writes and let scope exit flush and close the handle:

```
IMPORT fs

SUB main()
  fs::writeText("events.log", "first line\nsecond line\n")
  LET events AS List OF String = ["started", "ready"]
  RES log = fs::openFile("events.log", "write")
  fs::setBuffered(log, TRUE)
  FOR EACH event IN events
    fs::writeAll(log, event & "\n")
  NEXT
  ' log is flushed and closed automatically at scope exit
END SUB
```

Enable buffering for a bulk write, then flush and disable it before durable work:

```
IMPORT fs

SUB main()
  fs::writeText("report.txt", "first line\nsecond line\n")
  LET header AS String = "id,name\n"
  LET body AS String = "1,alice\n"
  RES out = fs::openFile("report.txt", "write")
  fs::setBuffered(out, TRUE)
  fs::writeAll(out, header)
  fs::writeAll(out, body)
  fs::setBuffered(out, FALSE)   ' drains the pending header and body, then disables
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setBuffered",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File, Boolean"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "file",
                    desc: "An open `File` resource whose buffering mode is being changed.",
                    aliases: &[],
                    ty: ParameterType::named(super::FILE_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "enabled",
                    desc:
                        "`TRUE` to enable output buffering for this handle; `FALSE` to drain any \
                           pending output and disable it.",
                    aliases: &[],
                    ty: ParameterType::Boolean,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_fs_set_buffered),
        }],
    });
}
