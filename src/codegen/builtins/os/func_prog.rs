//! `os::prog` — the invocation spelling from `argv[0]`.

use super::gen_shared::{
    build_string_from_len, push_alloc_error, void_result, OS_ARGC_GLOBAL_SYMBOL,
    OS_ARGV_GLOBAL_SYMBOL,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::util::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::codegen::string::validate::emit_call_validate_utf8;
use crate::target::shared::abi;
use crate::types::ParameterType;

pub(crate) fn lower_prog(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let empty = format!("{symbol}_empty");
    let scan_loop = format!("{symbol}_scan_loop");
    let scan_done = format!("{symbol}_scan_done");
    let encoding_error = format!("{symbol}_encoding_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let argc = vregs.next();
    let argv = vregs.next();
    let arg = vregs.next();
    let cursor = vregs.next();
    let len = vregs.next();
    let byte = vregs.next();
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    push_symbol_address(
        &symbol,
        OS_ARGC_GLOBAL_SYMBOL,
        &argc,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::load_u64(&argc, &argc, 0));
    push_symbol_address(
        &symbol,
        OS_ARGV_GLOBAL_SYMBOL,
        &argv,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(&argv, &argv, 0),
        abi::compare_immediate(&argc, "0"),
        abi::branch_eq(&empty),
        abi::load_u64(&arg, &argv, 0),
        abi::move_register(&cursor, &arg),
        abi::move_immediate(&len, "Integer", "0"),
        abi::label(&scan_loop),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&scan_done),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::add_immediate(&len, &len, 1),
        abi::branch(&scan_loop),
        abi::label(&scan_done),
        abi::move_register(abi::c_arg(0), &arg),
        abi::move_register(abi::c_arg(1), &len),
    ]);
    emit_call_validate_utf8(
        &symbol,
        &encoding_error,
        &mut instructions,
        &mut relocations,
    );
    build_string_from_len(
        &symbol,
        &arg,
        &len,
        &alloc_error,
        &format!("{symbol}_str"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&empty)]);
    raise_error_into(
        &symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&encoding_error)]);
    raise_error_into(&symbol, "ErrEncoding", &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(&symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = 0;
    Ok(void_result("os.prog"))
}

const INTRO: &str = r#"The program name from this invocation"#;
const DESC: &str = r#"`os::prog` returns the program name exactly as the host supplied it in `argv[0]`.
This is the invocation spelling, which can be relative or a symlink; it is not the
absolute resolved path returned by `os::executablePath`. If the host spelling is
not valid UTF-8, it raises `ErrEncoding`."#;
const EX: &str = r#"Print the invocation name:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::prog())
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "prog",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec!["ErrEncoding", "ErrInvalidArgument"],
            body: Body::abi_function(lower_prog),
        }],
    });
}
