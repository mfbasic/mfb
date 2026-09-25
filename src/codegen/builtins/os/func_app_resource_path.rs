//! `os::appResourcePath` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_app_resource_path`]). **This is the one `os`
//! member that consumes per-compilation build context**: it reads the real
//! `build_mode`/`module_name` off the [`AbiCtx`] (the strip/suffix selection baked
//! into the resource-base offset). Docs migrated from
//! `src/docs/man/builtins/os/appResourcePath.md`.

use super::gen_host_paths::{
    emit_capture_relative, emit_join_result, emit_path_error_tails, emit_validate_relative,
};
use super::gen_paths::{emit_executable_path_into, resource_base_offset};
use super::gen_shared::{void_result, EXE_PATH_FRAME_LOCALS};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::PlatformFamily;
use crate::codegen::engine::util::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;

/// `os::appResourcePath(relative)` — the absolute on-disk path of a build resource
/// (plan-55-B §4.4). Rejects a `.`/`..` component (`ErrInvalidPath`), acquires the
/// executable path, strips `strip` trailing components and appends the mode
/// `suffix` to form the base, and concatenates `base + "/" + relative` into an owned
/// arena `String`. Acquisition failure → `ErrUnsupported` (like `os::executablePath`).
pub(crate) fn lower_app_resource_path(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    // bug-454. Two of this body's three separator decisions are platform-
    // dependent, and they are dependent for DIFFERENT reasons:
    //
    // * The `.`/`..` component validation walks the caller's `relative`
    //   argument. On Windows `\` is a directory separator to every Win32 path
    //   API, so `..\secret` navigates out of the base exactly as `../secret`
    //   does — the thing this check exists to refuse. Splitting on `/` alone
    //   would wave it through as one component that is not all-dots. Accepting
    //   BOTH bytes there rejects strictly more traversal and nothing valid: a
    //   Windows filename cannot contain `\`, and only a whole `.`/`..`
    //   component is refused either way.
    // * The backward scan walks the OS-produced executable path.
    //   `GetModuleFileNameW` returns `C:\dir\app.exe` — no `/` at all — so
    //   scanning for `/` runs the cursor to 0 and raises `ErrUnsupported`
    //   however good the acquisition is. That site takes the platform byte
    //   ALONE, the same call `fs::isWithin`'s `within_sep` makes for the
    //   `realpath`-produced bytes it compares.
    //
    // The third decision, the byte this body JOINS with, is deliberately NOT
    // platform-dependent: `/` on every target, matching `fs::pathJoin`'s
    // `SEP = 47`, which is `/` on Windows too. Portable MFB path strings are
    // `/`-delimited everywhere; only OS-produced bytes carry the native
    // separator. Win32 accepts `/` in every path it parses.
    let windows = ctx.platform.family() == PlatformFamily::Windows;
    let (strip, suffix) = resource_base_offset(ctx.build_mode, ctx.module_name);
    let suffix_bytes = suffix.into_bytes();
    let fail = format!("{symbol}_fail");
    let bad_arg = format!("{symbol}_bad_arg");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    // Capture the incoming `String` argument (pointer + length) before the exe-path
    // acquisition clobbers the ARG registers.
    let mut instructions = Vec::new();
    let arg = emit_capture_relative(&mut vregs, &mut instructions);
    let mut relocations = Vec::new();
    // Step 1 (§4.4): reject a `.` or `..` path component.
    emit_validate_relative(&symbol, &arg, windows, &bad_arg, &mut vregs, &mut instructions);
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
        // The platform separator of the OS-produced executable path: `/` (47) on
        // POSIX, `\` (92) on Windows (`GetModuleFileNameW` normalizes to
        // backslash), per the note at the head of this function.
        abi::compare_immediate(&slash_byte, if windows { "92" } else { "47" }),
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
    // Step 4 (§4.4): `prefix ["/" suffix] ["/" relative]`. The mode suffix is part
    // of the base; the joining `/` exists only for a non-empty `relative`, so an
    // empty one yields the bare base with no trailing `/` (plan-156-A §4.2).
    let mut suffix_bytes_with_slash = Vec::new();
    if !suffix_bytes.is_empty() {
        suffix_bytes_with_slash.push(b'/');
        suffix_bytes_with_slash.extend_from_slice(&suffix_bytes);
    }
    emit_join_result(
        &symbol,
        &buf,
        &prefix_len,
        &suffix_bytes_with_slash,
        &arg,
        &alloc_error,
        &done,
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    emit_path_error_tails(
        &symbol,
        &fail,
        &bad_arg,
        &alloc_error,
        &done,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = EXE_PATH_FRAME_LOCALS;
    Ok(void_result("os.appResourcePath"))
}
use crate::types::ParameterType;

const INTRO: &str = r#"The absolute path of a build resource"#;
const DESC: &str = r#"`os::appResourcePath` returns the **absolute** on-disk path of a resource the build
copied out of the project's manifest `resources` section, as a `String`.
The `relative` argument is the resource's path below its declared destination
directory (for example `music/song.ogg`), and the result is `<base>/<relative>`.
Omit `relative` (or pass `""`) to get the resource directory itself: the result
is then `<base>`, with no trailing `/`.

The base directory is derived at runtime from the running executable's own path
and a build-mode offset baked into the binary, so the same call resolves
correctly for every build shape:

| Build | Executable path | Resource base |
| --- | --- | --- |
| console | `…/build/<name>` | `…/build` |
| macOS `--app` | `…/Contents/MacOS/<name>` | `…/Contents/Resources` |
| Linux `--app` | `…/usr/bin/<name>` | `…/usr/share/<name>` |
| Windows `--app` | `…\build\<name>.exe` | `…\build` |

Every native target resolves the call, `windows-x86_64` included. Resolution
reads only the executable's own path — `/proc/self/exe` on Linux,
`_NSGetExecutablePath` on macOS, `GetModuleFileNameW` on Windows — and never
consults `$APPDIR` or any other environment variable.

The result is absolute and contains no `..` segments, so it opens with `fs::open`
regardless of the working directory — including a macOS `.app` launched from
Finder or a mounted `.AppImage`.

The base carries the separator the host produced (`/` on macOS and Linux, `\` on
Windows), and the base and `relative` are joined with `/` on every target — the
same byte `fs::pathJoin` uses. So a Windows result reads
`C:\proj\build/song.ogg`, which every Win32 path API accepts, and
`strings::endsWith(path, "/song.ogg")` is true on every target.

A `relative` containing a `.` or `..` **path component** raises `ErrInvalidPath`
— a resource path must not navigate out of the base. A dot *inside* a filename
(`song.ogg`, `..foo`, `a..b`) is fine; only a whole component that is exactly `.`
or `..` is rejected. A component ends at `/` on every target, and on Windows also
at `\`, which separates directories there too — so `..\secret` is refused as
well. A leading `/` is left as-is (it collapses under the base). If the host
cannot determine the executable path, `os::appResourcePath` raises
`ErrUnsupported`. It reads host state only and has no side effects."#;
const EX: &str = r#"Open a resource shipped beside the program:

```
IMPORT os
IMPORT fs
IMPORT io

SUB main()
  LET path AS String = os::appResourcePath("music/song.ogg")
  io::print(path)
END SUB
```

List everything the build shipped, starting from the resource directory:

```
IMPORT os
IMPORT fs
IMPORT io

SUB main()
  FOR EACH entry IN fs::listDirectory(os::appResourcePath())
    io::print(entry)
  NEXT
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "appResourcePath",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "relative",
                desc: "The resource path below the build output (for example `music/song.ogg`); no `.`/`..` path component. Omit it (or pass `\"\"`) for the resource directory itself.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::Fill {
                    type_name: ParameterType::String,
                    expr: "",
                },
            }],
            return_type: ParameterType::String,
            // bug-454: both are raised by `lower_app_resource_path` (through
            // `raise_error_into`) and were undeclared, so the rendered page had no
            // Errors section at all. `raise_error_into` runs no declaration check,
            // and the static `every_raise_error_site_is_declared_in_its_descriptor`
            // scan only reads two-string-literal call sites of the CodeBuilder
            // method — so nothing caught it. (That scan is textual: do not spell
            // its anchor followed by two quoted strings in prose, or it flags the
            // comment.)
            errors: vec!["ErrUnsupported", "ErrInvalidPath"],
            body: Body::abi_function(lower_app_resource_path),
        }],
    });
}
