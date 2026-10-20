//! Native code generation for user `LINK` bindings (plan-linker.md §12).
//!
//! Each `LINK "lib" AS alias` block contributes:
//!
//! - a per-program load-time initializer (`_mfb_linker_init`) that `dlopen`s each
//!   distinct library and `dlsym`s every declared symbol into a per-function
//!   global pointer slot (§12.1), aborting before `main` on any failure;
//! - one MFB↔C marshaling thunk per `FUNC` (`_mfb_linker_<alias>_<name>`, §12.2)
//!   that marshals arguments per §12.3, calls through the resolved pointer, then
//!   marshals the native return and any `OUT` slots back, applying `SUCCESS_ON`
//!   and `RESULT`.
//!
//! The resolved function pointers live in the program's writable global region
//! (addressed `x19 + ENTRY_GLOBALS_OFFSET + slot*8`), reserved immediately after
//! the program's own globals. Read-only C strings (library filenames and symbol
//! names) live in the constant data section.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::string::validate::*;
use crate::ir::{IrLinkExpr, IrLinkFunction};
use crate::target::shared::abi;
use crate::target::shared::nir::{self, link_thunk_symbol};
use crate::types::CAbiType;
use std::collections::HashMap;
use std::collections::HashSet;

use crate::codegen::collection::layout::emit_alloc_byte_list;
use crate::codegen::link::locator::LinkLibraries;
use crate::operators::BinaryOp;

mod link_expr;
mod lower_thunk;
mod struct_marshal;
mod value_marshal;
use link_expr::*;
use lower_thunk::*;
use struct_marshal::*;
use value_marshal::*;
/// sec-02: bytes of guard region appended past an `OUT CBuffer`'s declared
/// `SIZE`, stamped with a canary before the native call and checked after. A
/// callee that writes past `SIZE` clobbers the canary (contiguous writers like
/// `read`/`pread`/`recv` hit it first), converting a silent arena overrun into a
/// deterministic `ErrNativeBufferOverrun` trap. Two 8-byte words = 16 bytes.
const BUFFER_GUARD_BYTES: usize = 16;
/// The two canary words (distinctive, non-trivial) — the odds a legitimate callee
/// writes both exactly into the guard by coincidence are ~2^-128. Kept below
/// `i64::MAX` so every backend's immediate parser accepts them (link_thunk is
/// shared codegen). Only that store and check use the SAME constant matters.
const BUFFER_CANARY_0: &str = "6510615555426900570"; // 0x5A5A5A5A5A5A5A5A
const BUFFER_CANARY_1: &str = "3038287259199220266"; // 0x2A2A2A2A2A2A2A2A

/// The generated functions and data objects backing the program's `LINK`
/// bindings.
pub(crate) struct LinkSupport {
    pub(crate) functions: Vec<CodeFunction>,
    pub(crate) data_objects: Vec<CodeDataObject>,
}

/// A read-only NUL-terminated C string constant.
fn cstring_object(symbol: &str, text: &str) -> CodeDataObject {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    let value = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: "c-string { u8 bytes[]; u8 nul }".to_string(),
        align: 1,
        size: bytes.len(),
        value,
    }
}

fn lib_symbol(index: usize) -> String {
    format!("_mfb_linker_lib_{index}")
}

fn sym_symbol(index: usize) -> String {
    format!("_mfb_linker_sym_{index}")
}

/// Read-only constant naming the `k`-th `FREE` deallocator symbol (e.g.
/// `sqlite3_free`), resolved by `dlsym` into a slot past the per-function slots.
fn free_sym_symbol(k: usize) -> String {
    format!("_mfb_linker_free_{k}")
}

/// The writable global slot (relative to `x19`) holding the resolved pointer for
/// the `index`-th `LINK` function. Reserved after the program's `globals_base`
/// own global slots.
fn slot_offset(globals_base: usize, index: usize) -> usize {
    ENTRY_GLOBALS_OFFSET + (globals_base + index) * 8
}

/// Emit `adrp`/`add` to materialize the address of data `symbol` into `dst`.
pub(crate) fn emit_data_address(
    from: &str,
    dst: impl Into<abi::Reg>,
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let dst: abi::Reg = dst.into();
    instructions.push(abi::load_page_address(&dst, symbol));
    instructions.push(abi::add_page_offset(&dst, &dst, symbol));
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
}

