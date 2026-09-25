//! `app::getTitle` — read the app window's title.

// --- codegen tier imports (migration) ---
use super::gen_window::{emit_title_lock, APP_DEFAULT_TITLE_SYMBOL, APP_TITLE_SYMBOL};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `app::getTitle()` — under the title lock, pick the current title block (the
/// process-global pointer, or the default-title data when it is still `0`) and
/// copy the whole `[u64 length][bytes][NUL]` block into a fresh arena `String`.
/// The copy happens under the lock because a concurrent `setTitle` frees the
/// block it replaces. Every exit — including the out-of-memory one — runs the
/// single unlock at `done`, with the four result registers held in vregs across
/// the unlock call.
pub(crate) fn lower_get_title(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let have_block = format!("{symbol}_have_block");
    let oom = format!("{symbol}_oom");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let addr = vregs.next();
    let current = vregs.next();
    let total = vregs.next();
    let block = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    emit_title_lock(
        &mut EmitCtx {
            symbol: symbol.as_str(),
            platform_imports: ctx.platform_imports,
            platform: ctx.platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        true,
    )?;
    push_symbol_address(
        &symbol,
        APP_TITLE_SYMBOL,
        &addr,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(&current, &addr, 0),
        abi::compare_immediate(&current, "0"),
        abi::branch_ne(&have_block),
    ]);
    push_symbol_address(
        &symbol,
        APP_DEFAULT_TITLE_SYMBOL,
        &current,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&have_block),
        abi::load_u64(&total, &current, 0),
        abi::add_immediate(&total, &total, 9),
        abi::move_register(abi::return_register(), &total),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(&symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&oom),
        abi::move_register(&block, abi::mfb_return(1)),
        abi::move_register(&src, &current),
        abi::move_register(&dst, &block),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &total),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::move_register(RESULT_VALUE_REGISTER, &block),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&oom),
    ]);
    raise_error_into(
        &symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    let saved_tag = vregs.next();
    let saved_value = vregs.next();
    let saved_message = vregs.next();
    let saved_source = vregs.next();
    instructions.extend([
        abi::label(&done),
        abi::move_register(&saved_tag, RESULT_TAG_REGISTER),
        abi::move_register(&saved_value, RESULT_VALUE_REGISTER),
        abi::move_register(&saved_message, RESULT_ERROR_MESSAGE_REGISTER),
        abi::move_register(&saved_source, RESULT_ERROR_SOURCE_REGISTER),
    ]);
    emit_title_lock(
        &mut EmitCtx {
            symbol: symbol.as_str(),
            platform_imports: ctx.platform_imports,
            platform: ctx.platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        false,
    )?;
    instructions.extend([
        abi::move_register(RESULT_TAG_REGISTER, &saved_tag),
        abi::move_register(RESULT_VALUE_REGISTER, &saved_value),
        abi::move_register(RESULT_ERROR_MESSAGE_REGISTER, &saved_message),
        abi::move_register(RESULT_ERROR_SOURCE_REGISTER, &saved_source),
        abi::return_(),
    ]);
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "app.getTitle".to_string(),
    })
}

const INTRO: &str = r#"Read the text in the app window's title bar"#;
const DESC: &str = r#"`app::getTitle` returns the title of the program's window as a new `String`. It
takes no arguments.

Before the program calls `app::setTitle`, this is the title the window was built
with: the project name on Linux and Windows, `MFBASIC App` on macOS. After it, it
is exactly the last `title` passed to `app::setTitle` — from any thread of the
program — even while there is no window in `app::Mode.None`.

Raises `ErrOutOfMemory` if there is no memory for the returned `String`."#;
const EX: &str = r#"Add a marker to the current title:

```
IMPORT app

SUB main
  app::setTitle(app::getTitle() & " (unsaved)")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getTitle",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_get_title),
        }],
    });
}
