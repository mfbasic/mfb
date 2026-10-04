//! `fs::readLineRaw` — internal-only byte-exact line reader (bug-809).
//!
//! Never user-callable (`internal_only`): the `csv` companion's `File` reader
//! (`csv::parse(fs::File)`, `csv::readRow`) calls it. It is `fs::readLine` with
//! two differences the CSV grammar needs and a public line reader must not have:
//!
//! * the line keeps its terminator (`"a\n"`, `"a\r\n"`, or an unterminated final
//!   `"a"`). A quoted CSV field may span lines and keeps its CR/LF bytes exactly,
//!   which a stripped line cannot reconstruct (it cannot tell LF from CRLF);
//! * end of input returns `""` instead of raising `ErrEndOfFile`. A line that is
//!   not the end always holds at least its LF, so `""` is unambiguous, and the
//!   reader needs no `fs::eof` probe (three seeks per call, and none at all on a
//!   handle that cannot seek).
//!
//! It shares `fs::readLine`'s block read buffer, so the two interleave on one
//! handle exactly.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::readLineRaw` — the shared readLine emitter in its
/// keep-terminator mode.
pub(crate) fn lower_fs_read_line_raw(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_read_write::lower_fs_read_line_helper(
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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readLineRaw",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("File"),
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "file",
                desc: "",
                aliases: &[],
                ty: ParameterType::named(super::FILE_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![
                "ErrOutOfMemory",
                "ErrReadFailed",
                "ErrEncoding",
                "ErrResourceClosed",
                "ErrResourceMoved",
            ],
            body: Body::abi_function(lower_fs_read_line_raw),
        }],
    });
}