/// POSIX `handle = dlopen(filename, RTLD_NOW)` — the default `emit_lib_open`.
/// `filename_symbol` names the read-only C string holding the `dlopen` filename;
/// the resolved handle lands in `return_register()` (0 on failure). The
/// vendored-vs-system distinction is not represented here: on ELF/Mach-O the
/// image's rpath (`$ORIGIN/vendor`, `@loader_path/vendor`) already points the
/// loader at the vendored copy, so a plain `dlopen(name, …)` finds it. Windows
/// has no rpath and overrides this (bug-431).
pub(crate) fn emit_posix_dlopen<P: CodegenPlatform + ?Sized>(
    platform: &P,
    filename_symbol: &str,
    from: &str,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    emit_data_address(
        from,
        abi::return_register(),
        filename_symbol,
        instructions,
        relocations,
    );
    instructions.push(abi::move_immediate(abi::c_arg(1), "Integer", "2")); // RTLD_NOW
    platform.emit_external_call("dlopen", from, platform_imports, instructions, relocations)
}

/// POSIX `slot = dlsym(handle, symbolName)` — the default `emit_lib_get_sym`. The
/// library handle is read from `handle_reg`; the resolved address lands in
/// `return_register()` (0 if absent).
pub(crate) fn emit_posix_dlsym<P: CodegenPlatform + ?Sized>(
    platform: &P,
    handle_reg: impl Into<abi::Reg>,
    symbol_symbol: &str,
    from: &str,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    instructions.push(abi::move_register(abi::return_register(), handle_reg));
    emit_data_address(
        from,
        abi::c_arg(1),
        symbol_symbol,
        instructions,
        relocations,
    );
    platform.emit_external_call("dlsym", from, platform_imports, instructions, relocations)
}

/// bug-431: the Windows `_mfb_linker_init` builds each vendored DLL's absolute
/// path (`<exe_dir>\vendor\<dlopen_name>`) in this writable scratch buffer before
/// `LoadLibraryExA`. Single writer, single reader, and the initializer runs once
/// at startup before any thread exists, so one shared buffer is safe. Sized for a
/// full path plus the `\vendor\` join and the longest bare DLL name.
pub(crate) const WIN_LINK_PATHBUF_SYMBOL: &str = "_mfb_linker_win_pathbuf";
pub(crate) const WIN_LINK_PATHBUF_BYTES: usize = 1024;
/// bug-431: the `\vendor\` path component appended between the exe directory and
/// the DLL name (a read-only NUL-terminated C string for `lstrcatA`).
pub(crate) const WIN_LINK_VENDORSEP_SYMBOL: &str = "_mfb_linker_win_vendorsep";
pub(crate) const WIN_LINK_VENDORSEP_TEXT: &str = "\\vendor\\";

/// Emit `bl _mfb_arena_alloc` (size in `x0`, align in `x1`); on success the block
/// pointer is in `x1`. Branches to `fail` on allocation failure.

/// Build the full `LINK` support: the load-time initializer, one thunk per
/// function, and the backing data objects.
/// The build-level scalars every LINK thunk in a module shares.
///
/// Two numbers of the same shape passed adjacently is the setup for a silent
/// transposition, and `max_buffer_bytes` (plan-58-C) is what made it two.
#[derive(Clone, Copy)]
pub(crate) struct LinkCodegenOptions {
    pub(crate) globals_base: usize,
    /// project.json `options.maxBuffer` in bytes — the `OUT CBuffer` allocation ceiling.
    /// The CONSUMING project's setting: thunks are emitted when an executable
    /// links, so a binding cannot raise an application's ceiling on its behalf.
    pub(crate) max_buffer_bytes: u64,
}

/// The per-thunk scalars `lower_link_thunk` needs that are not the function
/// itself: where it sits in the link table, where the globals region starts, its
/// `FREE` slot, and the project's buffer ceiling.
///
/// Bundled rather than passed positionally because they are four unrelated
/// numbers of the same shape — `lower_link_thunk(f, cs, rf, 0, 0, None, ...)` is
/// exactly the call that transposes two arguments silently. Adding
/// `max_buffer_bytes` (plan-58-C) is what pushed the arity past the point where
/// that stopped being hypothetical.
#[derive(Clone, Copy)]
struct ThunkContext {
    /// Index in the link table, used for the per-function symbol slot.
    index: usize,
    globals_base: usize,
    /// The globals slot holding the `FREE` deallocator's resolved address.
    free_slot: Option<usize>,
    /// project.json `options.maxBuffer` in bytes — the `OUT CBuffer` allocation ceiling.
    /// The CONSUMING project's setting; see `emit_link_support`.
    max_buffer_bytes: u64,
}

