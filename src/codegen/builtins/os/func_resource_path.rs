//! `os::resourcePath` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_resource_path`]). **This is the one `os`
//! member that consumes per-compilation build context**: it reads the real
//! `build_mode`/`module_name` off the [`AbiCtx`] (the strip/suffix selection baked
//! into the resource-base offset). Docs migrated from
//! `src/docs/man/builtins/os/resourcePath.md`.

use super::gen_paths::{
    emit_executable_path_into, emit_reject_dot_component, resource_base_offset,
};
use super::gen_shared::{
    alloc_reloc, emit_copy_counted, emit_store_byte_advance, push_alloc_error, void_result,
    EXE_PATH_FRAME_LOCALS,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;

/// `os::resourcePath(relative)` — the absolute on-disk path of a build resource
/// (plan-55-B §4.4). Rejects a `.`/`..` component (`ErrInvalidPath`), acquires the
/// executable path, strips `strip` trailing components and appends the mode
/// `suffix` to form the base, and concatenates `base + "/" + relative` into an owned
/// arena `String`. Acquisition failure → `ErrUnsupported` (like `os::executablePath`).
pub(crate) fn lower_resource_path(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (strip, suffix) = resource_base_offset(ctx.build_mode, ctx.module_name);
    let suffix_bytes = suffix.into_bytes();
    let fail = format!("{symbol}_fail");
    let bad_arg = format!("{symbol}_bad_arg");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    // Capture the incoming `String` argument (pointer + length) before the exe-path
    // acquisition clobbers the ARG registers. A `String` block is
    // `[8-byte length][bytes][NUL]`; its data starts at pointer + 8.
    let arg_ptr = vregs.next();
    let arg_len = vregs.next();
    let arg_data = vregs.next();
    let mut instructions = vec![
        abi::move_register(&arg_ptr, abi::c_arg(0)),
        abi::load_u64(&arg_len, &arg_ptr, 0),
        abi::add_immediate(&arg_data, &arg_ptr, 8),
    ];
    let mut relocations = Vec::new();
    // Step 1 (§4.4): reject a `.` or `..` path component.
    let scan_index = vregs.next();
    let comp_len = vregs.next();
    let comp_all_dots = vregs.next();
    let scan_byte = vregs.next();
    let validate_loop = format!("{symbol}_validate_loop");
    let validate_body = format!("{symbol}_validate_body");
    let validate_slash = format!("{symbol}_validate_slash");
    let validate_char = format!("{symbol}_validate_char");
    let validate_not_dot = format!("{symbol}_validate_not_dot");
    let validate_next = format!("{symbol}_validate_next");
    let validate_end = format!("{symbol}_validate_end");
    let check_boundary_ok = format!("{symbol}_boundary_ok");
    instructions.extend([
        abi::move_immediate(&scan_index, "Integer", "0"),
        abi::move_immediate(&comp_len, "Integer", "0"),
        abi::move_immediate(&comp_all_dots, "Integer", "1"),
        abi::label(&validate_loop),
        abi::compare_registers(&scan_index, &arg_len),
        abi::branch_ge(&validate_end),
        abi::label(&validate_body),
        abi::add_registers(&scan_byte, &arg_data, &scan_index),
        abi::load_u8(&scan_byte, &scan_byte, 0),
        abi::compare_immediate(&scan_byte, "47"), // '/'
        abi::branch_eq(&validate_slash),
        abi::branch(&validate_char),
        abi::label(&validate_slash),
    ]);
    emit_reject_dot_component(
        &comp_len,
        &comp_all_dots,
        &bad_arg,
        &check_boundary_ok,
        &mut instructions,
    );
    instructions.extend([
        abi::label(&check_boundary_ok),
        abi::move_immediate(&comp_len, "Integer", "0"),
        abi::move_immediate(&comp_all_dots, "Integer", "1"),
        abi::branch(&validate_next),
        abi::label(&validate_char),
        abi::add_immediate(&comp_len, &comp_len, 1),
        abi::compare_immediate(&scan_byte, "46"), // '.'
        abi::branch_eq(&validate_not_dot),
        abi::move_immediate(&comp_all_dots, "Integer", "0"),
        abi::label(&validate_not_dot),
        abi::branch(&validate_next),
        abi::label(&validate_next),
        abi::add_immediate(&scan_index, &scan_index, 1),
        abi::branch(&validate_loop),
        abi::label(&validate_end),
    ]);
    emit_reject_dot_component(
        &comp_len,
        &comp_all_dots,
        &bad_arg,
        &format!("{symbol}_validate_done"),
        &mut instructions,
    );
    instructions.push(abi::label(&format!("{symbol}_validate_done")));
    // Step 2 (§4.4): acquire the executable path, then compute its byte length `n`.
    let (buf, count) = emit_executable_path_into(
        &mut EmitCtx {
            symbol: symbol.as_str(),
            platform_imports: ctx.platform_imports,
            platform: ctx.platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &fail,
        &mut vregs,
    )?;
    let n = vregs.next();
    match count {
        Some(count) => instructions.push(abi::move_register(&n, &count)),
        None => {
            // macOS: NUL-terminated buffer — scan for the NUL to get the length.
            let strlen_loop = format!("{symbol}_strlen_loop");
            let strlen_done = format!("{symbol}_strlen_done");
            let strlen_byte = vregs.next();
            let strlen_ptr = vregs.next();
            instructions.extend([
                abi::move_immediate(&n, "Integer", "0"),
                abi::move_register(&strlen_ptr, &buf),
                abi::label(&strlen_loop),
                abi::load_u8(&strlen_byte, &strlen_ptr, 0),
                abi::compare_immediate(&strlen_byte, "0"),
                abi::branch_eq(&strlen_done),
                abi::add_immediate(&n, &n, 1),
                abi::add_immediate(&strlen_ptr, &strlen_ptr, 1),
                abi::branch(&strlen_loop),
                abi::label(&strlen_done),
            ]);
        }
    }
    // Step 3 (§4.4): backward scan for the `strip`-th slash from the end.
    let prefix_len = vregs.next();
    let slash_scan = vregs.next();
    let slashes_left = vregs.next();
    let slash_byte = vregs.next();
    let slash_loop = format!("{symbol}_slash_loop");
    let slash_found = format!("{symbol}_slash_found");
    let prefix_ready = format!("{symbol}_prefix_ready");
    instructions.extend([
        abi::move_register(&slash_scan, &n),
        abi::move_immediate(&slashes_left, "Integer", &strip.to_string()),
        abi::label(&slash_loop),
        abi::compare_immediate(&slash_scan, "0"),
        abi::branch_eq(&fail),
        abi::subtract_immediate(&slash_scan, &slash_scan, 1),
        abi::add_registers(&slash_byte, &buf, &slash_scan),
        abi::load_u8(&slash_byte, &slash_byte, 0),
        abi::compare_immediate(&slash_byte, "47"), // '/'
        abi::branch_eq(&slash_found),
        abi::branch(&slash_loop),
        abi::label(&slash_found),
        abi::subtract_immediate(&slashes_left, &slashes_left, 1),
        abi::compare_immediate(&slashes_left, "0"),
        abi::branch_eq(&prefix_ready),
        abi::branch(&slash_loop),
        abi::label(&prefix_ready),
        abi::move_register(&prefix_len, &slash_scan),
    ]);
    // Step 4 (§4.4): total result length.
    let extra = if suffix_bytes.is_empty() {
        1
    } else {
        suffix_bytes.len() + 2
    };
    let total_len = vregs.next();
    instructions.extend([
        abi::add_registers(&total_len, &prefix_len, &arg_len),
        abi::add_immediate(&total_len, &total_len, extra),
        abi::add_immediate(abi::return_register(), &total_len, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(&symbol, &mut relocations);
    let block = vregs.next();
    let dst = vregs.next();
    let copy_index = vregs.next();
    let copy_byte = vregs.next();
    let copy_src = vregs.next();
    let alloc_ok = format!("{symbol}_alloc_ok");
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&block, abi::mfb_return(1)),
        abi::store_u64(&total_len, &block, 0),
        abi::add_immediate(&dst, &block, 8),
    ]);
    emit_copy_counted(
        &buf,
        &prefix_len,
        &dst,
        &copy_src,
        &copy_index,
        &copy_byte,
        &format!("{symbol}_copy_prefix"),
        &mut instructions,
    );
    emit_store_byte_advance(b'/', &dst, &copy_byte, &mut instructions);
    if !suffix_bytes.is_empty() {
        for &b in &suffix_bytes {
            emit_store_byte_advance(b, &dst, &copy_byte, &mut instructions);
        }
        emit_store_byte_advance(b'/', &dst, &copy_byte, &mut instructions);
    }
    emit_copy_counted(
        &arg_data,
        &arg_len,
        &dst,
        &copy_src,
        &copy_index,
        &copy_byte,
        &format!("{symbol}_copy_arg"),
        &mut instructions,
    );
    instructions.extend([
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(RESULT_VALUE_REGISTER, &block),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    instructions.push(abi::label(&fail));
    raise_error_into(
        &symbol,
        "ErrUnsupported",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&bad_arg)]);
    raise_error_into(
        &symbol,
        "ErrInvalidPath",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(&symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = EXE_PATH_FRAME_LOCALS;
    Ok(void_result("os.resourcePath"))
}
use crate::types::ParameterType;

const INTRO: &str = r#"The absolute path of a build resource"#;
const DESC: &str = r#"`os::resourcePath` returns the **absolute** on-disk path of a resource the build
copied out of the project's manifest `resources` section, as an owned `String`.
The `relative` argument is the resource's path below its declared destination
directory (for example `music/song.ogg`), and the result is `<base>/<relative>`.

The base directory is derived at runtime from the running executable's own path
and a build-mode offset baked into the binary, so the same call resolves
correctly for every build shape:

| Build | Executable path | Resource base |
| --- | --- | --- |
| console | `…/build/<name>` | `…/build` |
| macOS `--app` | `…/Contents/MacOS/<name>` | `…/Contents/Resources` |
| Linux `--app` | `…/usr/bin/<name>` | `…/usr/share/<name>` |

The result is absolute and contains no `..` segments, so it opens with `fs::open`
regardless of the working directory — including a macOS `.app` launched from
Finder or a mounted `.AppImage`. Resolution reads only the executable's own path
(`/proc/self/exe` on Linux, `_NSGetExecutablePath` on macOS) and never consults
`$APPDIR` or any other environment variable.

A `relative` containing a `.` or `..` **path component** raises `ErrInvalidPath`
— a resource path must not navigate out of the base. A dot *inside* a filename
(`song.ogg`, `..foo`, `a..b`) is fine; only a whole component that is exactly `.`
or `..` is rejected. A leading `/` is left as-is (it collapses under the base). If
the host cannot determine the executable path, `os::resourcePath` raises
`ErrUnsupported`. It reads host state only and has no side effects."#;
const EX: &str = r#"Open a resource shipped beside the program:

```
IMPORT os
IMPORT fs
IMPORT io

SUB main()
  LET path AS String = os::resourcePath("music/song.ogg")
  io::print(path)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "resourcePath",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "relative",
                desc: "The resource path below the build output (for example `music/song.ogg`); no `.`/`..` path component.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_resource_path),
        }],
    });
}
