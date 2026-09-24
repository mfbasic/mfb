//! `app::setTitle` — change the app window's title.

// --- codegen tier imports (migration) ---
use super::gen_window::{emit_title_lock, APP_TITLE_SYMBOL};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `app::setTitle(title)` — copy the argument `String` block
/// (`[u64 length][bytes][NUL]`, `length + 9` bytes) into a fresh process-heap
/// block, swap it into the process-global title pointer under the title lock,
/// free the block it replaced (after the unlock — a reader copies under the lock,
/// so once the swap is visible nobody can still be reading the old block), then
/// run the backend window sync ([`CodegenPlatform::emit_app_window_sync`]).
///
/// The copy is made because the argument lives in the caller's arena, which is
/// per-thread and reset under the title: the UI thread and other program threads
/// read the title long after this call returns.
pub(crate) fn lower_set_title(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let oom = format!("{symbol}_oom");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let no_old = format!("{symbol}_no_old");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let title = vregs.next();
    let total = vregs.next();
    let block = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let addr = vregs.next();
    let old = vregs.next();
    let mut instructions = vec![
        abi::move_register(&title, abi::c_arg(0)),
        // header + bytes + NUL
        abi::load_u64(&total, &title, 0),
        abi::add_immediate(&total, &total, 9),
        abi::move_register(abi::c_arg(0), &total),
    ];
    let mut relocations = Vec::new();
    ctx.platform.emit_heap_alloc(
        &symbol,
        ctx.platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&block, abi::return_register()),
        abi::compare_immediate(&block, "0"),
        abi::branch_eq(&oom),
        abi::move_register(&src, &title),
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
    ]);
    let mut emit = EmitCtx {
        symbol: symbol.as_str(),
        platform_imports: ctx.platform_imports,
        platform: ctx.platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    };
    emit_title_lock(&mut emit, true)?;
    push_symbol_address(
        &symbol,
        APP_TITLE_SYMBOL,
        &addr,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(&old, &addr, 0),
        abi::store_u64(&block, &addr, 0),
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
    // `0` is the never-set default, which lives in read-only data, not the heap.
    instructions.extend([
        abi::compare_immediate(&old, "0"),
        abi::branch_eq(&no_old),
        abi::move_register(abi::c_arg(0), &old),
    ]);
    ctx.platform.emit_heap_free(
        &symbol,
        ctx.platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&no_old));
    if let Some(result) =
        ctx.platform
            .emit_app_window_sync(
        &symbol,
        ctx.platform_imports, &mut instructions, &mut relocations)
    {
        result?;
    }
    instructions.extend([
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
    instructions.extend([abi::label(&done), abi::return_()]);
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "app.setTitle".to_string(),
    })
}

const INTRO: &str = r#"Change the text in the app window's title bar"#;
const DESC: &str = r#"`app::setTitle` sets the text shown in the title bar of the program's window.
It returns nothing. A later `app::getTitle` returns `title` exactly.

The window starts with a title chosen when the program is built — the project
name on Linux and Windows, `MFBASIC App` on macOS. `app::setTitle` replaces it for
the rest of the run.

In `app::Mode.None` there is no window to show the title. The title is still
remembered, and the window shows it as soon as a later `app::setMode` brings the
window up. Switching between `app::Mode.Console` and `app::Mode.Canvas` keeps it.

The window shows the text only up to its first NUL character, if it has one; an
empty `title` gives a window with a blank title bar. `app::getTitle` still returns
the whole `title`.

A program may call `app::setTitle` as often as it likes — once a frame to show a
score or a frame rate is fine.

Raises `ErrOutOfMemory` if there is no memory for a copy of `title`; the window
keeps its previous title."#;
const EX: &str = r#"Show progress in the title bar:

```
IMPORT app

SUB main
  app::setMode(app::Mode.Console)
  FOR i = 1 TO 3
    app::setTitle("Working: step " & toString(i) & " of 3")
  NEXT
  app::setTitle("Done")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setTitle",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "title",
                desc: "The new title-bar text. Any `String`, including an empty one.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_set_title),
        }],
    });
}