pub(crate) fn emit_link_support(
    link_functions: &[IrLinkFunction],
    link_cstructs: &[crate::ir::IrCStruct],
    record_fields: &HashMap<
        crate::types::ParameterType,
        Vec<(String, crate::types::ParameterType)>,
    >,
    options: LinkCodegenOptions,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    libraries: &LinkLibraries,
    close_op_symbols: &HashSet<String>,
) -> Result<LinkSupport, String> {
    let LinkCodegenOptions {
        globals_base,
        max_buffer_bytes,
    } = options;
    let mut data_objects = Vec::new();

    // Distinct libraries in declaration order, each mapped to a constant symbol.
    let mut library_index: Vec<String> = Vec::new();
    for function in link_functions {
        if !library_index.iter().any(|lib| lib == &function.library) {
            library_index.push(function.library.clone());
        }
    }
    // plan-46-C: the `dlopen` filename is the binding author's declared `source`
    // for this build's exact (os, arch, libc) — resolved from the imported
    // binding's section-10 table, never synthesized. Synthesizing a soname from
    // the logical name (`lib{logical}.so.0` / `lib{logical}.dylib`) does not
    // consult the manifest and misses every unversioned `.so`, `.so.3`,
    // non-`lib`-prefixed, or per-arch/libc variant — do not reintroduce it.
    // bug-431: per-library, whether the resolved locator is a *vendored* copy
    // (as opposed to a `system` soname the loader finds by name). On Windows the
    // vendored ones are loaded from the exe-relative `vendor/` directory; POSIX
    // ignores the flag (its rpath already covers vendoring).
    let mut library_vendored: Vec<bool> = Vec::with_capacity(library_index.len());
    for (index, library) in library_index.iter().enumerate() {
        let resolved = libraries.get(library)?;
        data_objects.push(cstring_object(&lib_symbol(index), &resolved.dlopen_name));
        library_vendored
            .push(resolved.locator.lib_type == crate::manifest::libraries::LibType::Vendor);
    }
    // bug-431: the Windows loader builds each vendored DLL's absolute path in a
    // writable scratch buffer and appends `\vendor\` via `lstrcatA`; emit those
    // two data objects when a Windows build actually vendors something. On POSIX,
    // or a Windows build whose libraries are all `system`, neither is emitted, so
    // the binary is byte-identical to today.
    if platform.family() == crate::codegen::engine::types::PlatformFamily::Windows
        && library_vendored.iter().any(|&v| v)
    {
        data_objects.push(CodeDataObject {
            symbol: WIN_LINK_PATHBUF_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "c-string { u8 bytes[] }".to_string(),
            align: 8,
            size: WIN_LINK_PATHBUF_BYTES,
            value: "00".repeat(WIN_LINK_PATHBUF_BYTES),
        });
        data_objects.push(cstring_object(
            WIN_LINK_VENDORSEP_SYMBOL,
            WIN_LINK_VENDORSEP_TEXT,
        ));
    }
    // One symbol-name constant per function (indexed by position).
    for (index, function) in link_functions.iter().enumerate() {
        data_objects.push(cstring_object(&sym_symbol(index), &function.symbol));
    }

    // Each `FREE` block resolves its deallocator into a slot reserved past the
    // per-function slots. `free_index_of[i]` is the function's deallocator index
    // `k` (so its slot is `link_count + k`), or `None` when it has no FREE.
    let link_count = link_functions.len();
    let mut free_index_of: Vec<Option<usize>> = Vec::with_capacity(link_count);
    let mut free_count = 0usize;
    for function in link_functions {
        if let Some(free) = &function.free {
            data_objects.push(cstring_object(&free_sym_symbol(free_count), &free.symbol));
            free_index_of.push(Some(free_count));
            free_count += 1;
        } else {
            free_index_of.push(None);
        }
    }

    // plan-59-A: the resource TYPES that are represented as resource records —
    // which is now EVERY type a native func returns as `AS RES R`, with or
    // without a `STATE S`. (plan-53-A wrapped only the stateful ones, so a
    // stateless `Db` was the raw handle and had nowhere to put a `closed` flag.)
    //
    // Record-ness is per-TYPE, not per-declaration: a BARE `db AS R` param
    // still receives a record pointer and must load the handle from FD@0 before
    // the native call (e.g. `close`/`exec`). This set is what lets a thunk tell a
    // record-resource param from a scalar one, and it must stay in lockstep with
    // the return-side wrap below — widening one without the other hands `FD@0` a
    // raw handle to dereference.
    let record_native_resources: HashSet<crate::types::ParameterType> = link_functions
        .iter()
        .filter(|f| f.return_resource)
        .map(|f| f.return_type.without_state())
        .collect();

    // plan-59-B: the guard's two error strings, emitted HERE rather than relying
    // on the standard-error-message block in `mod.rs`.
    //
    // That block is gated on the program using a `_mfb_rt_fs_` or
    // `_mfb_rt_thread_` runtime symbol (`mod.rs:641-644`). A pure-`LINK` program
    // uses neither — `native-resource-import-valid` is exactly that shape — so
    // the guard's relocation resolved to nothing and the build failed with
    // "data relocation target '_mfb_str_error_resource_closed' is not a data
    // object". Emitting them alongside the LINK support's own constants keeps the
    // guard self-contained: whatever else the program does or does not import,
    // a thunk that can emit the guard also carries the strings it names.
    if link_functions.iter().any(|f| {
        f.params
            .iter()
            .any(|(_, type_)| record_native_resources.contains(&type_.without_state()))
    }) {
        for (_, message, symbol) in ["ErrResourceClosed", "ErrResourceMoved"].map(|name| {
            crate::codegen::registry::runtime_error_triple(name).expect("errorCode name")
        }) {
            if !data_objects.iter().any(|object| object.symbol == symbol) {
                data_objects.push(CodeDataObject {
                    symbol: symbol.to_string(),
                    kind: "constant".to_string(),
                    layout: "mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }"
                        .to_string(),
                    align: 8,
                    size: align(8 + message.len() + 1, 8),
                    value: message.to_string(),
                });
            }
        }
    }

    // plan-59-B: the `LINK` functions that are some resource's registered
    // `CLOSE BY` op, by dotted `alias.func` — the set that must set
    // `RESOURCE_CLOSED_BIT` on the record it was handed.
    //
    // bug-377: matched by SYMBOL, and computed by the same `resolve_closer_symbol`
    // the scope-drop call site uses, NOT from `native_resources`.
    //
    // `native_resources` holds the current project's `RESOURCE T CLOSE BY`
    // declarations only — a decoded package contributes none (`ir/binary/`
    // drops them by contract) — so an imported package's close thunk was left out
    // of this set and never set the closed bit. An explicit `sqlite3::close(db)`
    // then released the handle while leaving the record looking open, and the
    // scope-exit drop closed it a second time for real: a double free into the
    // native library (`EXC_BAD_ACCESS` inside `libsqlite3`).
    //
    // Matching on the resolved symbol rather than on a name is what keeps the two
    // sides honest. A close op reaches this module under either of two spellings
    // (see `resolve_closer_symbol`), so a name comparison marked one shape and
    // missed the other — and the shape it missed is precisely the one the drop
    // path was closing.

    let initializer = lower_link_initializer(
        link_functions,
        &library_index,
        globals_base,
        link_count,
        &free_index_of,
        &library_vendored,
        platform_imports,
        platform,
    )?;
    let mut functions = vec![initializer];
    for (index, function) in link_functions.iter().enumerate() {
        let free_slot = free_index_of[index].map(|k| link_count + k);
        functions.push(lower_link_thunk(
            function,
            link_cstructs,
            record_fields,
            ThunkContext {
                index,
                globals_base,
                free_slot,
                max_buffer_bytes,
            },
            &record_native_resources,
            close_op_symbols.contains(&crate::target::shared::nir::link_thunk_symbol(
                &function.alias,
                &function.name,
            )),
        )?);
    }

    Ok(LinkSupport {
        functions,
        data_objects,
    })
}

/// Emit `_mfb_linker_init`: `dlopen` each library, `dlsym` each symbol into its
/// global slot. Returns the standard `(tag, value, message)` result so the
/// program entry handles a load failure exactly like a global-initializer error.
fn lower_link_initializer(
    link_functions: &[IrLinkFunction],
    library_index: &[String],
    globals_base: usize,
    link_count: usize,
    free_index_of: &[Option<usize>],
    library_vendored: &[bool],
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    // Vreg-allocated (plan-00-G Phase 2). The only value held across the
    // `dlopen`/`dlsym` libc calls is the library `handle`; as a vreg the allocator
    // keeps it in a callee-saved register across the calls (the calls are libc =
    // PCS, so callee-saved survives) instead of the old manual stack slot. `x19`
    // (arena_base, where the resolved slots land) and the libc ABI registers
    // (x0/x1) stay physical.
    let symbol = nir::LINK_INIT_SYMBOL;
    let fail = format!("{symbol}_fail");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let handle = vregs.next();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    for (lib_idx, library) in library_index.iter().enumerate() {
        // handle = dlopen(filename, RTLD_NOW) on POSIX; on Windows this is a
        // `LoadLibraryExA` resolving the vendored DLL from the exe-relative
        // `vendor/` directory (bug-431). Either way the module handle lands in
        // `return_register()`, so the failure check and store below are shared.
        platform.emit_lib_open(
            &lib_symbol(lib_idx),
            library_vendored[lib_idx],
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&fail),
            abi::move_register(&handle, abi::return_register()),
        ]);
        for (fn_idx, function) in link_functions.iter().enumerate() {
            if &function.library != library {
                continue;
            }
            // slot = dlsym(handle, symbolName) on POSIX; `GetProcAddress` on
            // Windows. The resolved address lands in `return_register()`.
            platform.emit_lib_get_sym(
                handle.into(),
                &sym_symbol(fn_idx),
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::compare_immediate(abi::return_register(), "0"),
                abi::branch_eq(&fail),
                abi::store_u64(
                    abi::return_register(),
                    ARENA_STATE_REGISTER,
                    slot_offset(globals_base, fn_idx),
                ),
            ]);
            // A FREE deallocator lives in the same library; resolve it into its
            // own slot (reserved past the per-function slots).
            if let Some(k) = free_index_of[fn_idx] {
                platform.emit_lib_get_sym(
                    handle.into(),
                    &free_sym_symbol(k),
                    symbol,
                    platform_imports,
                    &mut instructions,
                    &mut relocations,
                )?;
                instructions.extend([
                    abi::compare_immediate(abi::return_register(), "0"),
                    abi::branch_eq(&fail),
                    abi::store_u64(
                        abi::return_register(),
                        ARENA_STATE_REGISTER,
                        slot_offset(globals_base, link_count + k),
                    ),
                ]);
            }
        }
    }

    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&fail),
    ]);
    raise_error_into(
        symbol,
        "ErrNativeBindingUnavailable",
        &mut instructions,
        &mut relocations,
    );
    // Same no-origin sentinel as the per-function thunks (bug-371): the loader
    // has no MFBASIC source location, and x3 is otherwise whatever `dlopen`/
    // `dlsym` left behind.
    instructions.extend([
        abi::label(&done),
        abi::move_immediate(RESULT_ERROR_SOURCE_REGISTER, "Integer", "0"),
        abi::return_(),
    ]);

    Ok(finalize_vreg_helper(
        "linker.init",
        symbol,
        "Nothing",
        instructions,
        relocations,
    ))
}

/// Frame slots and failure labels the return marshaler needs.
struct ReturnMarshal<'a> {
    cret_off: usize,
    cretd_off: usize,
    status_off: usize,
    alloc_fail: &'a str,
    encoding_fail: &'a str,
    nan_fail: &'a str,
    inf_fail: &'a str,
}

struct ThunkScratch {
    t0: VirtualRegister,
    t1: VirtualRegister,
    t2: VirtualRegister,
    t3: VirtualRegister,
    t4: VirtualRegister,
    t5: VirtualRegister,
    t6: VirtualRegister,
    /// The resolved native function pointer a `blr` calls through.
    callee: VirtualRegister,
}

impl ThunkScratch {
    fn mint(vregs: &mut Vregs) -> Self {
        ThunkScratch {
            t0: vregs.next(),
            t1: vregs.next(),
            t2: vregs.next(),
            t3: vregs.next(),
            t4: vregs.next(),
            t5: vregs.next(),
            t6: vregs.next(),
            callee: vregs.next(),
        }
    }
}

#[cfg(test)]
mod tests;
