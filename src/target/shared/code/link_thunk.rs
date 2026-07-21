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

use std::collections::HashMap;

use super::builder_collection_layout::emit_alloc_byte_list;
use super::link_locator::LinkLibraries;
use super::*;
use crate::ir::{IrLinkExpr, IrLinkFunction};
use crate::target::shared::abi;
use crate::target::shared::nir::{self, link_thunk_symbol};

/// The generated functions and data objects backing the program's `LINK`
/// bindings.
pub(super) struct LinkSupport {
    pub(super) functions: Vec<CodeFunction>,
    pub(super) data_objects: Vec<CodeDataObject>,
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
fn emit_data_address(
    from: &str,
    dst: &str,
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::load_page_address(dst, symbol));
    instructions.push(abi::add_page_offset(dst, dst, symbol));
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

/// Emit `bl _mfb_arena_alloc` (size in `x0`, align in `x1`); on success the block
/// pointer is in `x1`. Branches to `fail` on allocation failure.

/// Build the full `LINK` support: the load-time initializer, one thunk per
/// function, and the backing data objects.
/// The build-level scalars every LINK thunk in a module shares.
///
/// Two numbers of the same shape passed adjacently is the setup for a silent
/// transposition, and `max_buffer_bytes` (plan-58-C) is what made it two.
#[derive(Clone, Copy)]
pub(super) struct LinkCodegenOptions {
    pub(super) globals_base: usize,
    /// project.json `maxBuffer` in bytes — the `OUT CBuffer` allocation ceiling.
    /// The CONSUMING project's setting: thunks are emitted when an executable
    /// links, so a binding cannot raise an application's ceiling on its behalf.
    pub(super) max_buffer_bytes: u64,
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
    /// project.json `maxBuffer` in bytes — the `OUT CBuffer` allocation ceiling.
    /// The CONSUMING project's setting; see `emit_link_support`.
    max_buffer_bytes: u64,
}

pub(super) fn emit_link_support(
    link_functions: &[IrLinkFunction],
    link_cstructs: &[crate::ir::IrCStruct],
    record_fields: &HashMap<String, Vec<(String, String)>>,
    options: LinkCodegenOptions,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    libraries: &LinkLibraries,
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
    for (index, library) in library_index.iter().enumerate() {
        let resolved = libraries.get(library)?;
        data_objects.push(cstring_object(&lib_symbol(index), &resolved.dlopen_name));
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

    // plan-59-A: the resource TYPES that are represented as 80-byte records —
    // which is now EVERY type a native func returns as `AS RES R`, with or
    // without a `STATE S`. (plan-53-A wrapped only the stateful ones, so a
    // stateless `Db` was the raw handle and had nowhere to put a `closed` flag.)
    //
    // Record-ness is per-TYPE, not per-declaration: a BARE `RES db AS R` param
    // still receives a record pointer and must load the handle from FD@0 before
    // the native call (e.g. `close`/`exec`). This set is what lets a thunk tell a
    // record-resource param from a scalar one, and it must stay in lockstep with
    // the return-side wrap below — widening one without the other hands `FD@0` a
    // raw handle to dereference.
    let record_native_resources: HashSet<String> = link_functions
        .iter()
        .filter(|f| f.return_resource)
        .map(|f| crate::builtins::resource::base_resource_name(&f.return_type).to_string())
        .collect();

    let initializer = lower_link_initializer(
        link_functions,
        &library_index,
        globals_base,
        link_count,
        &free_index_of,
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
        // handle = dlopen(filename, RTLD_NOW)
        emit_data_address(
            symbol,
            abi::return_register(),
            &lib_symbol(lib_idx),
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::move_immediate(abi::ARG[1], "Integer", "2")); // RTLD_NOW
        platform.emit_libc_call(
            "dlopen",
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
            // slot = dlsym(handle, symbolName)
            instructions.push(abi::move_register(abi::return_register(), &handle));
            emit_data_address(
                symbol,
                abi::ARG[1],
                &sym_symbol(fn_idx),
                &mut instructions,
                &mut relocations,
            );
            platform.emit_libc_call(
                "dlsym",
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
                instructions.push(abi::move_register(abi::return_register(), &handle));
                emit_data_address(
                    symbol,
                    abi::ARG[1],
                    &free_sym_symbol(k),
                    &mut instructions,
                    &mut relocations,
                );
                platform.emit_libc_call(
                    "dlsym",
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
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_NATIVE_LINK_LOAD_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    emit_data_address(
        symbol,
        RESULT_ERROR_MESSAGE_REGISTER,
        ERR_NATIVE_LINK_LOAD_SYMBOL,
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

/// Emit one marshaling thunk for a `LINK` function (plan-linker.md §12.2/§12.3).
fn lower_link_thunk(
    function: &IrLinkFunction,
    link_cstructs: &[crate::ir::IrCStruct],
    record_fields: &HashMap<String, Vec<(String, String)>>,
    ctx: ThunkContext,
    record_native_resources: &HashSet<String>,
) -> Result<CodeFunction, String> {
    let ThunkContext {
        index,
        globals_base,
        free_slot,
        max_buffer_bytes,
    } = ctx;
    let symbol = link_thunk_symbol(&function.alias, &function.name);
    let n_params = function.params.len();
    let m_slots = function.abi_slots.len();
    // Only SCALAR OUT slots occupy the 8-byte OUT region; a struct slot gets a
    // sized buffer in the struct region instead (plan-50-E). Counting structs here
    // would inflate the frame and, worse, desynchronize `expr_offsets` from the
    // staging loop's sequence.
    let is_struct_ctype = |ctype: &str| {
        link_cstructs
            .iter()
            .any(|c| c.alias == function.alias && c.name == ctype)
    };
    let n_out = function
        .abi_slots
        .iter()
        .filter(|slot| slot.direction.writes_back() && !is_struct_ctype(&slot.ctype))
        .count();

    const STATUS_OFF: usize = 8;
    const CRET_OFF: usize = 16;
    // Scratch slot 24 is reserved for string-return marshaling (RET_OFF).
    let param_base = 32;
    let cslot_base = param_base + n_params * 8;
    let out_base = cslot_base + m_slots * 8;
    // One extra slot past the OUT buffers holds the floating-point return bits
    // (`d0`) when the native return is `CDouble`.
    let cretd_off = out_base + n_out * 8;

    // plan-50-E: a struct slot needs a sized, aligned buffer for the C struct
    // itself; the cslot holds its ADDRESS. Layouts are recomputed here from the
    // CSTRUCT's field ctypes — never transported — so a crafted package cannot
    // dictate an offset.
    let target = "";
    let cstruct_of = |ctype: &str| -> Option<&crate::ir::IrCStruct> {
        link_cstructs
            .iter()
            .find(|c| c.alias == function.alias && c.name == ctype)
    };
    // (slot index) -> (buffer offset, layout, the CSTRUCT)
    let mut struct_slots: Vec<(usize, usize, crate::ir::CLayout, &crate::ir::IrCStruct)> =
        Vec::new();
    let mut struct_cursor = cretd_off + 8;
    for (slot_idx, slot) in function.abi_slots.iter().enumerate() {
        let Some(decl) = cstruct_of(&slot.ctype) else {
            continue;
        };
        let fields: Vec<(String, String)> = decl
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.ctype.clone()))
            .collect();
        let layout = crate::ir::compute_c_layout(&fields, target)?;
        struct_cursor = align(struct_cursor, layout.align);
        struct_slots.push((slot_idx, struct_cursor, layout.clone(), decl));
        struct_cursor += layout.size;
    }
    // plan-50-F: a record with `String` fields does NOT hold String
    // pointers — it INLINES each String's block into a trailing data region and
    // stores the block-relative OFFSET in the field slot. See
    // `record_field_is_inlined` / `emit_build_inlined_record`: only
    // Address/Datagram/DatagramText/AudioDevice keep pointer strings, and a
    // CSTRUCT can map to none of those. So the whole record must be built in one
    // allocation whose size depends on every field's strlen — which means every
    // length is measured BEFORE the record is allocated, and each needs a slot.
    //
    // Per CString field: [char* , len]. Then a cursor and the running total.
    let cstr_area = align(struct_cursor, 8);
    let n_cstr = struct_slots
        .iter()
        .map(|(_, _, _, decl)| decl.fields.iter().filter(|f| f.ctype == "CString").count())
        .max()
        .unwrap_or(0);
    let cursor_off = cstr_area + n_cstr * 16;
    let total_off = cursor_off + 8;
    // plan-53-A: two scratch slots for building a stateful native resource's
    // 80-byte record after the call — one parks the native handle, one the record
    // pointer, across the `arena_alloc` that clobbers all caller-saved registers.
    let rec_handle_off = total_off + 8;
    let rec_ptr_off = rec_handle_off + 8;
    // plan-58-B: one scratch word per `OUT CBuffer` slot holding its byte
    // capacity `N`. It must be a FRAME word, not a register: the byte-list
    // allocation between computing `N` and using it destroys every caller-saved
    // register, and `emit_alloc_byte_list` reads the count from a frame offset
    // anyway. The block POINTER lives in the slot's ordinary `out_base` word.
    let cbuffer_slots: Vec<usize> = function
        .abi_slots
        .iter()
        .enumerate()
        .filter(|(_, slot)| slot.ctype == "CBuffer")
        .map(|(idx, _)| idx)
        .collect();
    let cbuffer_size_base = rec_ptr_off + 8;
    let frame = align(cbuffer_size_base + cbuffer_slots.len() * 8 + 24, 16);

    // plan-50-H: the wrapper's result is whatever `RETURN <expr>` names. A bare
    // `RETURN <slot>` (an `IrLinkExpr::Var`) selects that slot's value; anything
    // else is a computed expression. Both magic-name tests — `slot.name ==
    // "return"` and `abi_return_name == "return"` — are gone.
    let result_var: Option<&str> = match &function.result {
        Some(IrLinkExpr::Var(name)) => Some(name.as_str()),
        _ => None,
    };

    // §12.3/§12.4 boundary validations that this signature needs.
    // The C return is the result exactly when `RETURN` names it.
    let returns_value = result_var == Some(function.abi_return_name.as_str());
    // plan-50-E: a BIND IN field range-checks the same way a CInt32 argument does
    // (a 64-bit Integer that does not fit the C field fails with ErrOverflow
    // rather than truncating), so it needs the same `range_fail` block. Without
    // this the thunk branches to a label that is never emitted.
    let bind_in_needs_range = function.bind_in.iter().any(|bind| {
        let Some(slot) = function.abi_slots.iter().find(|s| s.name == bind.slot) else {
            return false;
        };
        let Some(decl) = link_cstructs
            .iter()
            .find(|c| c.alias == function.alias && c.name == slot.ctype)
        else {
            return false;
        };
        bind.fields.iter().any(|field| {
            decl.fields
                .iter()
                .find(|f| f.name == field.name)
                .is_some_and(|f| narrow_signed_bits(&f.ctype).is_some())
        })
    });
    let needs_range = bind_in_needs_range
        || function.abi_slots.iter().any(|slot| {
            !slot.direction.writes_back()
                && slot.ctype == "CInt32"
                && function.params.iter().any(|(name, _)| name == &slot.name)
        });
    // plan-50-F: a `CString` struct field is copied out with the same helper, so
    // it needs the same `encoding_fail` block. Without this the thunk branches to
    // a label that is never emitted.
    let struct_has_cstring_field = function.abi_slots.iter().any(|slot| {
        link_cstructs
            .iter()
            .find(|c| c.alias == function.alias && c.name == slot.ctype)
            .is_some_and(|c| c.fields.iter().any(|f| f.ctype == "CString"))
    });
    let needs_encoding = struct_has_cstring_field
        || (returns_value
            && function.abi_return_ctype == "CPtr"
            && function.return_type == "String");
    let needs_float = returns_value && function.abi_return_ctype == "CDouble";

    let alloc_fail = format!("{symbol}_alloc_fail");
    let call_fail = format!("{symbol}_call_fail");
    let range_fail = format!("{symbol}_range_fail");
    let encoding_fail = format!("{symbol}_encoding_fail");
    let nan_fail = format!("{symbol}_nan_fail");
    let inf_fail = format!("{symbol}_inf_fail");
    // plan-58-B: the runtime size gate for `BUFFER … SIZE`.
    let buffer_size_fail = format!("{symbol}_buffer_size_fail");
    let needs_buffer_size = !cbuffer_slots.is_empty();
    let done = format!("{symbol}_done");

    // Map wrapper-parameter name -> declared order (its incoming register).
    let param_index: HashMap<&str, usize> = function
        .params
        .iter()
        .enumerate()
        .map(|(idx, (name, _))| (name.as_str(), idx))
        .collect();
    // Map const-pin slot name -> immediate.
    let const_for: HashMap<&str, i64> = function
        .consts
        .iter()
        .map(|(name, value)| (name.as_str(), *value))
        .collect();

    // Vreg-allocated (plan-00-G Phase 2): the C-ABI marshaling slots are an
    // explicit `sp`-relative local region; x9/x10/x16 scratch become vregs the
    // allocator places (incoming wrapper args stay in their ABI x0-x7).
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // Save incoming wrapper arguments before any clobbering call.
    for index in 0..n_params {
        instructions.push(abi::store_u64(
            &abi::argument_register(index)?,
            abi::stack_pointer(),
            param_base + index * 8,
        ));
    }

    // One label counter shared across the CBuffer SIZE expressions, the
    // SUCCESS_ON gate and the RESULT expression: a per-block counter restarted at
    // 0 in each produced two identically named labels (`{symbol}_cmp0_end`) when
    // two of them emitted a comparison/NOT — a duplicate the encoder rejects
    // outright (bug-79). Declared here, above the first emitter, rather than at
    // the SUCCESS_ON gate where it used to live.
    let mut counter = 0usize;

    // plan-58-B: stage every `OUT CBuffer` BEFORE the main slot loop.
    //
    // Every other slot kind stages into FRAME storage sized at compile time. A
    // CBuffer cannot: its size is a runtime value, and its storage must outlive
    // the call because it becomes the returned MFBASIC value. So it is an arena
    // block, and allocating one destroys every caller-saved register
    // (`_mfb_arena_alloc` has no survivor set — `.ai/compiler.md`).
    //
    // Doing it in a separate pass is what makes that safe: at this point the only
    // live state is in frame words (the wrapper's parameters, spilled on entry),
    // so there is nothing in a register for the allocation to destroy. Running it
    // inside the main loop would clobber slots already staged into registers by
    // earlier iterations. `tests/rt-behavior/native/cbuffer_read` pins this with
    // scalar slots staged on BOTH sides of the buffer.
    //
    // The offsets used here must agree with the main loop's `out_seq` sequence and
    // with `expr_offsets`, or every expression variable after the buffer resolves
    // to the wrong slot. All three walk `writes_back() && !is_struct_ctype`.
    let cbuffer_out_off = |target_idx: usize| -> usize {
        let mut seq = 0usize;
        for (idx, slot) in function.abi_slots.iter().enumerate() {
            if is_struct_ctype(&slot.ctype) {
                continue;
            }
            if !slot.direction.writes_back() {
                continue;
            }
            if idx == target_idx {
                return out_base + seq * 8;
            }
            seq += 1;
        }
        unreachable!("a CBuffer slot is always an OUT non-struct slot")
    };
    for (buf_seq, &slot_idx) in cbuffer_slots.iter().enumerate() {
        let slot = &function.abi_slots[slot_idx];
        let size_off = cbuffer_size_base + buf_seq * 8;
        let out_off = cbuffer_out_off(slot_idx);
        let cslot_off = cslot_base + slot_idx * 8;

        // `check_buffer_slots` guarantees exactly one clause per CBuffer slot, so
        // this cannot be absent in a well-formed function. A decoded `.mfp` does
        // not carry BUFFER clauses yet (plan-58-C), and `ir::verify` rejects such
        // a package through rule 2 — but that runs on the project, not here, so
        // fail loudly rather than allocating a zero-length buffer.
        let Some(buffer) = function.buffers.iter().find(|b| b.slot == slot.name) else {
            return Err(format!(
                "LINK function '{}.{}' CBuffer slot '{}' has no BUFFER SIZE clause",
                function.alias, function.name, slot.name
            ));
        };

        // A SIZE expression may read only wrapper parameters and CONST pins
        // (plan-58-A rule 9, tightened): those are the only values that exist
        // before the call. Parameters are already spilled to `param_base`; pin
        // immediates are materialized into their cslot words here, which the main
        // loop then writes again — harmless, and it keeps the pin's value in the
        // one place an expression can read it from.
        let mut size_offsets: HashMap<&str, usize> = HashMap::new();
        for (name, &pidx) in &param_index {
            size_offsets.insert(name, param_base + pidx * 8);
        }
        for (pin_name, value) in &const_for {
            let Some(pin_idx) = function
                .abi_slots
                .iter()
                .position(|s| &s.name.as_str() == pin_name)
            else {
                continue;
            };
            let pin_off = cslot_base + pin_idx * 8;
            instructions.extend([
                abi::move_immediate("%v9", "Integer", &(*value as u64).to_string()),
                abi::store_u64("%v9", abi::stack_pointer(), pin_off),
            ]);
            size_offsets.entry(pin_name).or_insert(pin_off);
        }

        let mut vreg = LINK_EXPR_VREG_BASE;
        let size_reg = emit_link_expr(
            &buffer.size,
            &size_offsets,
            &mut vreg,
            &symbol,
            &mut counter,
            &mut instructions,
        );
        instructions.push(abi::store_u64(&size_reg, abi::stack_pointer(), size_off));

        // Gate the size BEFORE allocating. A negative `N` would compute a nonsense
        // block size, and an unbounded one is a whole-arena request driven
        // straight from a wrapper parameter. Signed compares on both ends: an
        // unsigned compare would let a negative `N` read as enormous and pass the
        // lower bound.
        instructions.extend([
            abi::compare_immediate(&size_reg, "0"),
            abi::branch_lt(&buffer_size_fail),
            abi::move_immediate("%v9", "Integer", &max_buffer_bytes.to_string()),
            abi::compare_registers(&size_reg, "%v9"),
            abi::branch_gt(&buffer_size_fail),
        ]);

        // Allocate the block and spill its pointer to the OUT word. The helper
        // writes the header (count/capacity/dataLength/dataCapacity all `N`) and
        // branches to `alloc_fail` on failure, so a wrapper with no LENGTH clause
        // needs no post-call work and the list is well-formed even if the callee
        // writes nothing.
        emit_alloc_byte_list(
            &symbol,
            &format!("cbuf{buf_seq}"),
            size_off,
            out_off,
            &alloc_fail,
            &mut instructions,
            &mut relocations,
        );

        // The C function gets `dataBase`, NOT the block pointer. Two different
        // pointers 40 bytes apart: hand over the block and the callee overwrites
        // the header, which a short write corrupts only partially and therefore
        // plausibly. Reload the block from its frame word first — the allocation
        // destroyed every register.
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), out_off),
            abi::add_immediate("%v9", "%v9", COLLECTION_HEADER_SIZE),
            abi::store_u64("%v9", abi::stack_pointer(), cslot_off),
        ]);
    }

    // Compute each C argument into its scratch slot. OUT buffers are addressed,
    // const pins are pinned, and ordinary params are marshaled per ABI type.
    let mut out_seq = 0usize;
    let mut result_out_off: Option<usize> = None;
    // The OUT-slot result's ctype drives the same marshalling the direct-return
    // path applies (bug-238); without it a `CInt32` OUT surfaced -1 as 4294967295.
    let mut result_out_ctype: Option<String> = None;
    for (slot_idx, slot) in function.abi_slots.iter().enumerate() {
        let cslot_off = cslot_base + slot_idx * 8;
        // plan-58-B: a CBuffer was fully staged by the pass above — block
        // allocated, pointer in its OUT word, `dataBase` in its cslot. Skip it
        // here, but ONLY after advancing `out_seq`: the sequence must match
        // `expr_offsets` and the staging pass, or every expression variable after
        // the buffer resolves to the wrong slot.
        //
        // Falling through instead would be silently destructive — the generic
        // scalar-OUT arm below overwrites the cslot with `&out_word` and ZEROES
        // the out word that now holds the block pointer, so the two stagings
        // clobber each other and the callee receives a pointer to frame memory.
        if slot.ctype == "CBuffer" {
            if result_var == Some(slot.name.as_str()) {
                result_out_off = Some(out_base + out_seq * 8);
                result_out_ctype = Some(slot.ctype.clone());
            }
            out_seq += 1;
            continue;
        }
        // plan-50-E: a struct slot passes the ADDRESS of a zeroed buffer, exactly
        // as an OUT scalar does — only sized.
        if let Some((_, buf_off, layout, decl)) =
            struct_slots.iter().find(|(idx, ..)| *idx == slot_idx)
        {
            // Zero the WHOLE buffer first. Mandatory, not hygiene: libsndfile
            // requires a zeroed SF_INFO for a non-RAW read, and an unzeroed buffer
            // leaks this thunk's stack into the C library. The tail uses narrower
            // stores so a struct whose size is not a multiple of 8 cannot write
            // past its own buffer into the next one.
            let mut z = 0usize;
            while z + 8 <= layout.size {
                instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), buf_off + z));
                z += 8;
            }
            while z + 4 <= layout.size {
                instructions.push(abi::store_u32(abi::ZERO, abi::stack_pointer(), buf_off + z));
                z += 4;
            }
            while z + 2 <= layout.size {
                instructions.push(abi::store_u16(abi::ZERO, abi::stack_pointer(), buf_off + z));
                z += 2;
            }
            while z < layout.size {
                instructions.push(abi::store_u8(abi::ZERO, abi::stack_pointer(), buf_off + z));
                z += 1;
            }
            // Then the bound input fields; everything else stays zero.
            marshal_struct_in(
                function,
                decl,
                layout,
                *buf_off,
                &slot.name,
                &symbol,
                param_base,
                &param_index,
                &range_fail,
                &alloc_fail,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::add_immediate("%v9", abi::stack_pointer(), *buf_off),
                abi::store_u64("%v9", abi::stack_pointer(), cslot_off),
            ]);
            continue;
        }
        if slot.direction.writes_back() {
            let out_off = out_base + out_seq * 8;
            out_seq += 1;
            instructions.extend([
                abi::store_u64(abi::ZERO, abi::stack_pointer(), out_off),
                abi::add_immediate("%v9", abi::stack_pointer(), out_off),
                abi::store_u64("%v9", abi::stack_pointer(), cslot_off),
            ]);
            if result_var == Some(slot.name.as_str()) {
                result_out_off = Some(out_off);
                result_out_ctype = Some(slot.ctype.clone());
            }
        } else if let Some(value) = const_for.get(slot.name.as_str()) {
            // §12.3: a `CInt32` slot is a signed 32-bit C argument. A param feeding
            // this slot is range-checked at runtime (`range_fail` → `ErrOverflow`);
            // a `CONST` pin is known at compile time, so an out-of-range value is
            // rejected here rather than silently truncated to its low 32 bits
            // (bug-66).
            if slot.ctype == "CInt32"
                && (*value < i64::from(i32::MIN) || *value > i64::from(i32::MAX))
            {
                return Err(format!(
                    "LINK function '{}.{}' CONST pin '{} = {}' does not fit the signed 32-bit \
                     range of its CInt32 ABI slot",
                    function.alias, function.name, slot.name, value
                ));
            }
            instructions.extend([
                abi::move_immediate("%v9", "Integer", &(*value as u64).to_string()),
                abi::store_u64("%v9", abi::stack_pointer(), cslot_off),
            ]);
        } else if let Some(&pidx) = param_index.get(slot.name.as_str()) {
            let param_off = param_base + pidx * 8;
            if slot.ctype == "CString" {
                emit_copy_string_to_cstring(
                    &symbol,
                    param_off,
                    cslot_off,
                    &alloc_fail,
                    &mut instructions,
                    &mut relocations,
                );
            } else if slot.ctype == "CInt32" {
                // §12.3: the 64-bit MFBASIC Integer must fit signed 32-bit; an
                // out-of-range value fails rather than silently truncating.
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), param_off),
                    abi::shift_left_immediate("%v10", "%v9", 32),
                    abi::arithmetic_shift_right_immediate("%v10", "%v10", 32),
                    abi::compare_registers("%v9", "%v10"),
                    abi::branch_ne(&range_fail),
                    abi::store_u64("%v9", abi::stack_pointer(), cslot_off),
                ]);
            } else if slot.ctype == "CPtr"
                && function.params.get(pidx).is_some_and(|(_, t)| {
                    record_native_resources
                        .contains(crate::builtins::resource::base_resource_name(t))
                })
            {
                // plan-59-A: a param whose resource TYPE is a native resource is a
                // RECORD pointer, but the native symbol wants the handle it wraps.
                // Load FD@0. Record-ness is per-TYPE and no longer depends on
                // STATE: this fires for a BARE `RES db AS Db` param (e.g.
                // `close`/`exec`) just as it does for a stateful `SoundFile` —
                // without it the record pointer, not the handle, reaches the C
                // library.
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), param_off),
                    abi::load_u64("%v9", "%v9", FILE_OFFSET_FD),
                    abi::store_u64("%v9", abi::stack_pointer(), cslot_off),
                ]);
            } else {
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), param_off),
                    abi::store_u64("%v9", abi::stack_pointer(), cslot_off),
                ]);
            }
        } else {
            return Err(format!(
                "LINK function '{}.{}' ABI slot '{}' has no source (param, const, or OUT)",
                function.alias, function.name, slot.name
            ));
        }
    }

    // Load the C arguments into their AAPCS64 registers, then call through the
    // resolved pointer.
    // bug-296: a LINK thunk calls a real C function through `blr`, so its
    // arguments follow the target's EXTERNAL C ABI, not the compiler's internal
    // 8-register model. On SysV x86-64 only six integer arguments are passed in
    // registers; the backend's `CALL_ARGS` extends the list with rax/rbp for
    // arguments 7 and 8, which is sound for the compiler's own calls but hands an
    // external callee two registers it never reads -- it takes those from the
    // stack, so it saw garbage, silently and with no diagnostic. Stack-argument
    // staging for external calls is the complete fix; until it exists, refuse the
    // call rather than emit one that is wrong.
    let external_int_registers = crate::target::shared::code::mir::active_backend()
        .register_model()
        .external_int_argument_registers();
    let int_slot_count = function
        .abi_slots
        .iter()
        .filter(|slot| slot.ctype != "CDouble")
        .count();
    if int_slot_count > external_int_registers {
        return Err(format!(
            "native function `{}` declares {int_slot_count} integer ABI slots, but this \
             target passes only {external_int_registers} integer arguments in registers and \
             stack arguments are not yet staged for native calls; reduce the slot count or \
             build for a target with more argument registers",
            function.name
        ));
    }
    let mut int_idx = 0usize;
    let mut flt_idx = 0usize;
    for (slot_idx, slot) in function.abi_slots.iter().enumerate() {
        let cslot_off = cslot_base + slot_idx * 8;
        if slot.ctype == "CDouble" {
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), cslot_off),
                abi::float_move_d_from_x(abi::fp_argument_register(flt_idx)?, "%v9"),
            ]);
            flt_idx += 1;
        } else {
            instructions.push(abi::load_u64(
                &abi::argument_register(int_idx)?,
                abi::stack_pointer(),
                cslot_off,
            ));
            int_idx += 1;
        }
    }
    instructions.extend([
        abi::load_u64(
            "%v16",
            ARENA_STATE_REGISTER,
            slot_offset(globals_base, index),
        ),
        abi::branch_link_register("%v16"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CRET_OFF),
    ]);
    if needs_float {
        // A `double` return arrives in `d0`, not `x0`; stash its bits.
        instructions.extend([
            abi::float_move_x_from_d("%v9", abi::FP_SCRATCH[0]),
            abi::store_u64("%v9", abi::stack_pointer(), cretd_off),
        ]);
    }

    // Derive the status value (sign-extending a 32-bit native return).
    instructions.push(abi::load_u64("%v9", abi::stack_pointer(), CRET_OFF));
    if function.abi_return_ctype == "CInt32" {
        instructions.extend([
            abi::shift_left_immediate("%v9", "%v9", 32),
            abi::arithmetic_shift_right_immediate("%v9", "%v9", 32),
        ]);
    }
    instructions.push(abi::store_u64("%v9", abi::stack_pointer(), STATUS_OFF));

    // plan-50-I: every name a `Var` may hold, mapped to the frame slot holding
    // its value. The ABI return maps to STATUS_OFF, NOT CRET_OFF — STATUS_OFF
    // holds the sign-extended value, which is what `SUCCESS_ON status = -1` must
    // compare against.
    let mut expr_offsets: HashMap<&str, usize> = HashMap::new();
    expr_offsets.insert(function.abi_return_name.as_str(), STATUS_OFF);
    {
        let mut seq = 0usize;
        for (slot_idx, slot) in function.abi_slots.iter().enumerate() {
            // A struct slot holds an address, not a value an expression can read;
            // it is skipped so the OUT sequence matches the staging loop exactly.
            if is_struct_ctype(&slot.ctype) {
                continue;
            }
            let off = if slot.direction.writes_back() {
                let o = out_base + seq * 8;
                seq += 1;
                o
            } else {
                cslot_base + slot_idx * 8
            };
            // The ABI return name wins a collision: it is the older meaning and
            // `SUCCESS_ON status = 0` must keep resolving to the status.
            expr_offsets.entry(slot.name.as_str()).or_insert(off);
        }
    }

    // SUCCESS_ON gate: a failing status produces an Error result.
    if let Some(success) = &function.success_on {
        let mut vreg = LINK_EXPR_VREG_BASE;
        let value = emit_link_expr(
            success,
            &expr_offsets,
            &mut vreg,
            &symbol,
            &mut counter,
            &mut instructions,
        );
        instructions.extend([
            abi::compare_immediate(&value, "0"),
            abi::branch_eq(&call_fail),
        ]);
    }

    // Produce the wrapper result value in RESULT_VALUE_REGISTER (x1).
    // plan-50-E: a bare `RETURN <struct-slot>` builds the mapped record from the
    // slot's post-call buffer. Checked first — a struct slot is also `writes_back`,
    // so it would otherwise be mistaken for a scalar OUT.
    if let Some((_, buf_off, layout, decl)) = result_var.and_then(|name| {
        struct_slots
            .iter()
            .find(|(idx, ..)| function.abi_slots[*idx].name == name)
    }) {
        marshal_struct_out(
            function,
            decl,
            layout,
            *buf_off,
            record_fields,
            &symbol,
            cstr_area,
            cursor_off,
            total_off,
            &alloc_fail,
            &encoding_fail,
            &nan_fail,
            &inf_fail,
            &mut instructions,
            &mut relocations,
        )?;
    } else if let Some(out_off) = result_out_off {
        // bug-238: an OUT-slot result carries the same C value shapes as a direct
        // return, so apply the same ctype-driven marshalling instead of a bare
        // 8-byte load — otherwise a `CInt32` OUT writing -1 surfaced as
        // 4294967295 (zero-extended) and a `CDouble` OUT bypassed the finiteness
        // rejection an MFBASIC `Float` requires.
        match result_out_ctype.as_deref().unwrap_or("") {
            "CInt32" => {
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), out_off),
                    abi::sign_extend_word(RESULT_VALUE_REGISTER, "%v9"),
                ]);
            }
            "CBool" => {
                let set = format!("{symbol}_out_bool_true");
                let end = format!("{symbol}_out_bool_end");
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), out_off),
                    abi::compare_immediate("%v9", "0"),
                    abi::branch_ne(&set),
                    abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                    abi::branch(&end),
                    abi::label(&set),
                    abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                    abi::label(&end),
                ]);
            }
            "CByte" => {
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), out_off),
                    abi::move_immediate("%v10", "Integer", "255"),
                    abi::and_registers(RESULT_VALUE_REGISTER, "%v9", "%v10"),
                ]);
            }
            "CDouble" => {
                // Mirrors the direct-return finiteness gate: a non-finite double has
                // all exponent bits set; the mantissa distinguishes Inf from NaN.
                let finite = format!("{symbol}_out_float_finite");
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), out_off),
                    abi::move_immediate("%v10", "Integer", "9218868437227405312"),
                    abi::and_registers("%v11", "%v9", "%v10"),
                    abi::compare_registers("%v11", "%v10"),
                    abi::branch_ne(&finite),
                    abi::move_immediate("%v12", "Integer", "4503599627370495"),
                    abi::and_registers("%v13", "%v9", "%v12"),
                    abi::compare_immediate("%v13", "0"),
                    abi::branch_eq(&inf_fail),
                    abi::branch(&nan_fail),
                    abi::label(&finite),
                    abi::move_register(RESULT_VALUE_REGISTER, "%v9"),
                ]);
            }
            // plan-58-B: the OUT word holds the byte-list BLOCK pointer, which is
            // exactly the wrapper's `List OF Byte` result — so this is a plain
            // load, and the `_` default below would coincidentally do the same
            // thing.
            //
            // The arm is written out anyway. Relying on a silent default is how
            // bug-238 happened: that same `_` is why a `CInt32` OUT surfaced `-1`
            // as `4294967295`. An arm that agrees with the default today still
            // states the intent, and stops a future edit to the default from
            // silently changing what a CBuffer returns.
            "CBuffer" => {
                // Truncate to what the callee actually wrote, then hand back the
                // block pointer — which IS the wrapper's `List OF Byte`.
                //
                // `LENGTH` is mandatory on a CBuffer (plan-58-B rule 10), so this
                // arm always has one to evaluate. Its expression is evaluated
                // HERE, after the call, which is why it may read the ABI return
                // and OUT slots that `BUFFER … SIZE` may not.
                let length = function
                    .result_length
                    .as_ref()
                    .expect("check_buffer_slots requires LENGTH on a returned CBuffer");
                let mut vreg = LINK_EXPR_VREG_BASE;
                let k = emit_link_expr(
                    length,
                    &expr_offsets,
                    &mut vreg,
                    &symbol,
                    &mut counter,
                    &mut instructions,
                );
                // Clamp to [0, N]. Not defensive padding: `pread`/`read` return
                // -1 on error and `sf_read_short` returns -1 or 0 at EOF. An
                // unclamped negative stored to `count` is a huge UNSIGNED value,
                // and every later collection read then walks off the block. An
                // over-capacity value does the same more slowly.
                // No counter suffix: a thunk has ONE `RETURN`, and rule 6 requires
                // a CBuffer to be the slot it names, so at most one CBuffer per
                // thunk reaches this arm and these labels cannot collide.
                let neg = format!("{symbol}_cbuf_len_neg");
                let capped = format!("{symbol}_cbuf_len_capped");
                let size_off = cbuffer_size_base
                    + cbuffer_slots
                        .iter()
                        .position(|&idx| {
                            function.abi_slots[idx].name.as_str() == result_var.unwrap_or_default()
                        })
                        .expect("the returned CBuffer is in cbuffer_slots")
                        * 8;
                instructions.extend([
                    abi::compare_immediate(&k, "0"),
                    abi::branch_lt(&neg),
                    // Reload N: the call clobbered everything.
                    abi::load_u64("%v9", abi::stack_pointer(), size_off),
                    abi::compare_registers(&k, "%v9"),
                    abi::branch_le(&capped),
                    abi::move_register(&k, "%v9"),
                    abi::branch(&capped),
                    abi::label(&neg),
                    abi::move_immediate(&k, "Integer", "0"),
                    abi::label(&capped),
                ]);
                // `capacity`/`dataCapacity` deliberately stay at N. That is what
                // makes `arena_free` reclaim the whole block: `emit_flat_block_size`
                // sizes from capacity, so lowering it to k would leak the tail.
                // `05_collections.md:173-198` sanctions `capacity > count` as
                // headroom, and a value copy is shrink-to-fit.
                instructions.extend([
                    abi::load_u64("%v10", abi::stack_pointer(), out_off),
                    abi::store_u64(&k, "%v10", COLLECTION_OFFSET_COUNT),
                    abi::store_u64(&k, "%v10", COLLECTION_OFFSET_DATA_LENGTH),
                    abi::move_register(RESULT_VALUE_REGISTER, "%v10"),
                ]);
            }
            _ => {
                instructions.push(abi::load_u64(
                    RESULT_VALUE_REGISTER,
                    abi::stack_pointer(),
                    out_off,
                ));
            }
        }
    } else if result_var == Some(function.abi_return_name.as_str()) {
        emit_return_passthrough(
            function,
            ReturnMarshal {
                cret_off: CRET_OFF,
                cretd_off,
                status_off: STATUS_OFF,
                alloc_fail: &alloc_fail,
                encoding_fail: &encoding_fail,
                nan_fail: &nan_fail,
                inf_fail: &inf_fail,
            },
            &symbol,
            &mut instructions,
            &mut relocations,
        )?;
    } else if let Some(result) = &function.result {
        let mut vreg = LINK_EXPR_VREG_BASE;
        let value = emit_link_expr(
            result,
            &expr_offsets,
            &mut vreg,
            &symbol,
            &mut counter,
            &mut instructions,
        );
        instructions.push(abi::move_register(RESULT_VALUE_REGISTER, &value));
    } else {
        instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    }

    // plan-59-A: a native func that produces `AS RES T` — with or without a
    // `STATE S` — hands back a resource RECORD, not the bare handle. Wrapping the
    // stateless case too is what gives it a `closed` flag at offset 8, which it
    // had nowhere to store while the handle itself was the value.
    // RESULT_VALUE_REGISTER currently holds
    // the native handle; wrap it in an 80-byte resource record so the value the
    // caller binds is a pointer to {FD@0, CLOSED@8, STATE@16, buffers…} — the exact
    // shape a built-in `File STATE S` uses, so `.state`, drop-reclamation
    // (plan-52-B), and the closed guard all work unchanged. STATE@16 is left NULL:
    // the caller's `RES x AS T STATE S = …` bind runs `emit_resource_state_init`,
    // which default-allocates the `S` record exactly as it does for a built-in
    // resource (or `BIND STATE` populates it first — plan-53-B). Without this the
    // handle IS the value and `.state` writes at offset 16 of the native handle's
    // own memory — memory corruption (the defect this fixes).
    if function.return_resource {
        instructions.extend([
            // Park the handle; alloc clobbers every caller-saved register.
            abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), rec_handle_off),
            abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
        ]);
        emit_alloc(&symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            // record pointer (RET[1]) → scratch; reload handle; store handle@FD.
            abi::store_u64(abi::RET[1], abi::stack_pointer(), rec_ptr_off),
            abi::load_u64("%v9", abi::stack_pointer(), rec_handle_off),
            abi::load_u64("%v10", abi::stack_pointer(), rec_ptr_off),
            abi::store_u64("%v9", "%v10", FILE_OFFSET_FD),
            // Zero CLOSED (open) and the File I/O buffer words a native resource
            // never uses (they must be zero, not the arena-alloc's poison —
            // plan-52-B). STATE@16 is handled below.
            abi::store_u64(abi::ZERO, "%v10", FILE_OFFSET_CLOSED),
            abi::store_u64(abi::ZERO, "%v10", FILE_OFFSET_BUF_PTR),
            abi::store_u64(abi::ZERO, "%v10", FILE_OFFSET_BUF_FILLED),
            abi::store_u64(abi::ZERO, "%v10", FILE_OFFSET_BUF_ENABLED),
            abi::store_u64(abi::ZERO, "%v10", FILE_OFFSET_READ_PTR),
            abi::store_u64(abi::ZERO, "%v10", FILE_OFFSET_READ_POS),
            abi::store_u64(abi::ZERO, "%v10", FILE_OFFSET_READ_FILL),
            abi::store_u64(abi::ZERO, "%v10", FILE_OFFSET_READ_AT_EOF),
        ]);
        // STATE@16: `BIND STATE <res> = <out-struct>` (plan-53-B) marshals the OUT
        // struct the native call filled into an `S` record and stores its pointer;
        // otherwise leave it null so the caller's bind default-inits it (a built-in
        // `File STATE S` works the same way — the producer never inits STATE).
        if let Some(struct_slot_name) = function.bind_state.as_deref() {
            let Some((_, buf_off, layout, decl)) = struct_slots
                .iter()
                .find(|(idx, ..)| function.abi_slots[*idx].name == struct_slot_name)
            else {
                return Err(format!(
                    "LINK function '{}.{}' BIND STATE names '{struct_slot_name}', which is not an OUT struct slot",
                    function.alias, function.name
                ));
            };
            // marshal_struct_out arena-allocates the `S` record from the post-call
            // buffer and leaves its pointer in RESULT_VALUE_REGISTER. It clobbers
            // scratch, so the record pointer is reloaded from its slot afterward.
            marshal_struct_out(
                function,
                decl,
                layout,
                *buf_off,
                record_fields,
                &symbol,
                cstr_area,
                cursor_off,
                total_off,
                &alloc_fail,
                &encoding_fail,
                &nan_fail,
                &inf_fail,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("%v10", abi::stack_pointer(), rec_ptr_off),
                abi::store_u64(RESULT_VALUE_REGISTER, "%v10", FILE_OFFSET_STATE),
            ]);
        } else {
            instructions.extend([
                abi::load_u64("%v10", abi::stack_pointer(), rec_ptr_off),
                abi::store_u64(abi::ZERO, "%v10", FILE_OFFSET_STATE),
            ]);
        }
        instructions.extend([
            // The record pointer is the wrapper result.
            abi::load_u64("%v10", abi::stack_pointer(), rec_ptr_off),
            abi::move_register(RESULT_VALUE_REGISTER, "%v10"),
        ]);
    }

    // FREE: release the caller-owned native return now that it is copied into the
    // owned wrapper result (mfbasic.md §17). The original pointer is still at
    // CRET_OFF; the deallocator (a C call) clobbers x0..x18, so the result value
    // is parked in the now-free STATUS slot across the call and reloaded after.
    // A NULL pointer is passed through unchanged — deallocators such as
    // sqlite3_free treat NULL as a no-op.
    if let Some(free_slot) = free_slot {
        instructions.extend([
            abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STATUS_OFF),
            abi::load_u64(&abi::argument_register(0)?, abi::stack_pointer(), CRET_OFF),
            abi::load_u64(
                "%v16",
                ARENA_STATE_REGISTER,
                slot_offset(globals_base, free_slot),
            ),
            abi::branch_link_register("%v16"),
            abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STATUS_OFF),
        ]);
    }

    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // call_fail: SUCCESS_ON rejected the native status.
    instructions.extend([
        abi::label(&call_fail),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_NATIVE_LINK_CALL_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    emit_data_address(
        &symbol,
        RESULT_ERROR_MESSAGE_REGISTER,
        ERR_NATIVE_LINK_CALL_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::branch(&done));

    // alloc_fail: a marshaling allocation failed.
    instructions.extend([
        abi::label(&alloc_fail),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    emit_data_address(
        &symbol,
        RESULT_ERROR_MESSAGE_REGISTER,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );

    // Boundary-validation failure epilogues (plan-linker.md §12.3/§12.4), emitted
    // only when the signature can reach them.
    for (needed, label, code, message) in [
        (
            needs_range,
            &range_fail,
            ERR_OVERFLOW_CODE,
            ERR_OVERFLOW_SYMBOL,
        ),
        (
            needs_encoding,
            &encoding_fail,
            ERR_ENCODING_CODE,
            ERR_ENCODING_SYMBOL,
        ),
        (
            needs_float,
            &nan_fail,
            ERR_FLOAT_NAN_CODE,
            ERR_FLOAT_NAN_SYMBOL,
        ),
        (
            needs_float,
            &inf_fail,
            ERR_FLOAT_INF_CODE,
            ERR_FLOAT_INF_SYMBOL,
        ),
        (
            needs_buffer_size,
            &buffer_size_fail,
            ERR_INVALID_ARGUMENT_CODE,
            ERR_INVALID_ARGUMENT_SYMBOL,
        ),
    ] {
        if !needed {
            continue;
        }
        instructions.push(abi::branch(&done));
        instructions.extend([
            abi::label(label),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", code),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        ]);
        emit_data_address(
            &symbol,
            RESULT_ERROR_MESSAGE_REGISTER,
            message,
            &mut instructions,
            &mut relocations,
        );
    }

    // bug-371: every path out of the thunk must leave the error-origin register
    // (`RESULT_ERROR_SOURCE_REGISTER`) holding the no-origin sentinel. A thunk
    // has no MFBASIC source location to report, and none of the epilogues above
    // write x3 — but the register is also an argument register, so on every
    // failure it still holds whatever the marshaling staged for the native call.
    // A caller that consumes the loose error (an inline `TRAP`, which reads x3
    // and builds an `ErrorLoc` from it) then read a garbage pointer as a record
    // block: `ErrOutOfMemory` from a nonsense block size, or SIGSEGV. Zeroed at
    // `done` so the OK path is covered too, and so a new epilogue cannot forget.
    instructions.extend([
        abi::label(&done),
        abi::move_immediate(RESULT_ERROR_SOURCE_REGISTER, "Integer", "0"),
        abi::return_(),
    ]);

    let (frame_obj, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], frame);
    Ok(CodeFunction {
        name: format!("linker.{}.{}", function.alias, function.name),
        symbol,
        params: function
            .params
            .iter()
            .enumerate()
            .map(|(idx, (name, type_))| {
                Ok(CodeParam {
                    name: name.clone(),
                    type_: type_.clone(),
                    // The wrapper's incoming MFB argument register, as a role
                    // token — the thunk body saves from the same bank
                    // (plan-34-D; ≤8 params, enforced by `argument_register`).
                    location: abi::argument_register(idx)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        returns: function.return_type.clone(),
        frame: frame_obj,
        stack_slots,
        instructions,
        relocations,
    })
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

/// Marshal the native return (`AS return <ctype>`) into the wrapper result in
/// `RESULT_VALUE_REGISTER` (plan-linker.md §12.3/§12.4).
fn emit_return_passthrough(
    function: &IrLinkFunction,
    m: ReturnMarshal,
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let cret_off = m.cret_off;
    let status_off = m.status_off;
    match function.abi_return_ctype.as_str() {
        "CPtr" if function.return_type == "String" => {
            emit_copy_cstring_to_string(
                symbol,
                cret_off,
                m.alloc_fail,
                m.encoding_fail,
                instructions,
                relocations,
            );
        }
        "CDouble" => {
            // §12.3: a C `double` may be NaN/Inf, but an MFBASIC `Float` is always
            // finite (mfbasic.md §3), so reject non-finite results at the boundary.
            // A non-finite double has all exponent bits set (`0x7FF0…`); the
            // mantissa then distinguishes Inf (zero) from NaN (non-zero).
            let finite = format!("{symbol}_float_finite");
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), m.cretd_off),
                abi::move_immediate("%v10", "Integer", "9218868437227405312"),
                abi::and_registers("%v11", "%v9", "%v10"),
                abi::compare_registers("%v11", "%v10"),
                abi::branch_ne(&finite),
                abi::move_immediate("%v12", "Integer", "4503599627370495"),
                abi::and_registers("%v13", "%v9", "%v12"),
                abi::compare_immediate("%v13", "0"),
                abi::branch_eq(m.inf_fail),
                abi::branch(m.nan_fail),
                abi::label(&finite),
                abi::move_register(RESULT_VALUE_REGISTER, "%v9"),
            ]);
        }
        "CPtr" | "CInt64" => {
            instructions.push(abi::load_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                cret_off,
            ));
        }
        "CInt32" => {
            instructions.push(abi::load_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                status_off,
            ));
        }
        "CBool" => {
            let set = format!("{symbol}_bool_true");
            let end = format!("{symbol}_bool_end");
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), cret_off),
                abi::compare_immediate("%v9", "0"),
                abi::branch_ne(&set),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                abi::branch(&end),
                abi::label(&set),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                abi::label(&end),
            ]);
        }
        "CByte" => {
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), cret_off),
                abi::move_immediate("%v10", "Integer", "255"),
                abi::and_registers(RESULT_VALUE_REGISTER, "%v9", "%v10"),
            ]);
        }
        // The narrow integers carry no return-side normalization: the C ABI
        // already delivers them in the low bits of the return register, and the
        // wrapper's MFBASIC type is `Integer`. Listed explicitly (rather than
        // left to a default arm) so an unknown ctype is a hard error — plan-50-A.
        "CInt8" | "CInt16" | "CUInt8" | "CUInt16" | "CUInt32" | "CUInt64" | "CFloat" => {
            instructions.push(abi::load_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                cret_off,
            ));
        }
        other => {
            // Unreachable: `abi_slot_ctype_is_known` gates this at syntaxcheck and
            // at `ir::verify` (both paths). This exists so a ctype added to the
            // allow-list without a marshaling arm fails loudly at build time
            // instead of silently moving a raw 64-bit value.
            return Err(format!(
                "LINK function '{}.{}' has unknown ABI return ctype '{other}'",
                function.alias, function.name
            ));
        }
    }
    Ok(())
}

/// Copy the NUL-free MFBASIC `String` at `[sp + str_off]` into a freshly arena
/// allocated NUL-terminated C buffer, storing the pointer at `[sp + out_off]`.
fn emit_copy_string_to_cstring(
    symbol: &str,
    str_off: usize,
    out_off: usize,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let loop_label = format!("{symbol}_cs{out_off}_copy");
    let done_label = format!("{symbol}_cs{out_off}_done");
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), str_off),
        abi::load_u64("%v10", "%v9", 0),
        abi::add_immediate(abi::return_register(), "%v10", 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), out_off),
        abi::load_u64("%v9", abi::stack_pointer(), str_off),
        abi::load_u64("%v10", "%v9", 0),
        abi::add_immediate("%v11", "%v9", 8),
        abi::move_register("%v12", abi::RET[1]),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&loop_label),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_eq(&done_label),
        abi::load_u8("%v14", "%v11", 0),
        abi::store_u8("%v14", "%v12", 0),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&loop_label),
        abi::label(&done_label),
        abi::store_u8(abi::ZERO, "%v12", 0),
    ]);
}

/// Copy a NUL-terminated C string at `[sp + cret_off]` into an owned MFBASIC
/// `String`, leaving the result pointer in `RESULT_VALUE_REGISTER`
/// (plan-linker.md §12.4 copy-and-leave). A NULL pointer yields an empty
/// `String`.
/// The string-return scratch slot in the thunk frame.
const RESULT_SAVE_OFF: usize = 24;

/// The status slot, reused as length scratch by the whole-return string copy.
/// Must track `STATUS_OFF`.
const STATUS_SCRATCH_OFF: usize = 8;

/// Copy the thunk's `const char *` return into an owned `String`, stashed in
/// [`RESULT_SAVE_OFF`].
///
/// This is the whole-return path, which runs once and last: the status has been
/// gated and nothing reads it again, so the STATUS slot is free length scratch.
/// A record's `String` FIELD does not come through here — a field is an inlined
/// sub-block of the record, not a separate String (see `marshal_struct_out`).
fn emit_copy_cstring_to_string(
    symbol: &str,
    cret_off: usize,
    alloc_fail: &str,
    encoding_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let null_label = format!("{symbol}_ret_null");
    let len_loop = format!("{symbol}_ret_len");
    let len_done = format!("{symbol}_ret_len_done");
    let copy_loop = format!("{symbol}_ret_copy");
    let copy_done = format!("{symbol}_ret_copy_done");
    let ret_done = format!("{symbol}_ret_done");
    let ret_off = RESULT_SAVE_OFF;
    let len_off = STATUS_SCRATCH_OFF;
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), cret_off),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&null_label),
        // strlen
        abi::move_register("%v12", "%v9"),
        abi::move_immediate("%v10", "Integer", "0"),
        abi::label(&len_loop),
        abi::load_u8("%v11", "%v12", 0),
        abi::compare_immediate("%v11", "0"),
        abi::branch_eq(&len_done),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v10", "%v10", 1),
        abi::branch(&len_loop),
        abi::label(&len_done),
        abi::store_u64("%v10", abi::stack_pointer(), len_off),
        abi::add_immediate(abi::return_register(), "%v10", 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), len_off),
        abi::store_u64("%v10", abi::RET[1], 0),
        abi::store_u64(abi::RET[1], abi::stack_pointer(), ret_off),
        abi::load_u64("%v11", abi::stack_pointer(), cret_off),
        abi::add_immediate("%v12", abi::RET[1], 8),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_eq(&copy_done),
        abi::load_u8("%v14", "%v11", 0),
        abi::store_u8("%v14", "%v12", 0),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, "%v12", 0),
        // §12.4: returned bytes are validated as UTF-8 at the boundary.
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ret_off),
        abi::add_immediate(abi::return_register(), abi::return_register(), 8),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), len_off),
    ]);
    emit_call_validate_utf8(symbol, encoding_fail, instructions, relocations);
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), ret_off),
        abi::branch(&ret_done),
        // NULL -> empty string [u64 0][nul].
        abi::label(&null_label),
        abi::move_immediate(abi::return_register(), "Integer", "9"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::ZERO, abi::RET[1], 0),
        abi::store_u8(abi::ZERO, abi::RET[1], 8),
        // Store on this path too: the save slot must be authoritative on BOTH
        // paths so a caller can read it from the frame rather than trusting a
        // physical register to survive (plan-50-F).
        abi::store_u64(abi::RET[1], abi::stack_pointer(), ret_off),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::label(&ret_done),
    ]);
}

/// First virtual-register index `emit_link_expr` hands out. The thunk body uses
/// fixed scratch vregs `%v9`..`%v16`; starting the expression's own vregs well
/// past that window guarantees the two name spaces never overlap, so the shared
/// linear-scan allocator sees each expression temporary as an independent value
/// (bug-56).
const LINK_EXPR_VREG_BASE: usize = 64;

/// Emit code computing the boolean/integer value of a `SUCCESS_ON`/`RESULT`
/// expression, reading the native return variable from `[sp + status_off]`.
///
/// Every intermediate is a fresh virtual register (`%vN`, allocated from `vreg`)
/// that the shared linear-scan allocator places and spills — the same discipline
/// as the rest of the thunk (plan-00-G). This replaces the historical scheme that
/// hand-assigned escalating *physical* registers (`x{base}`, `+2`, `+4`) with no
/// bound: a moderately right-nested tree walked `base` up into `x19`, the pinned
/// arena-base register, corrupting the arena program-wide (bug-56). Vregs never
/// touch reserved/callee-saved registers the thunk does not save. Returns the
/// name of the vreg holding the expression's value (0/1 for comparisons).
/// Emit a `SUCCESS_ON`/`RESULT` expression, resolving each `Var(name)` through
/// `offsets` to the frame slot holding that value (plan-50-I).
///
/// `offsets` maps every ABI slot name, and the ABI return name, to its frame
/// offset. An absent name is unreachable: both checkers reject a `Var` naming no
/// slot before codegen runs.
fn emit_link_expr(
    expr: &IrLinkExpr,
    offsets: &HashMap<&str, usize>,
    vreg: &mut usize,
    symbol: &str,
    counter: &mut usize,
    instructions: &mut Vec<CodeInstruction>,
) -> String {
    let dst = format!("%v{vreg}");
    *vreg += 1;
    match expr {
        IrLinkExpr::Int(value) => {
            instructions.push(abi::move_immediate(
                &dst,
                "Integer",
                &(*value as u64).to_string(),
            ));
        }
        // plan-58-B: integer arithmetic, so a `BUFFER … SIZE` can scale a
        // frame/element count to bytes (`frames * channels * 2`) and a `LENGTH`
        // can scale a callee's element count back. Two's-complement wrapping,
        // like every other integer path here; the results are range-gated by
        // their consumers (SIZE against CBUFFER_MAX_BYTES, LENGTH clamped to
        // capacity), so an overflow cannot become an out-of-bounds size.
        IrLinkExpr::Mul(lhs, rhs) | IrLinkExpr::Add(lhs, rhs) | IrLinkExpr::Sub(lhs, rhs) => {
            let lhs_reg = emit_link_expr(lhs, offsets, vreg, symbol, counter, instructions);
            let rhs_reg = emit_link_expr(rhs, offsets, vreg, symbol, counter, instructions);
            instructions.push(match expr {
                IrLinkExpr::Mul(..) => abi::multiply_registers(&dst, &lhs_reg, &rhs_reg),
                IrLinkExpr::Add(..) => abi::add_registers(&dst, &lhs_reg, &rhs_reg),
                _ => abi::subtract_registers(&dst, &lhs_reg, &rhs_reg),
            });
        }
        IrLinkExpr::Var(name) => {
            let off = offsets.get(name.as_str()).copied().unwrap_or_else(|| {
                unreachable!(
                    "LINK expr names slot `{name}`, which verification should have rejected"
                )
            });
            instructions.push(abi::load_u64(&dst, abi::stack_pointer(), off));
        }
        IrLinkExpr::Not(inner) => {
            let inner_reg = emit_link_expr(inner, offsets, vreg, symbol, counter, instructions);
            let id = *counter;
            *counter += 1;
            let set = format!("{symbol}_not{id}_zero");
            let end = format!("{symbol}_not{id}_end");
            instructions.extend([
                abi::compare_immediate(&inner_reg, "0"),
                abi::branch_eq(&set),
                abi::move_immediate(&dst, "Integer", "0"),
                abi::branch(&end),
                abi::label(&set),
                abi::move_immediate(&dst, "Integer", "1"),
                abi::label(&end),
            ]);
        }
        IrLinkExpr::Compare { op, lhs, rhs } => {
            let lhs_reg = emit_link_expr(lhs, offsets, vreg, symbol, counter, instructions);
            let rhs_reg = emit_link_expr(rhs, offsets, vreg, symbol, counter, instructions);
            let id = *counter;
            *counter += 1;
            let end = format!("{symbol}_cmp{id}_end");
            let branch = match op.as_str() {
                "=" => abi::branch_eq(&end),
                "<>" => abi::branch_ne(&end),
                "<" => abi::branch_lt(&end),
                ">" => abi::branch_gt(&end),
                "<=" => abi::branch_le(&end),
                ">=" => abi::branch_ge(&end),
                _ => abi::branch_eq(&end),
            };
            instructions.push(abi::compare_registers(&lhs_reg, &rhs_reg));
            instructions.push(abi::move_immediate(&dst, "Integer", "1"));
            instructions.push(branch);
            instructions.push(abi::move_immediate(&dst, "Integer", "0"));
            instructions.push(abi::label(&end));
        }
        IrLinkExpr::And(lhs, rhs) => {
            // `AND`/`OR` are *logical* connectives: any nonzero operand is true. A
            // bare `Var`/`Int` leaf is an arbitrary integer, so combining with a raw
            // bitwise `and`/`or` would compute e.g. `2 & 1 = 0` and wrongly report
            // two truthy operands as false. Normalize each operand to a canonical
            // `0`/`1` first so bitwise coincides with logical (bug-66). Comparison /
            // logical sub-expressions already yield `0`/`1`, so they pass through
            // unchanged and `AND`/`OR`-of-comparisons stays byte-identical.
            let lhs_reg = emit_link_bool(lhs, offsets, vreg, symbol, counter, instructions);
            let rhs_reg = emit_link_bool(rhs, offsets, vreg, symbol, counter, instructions);
            instructions.push(abi::and_registers(&dst, &lhs_reg, &rhs_reg));
        }
        IrLinkExpr::Or(lhs, rhs) => {
            let lhs_reg = emit_link_bool(lhs, offsets, vreg, symbol, counter, instructions);
            let rhs_reg = emit_link_bool(rhs, offsets, vreg, symbol, counter, instructions);
            instructions.push(abi::or_registers(&dst, &lhs_reg, &rhs_reg));
        }
    }
    dst
}

/// True when `expr` already produces a canonical `0`/`1` truth value, so an
/// `AND`/`OR` operand of this shape needs no normalization. Only the bare `Var`
/// and `Int` leaves carry an arbitrary integer.
fn link_expr_is_boolean(expr: &IrLinkExpr) -> bool {
    matches!(
        expr,
        IrLinkExpr::Not(_)
            | IrLinkExpr::Compare { .. }
            | IrLinkExpr::And(_, _)
            | IrLinkExpr::Or(_, _)
    )
}

/// Emit `expr` and guarantee its value is a canonical `0`/`1` truth value for use
/// as an `AND`/`OR` operand. Sub-expressions that already yield `0`/`1`
/// (comparisons and boolean connectives) pass through unchanged; a bare
/// `Var`/`Int` leaf is normalized with a compare-nonzero so any nonzero value
/// becomes `1` (bug-66).
fn emit_link_bool(
    expr: &IrLinkExpr,
    offsets: &HashMap<&str, usize>,
    vreg: &mut usize,
    symbol: &str,
    counter: &mut usize,
    instructions: &mut Vec<CodeInstruction>,
) -> String {
    let value = emit_link_expr(expr, offsets, vreg, symbol, counter, instructions);
    if link_expr_is_boolean(expr) {
        return value;
    }
    let dst = format!("%v{vreg}");
    *vreg += 1;
    let id = *counter;
    *counter += 1;
    let nonzero = format!("{symbol}_bool{id}_nz");
    let end = format!("{symbol}_bool{id}_end");
    instructions.extend([
        abi::compare_immediate(&value, "0"),
        abi::branch_ne(&nonzero),
        abi::move_immediate(&dst, "Integer", "0"),
        abi::branch(&end),
        abi::label(&nonzero),
        abi::move_immediate(&dst, "Integer", "1"),
        abi::label(&end),
    ]);
    dst
}

/// Emit the sized store for a struct field of `ctype` at `[sp + off]` from `src`
/// (plan-50-E §4.4).
fn store_field(ctype: &str, src: &str, off: usize) -> CodeInstruction {
    match ctype {
        "CInt8" | "CUInt8" | "CBool" | "CByte" => abi::store_u8(src, abi::stack_pointer(), off),
        "CInt16" | "CUInt16" => abi::store_u16(src, abi::stack_pointer(), off),
        "CInt32" | "CUInt32" | "CFloat" => abi::store_u32(src, abi::stack_pointer(), off),
        _ => abi::store_u64(src, abi::stack_pointer(), off),
    }
}

/// Emit the sized load for a struct field of `ctype` at `[sp + off]` into `dst`.
///
/// Every load zero-extends, so a SIGNED narrow field is sign-extended by the
/// caller — otherwise a `CInt32 sections = -1` surfaces as 4294967295, which is
/// exactly bug-238.
fn load_field(ctype: &str, dst: &str, off: usize) -> CodeInstruction {
    match ctype {
        "CInt8" | "CUInt8" | "CBool" | "CByte" => abi::load_u8(dst, abi::stack_pointer(), off),
        "CInt16" | "CUInt16" => abi::load_u16(dst, abi::stack_pointer(), off),
        "CInt32" | "CUInt32" | "CFloat" => abi::load_u32(dst, abi::stack_pointer(), off),
        _ => abi::load_u64(dst, abi::stack_pointer(), off),
    }
}

#[allow(clippy::too_many_arguments)]
/// Write the `BIND IN` fields into a struct slot's buffer before the call
/// (plan-50-E §4.4).
///
/// Only bound fields are written; the buffer is already fully zeroed, so every
/// other field is 0. Reads a wrapper PARAMETER (or an immediate) — there is no
/// record on the input side, hence none of the register-lifetime hazard that
/// `marshal_struct_out` has.
fn marshal_struct_in(
    function: &IrLinkFunction,
    decl: &crate::ir::IrCStruct,
    layout: &crate::ir::CLayout,
    buf_off: usize,
    slot_name: &str,
    symbol: &str,
    param_base: usize,
    param_index: &HashMap<&str, usize>,
    range_fail: &str,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let Some(bind) = function.bind_in.iter().find(|b| b.slot == slot_name) else {
        return Ok(());
    };
    for field in &bind.fields {
        let Some(pos) = decl.fields.iter().position(|f| f.name == field.name) else {
            return Err(format!(
                "LINK function '{}.{}' BIND IN sets unknown field '{}' of CSTRUCT '{}'",
                function.alias, function.name, field.name, decl.name
            ));
        };
        let ctype = decl.fields[pos].ctype.as_str();
        let off = buf_off + layout.offsets[pos];

        // plan-50-F: a `CString` field takes an MFBASIC String and gives C a
        // NUL-terminated copy that lives for the call. Handled before the integer
        // path: it is a pointer, not a value to range-check.
        if ctype == "CString" {
            let Some(param) = &field.param else {
                return Err(format!(
                    "LINK function '{}.{}' BIND IN field '{}' is a CString and must bind a String parameter, not a literal",
                    function.alias, function.name, field.name
                ));
            };
            let Some(&pidx) = param_index.get(param.as_str()) else {
                return Err(format!(
                    "LINK function '{}.{}' BIND IN binds field '{}' to unknown parameter '{param}'",
                    function.alias, function.name, field.name
                ));
            };
            // Allocates (clobbering x0-x17), but nothing is live across it: the
            // source is a frame slot and the destination is sp-relative.
            emit_copy_string_to_cstring(
                symbol,
                param_base + pidx * 8,
                off,
                alloc_fail,
                instructions,
                relocations,
            );
            continue;
        }

        if let Some(literal) = field.literal {
            instructions.push(abi::move_immediate(
                "%v10",
                "Integer",
                &(literal as u64).to_string(),
            ));
        } else if let Some(param) = &field.param {
            let Some(&pidx) = param_index.get(param.as_str()) else {
                return Err(format!(
                    "LINK function '{}.{}' BIND IN binds field '{}' to unknown parameter '{param}'",
                    function.alias, function.name, field.name
                ));
            };
            instructions.push(abi::load_u64(
                "%v10",
                abi::stack_pointer(),
                param_base + pidx * 8,
            ));
        } else {
            return Err(format!(
                "LINK function '{}.{}' BIND IN field '{}' binds neither a parameter nor a literal",
                function.alias, function.name, field.name
            ));
        }

        // A 64-bit MFBASIC Integer must fit the C field's width. Truncating
        // silently is the bug-238 class; the existing CInt32 argument path
        // range-checks for exactly this reason, and a struct field is no different.
        if let Some(bits) = narrow_signed_bits(ctype) {
            instructions.extend([
                abi::shift_left_immediate("%v11", "%v10", (64 - bits) as u8),
                abi::arithmetic_shift_right_immediate("%v11", "%v11", (64 - bits) as u8),
                abi::compare_registers("%v10", "%v11"),
                abi::branch_ne(range_fail),
            ]);
        }
        instructions.push(store_field(ctype, "%v10", off));
    }
    Ok(())
}

/// The bit width of a signed narrow C integer, or `None` when the ctype needs no
/// range check (64-bit, unsigned, float, or pointer).
fn narrow_signed_bits(ctype: &str) -> Option<u32> {
    match ctype {
        "CInt8" => Some(8),
        "CInt16" => Some(16),
        "CInt32" => Some(32),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
/// Build the wrapper's result record from a struct slot's post-call buffer
/// (plan-50-E §4.5).
///
/// **Register lifetime is the whole design here.** `_mfb_arena_alloc` destroys
/// every caller-saved register (`x0`-`x17`) with no survivor set, so the record
/// pointer cannot live in one across any later allocation. Two structural rules
/// make that safe:
///
///  1. allocate the record FIRST and spill its pointer to a stack slot;
///  2. read each field from the `sp`-relative struct buffer, which survives every
///     call by construction, and reload the record pointer per field.
///
/// Reading fields into registers and *then* allocating would lose them all — the
/// `copy-record-register-aliasing` bug in miniature.
fn marshal_struct_out(
    function: &IrLinkFunction,
    decl: &crate::ir::IrCStruct,
    layout: &crate::ir::CLayout,
    buf_off: usize,
    record_fields: &HashMap<String, Vec<(String, String)>>,
    symbol: &str,
    cstr_area: usize,
    cursor_off: usize,
    total_off: usize,
    alloc_fail: &str,
    encoding_fail: &str,
    nan_fail: &str,
    inf_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let record = record_fields.get(&decl.maps_to).ok_or_else(|| {
        format!(
            "LINK function '{}.{}' returns CSTRUCT '{}', whose record '{}' has no field layout",
            function.alias, function.name, decl.name, decl.maps_to
        )
    })?;

    // A record slot is one word at `8*i`, but a `String` field's word is
    // NOT a pointer — it is the offset, relative to the record's own block, of an
    // inlined `{len, bytes, NUL}` sub-block in a trailing data region
    // (`emit_build_inlined_record`). The caller walks that region contiguously to
    // size and copy the record, so the region must exist and be exact.
    //
    // That forces the shape of this routine: every length must be known BEFORE
    // the single allocation, so pass 0 measures, pass 1 sizes and allocates, and
    // pass 2 writes. There is no per-field allocation.
    const REC_OFF: usize = RESULT_SAVE_OFF;
    const ALIGN8_MASK: &str = "18446744073709551608"; // !7u64

    // Resolve each record field to its CSTRUCT field once.
    let mut plan: Vec<(usize, &str, usize)> = Vec::new(); // (rec_idx, ctype, buf offset)
    for (rec_idx, (rname, _)) in record.iter().enumerate() {
        let Some(cpos) = decl.fields.iter().position(|f| f.name == *rname) else {
            return Err(format!(
                "LINK function '{}.{}': record '{}' field '{rname}' has no CSTRUCT field",
                function.alias, function.name, decl.maps_to
            ));
        };
        plan.push((
            rec_idx,
            decl.fields[cpos].ctype.as_str(),
            buf_off + layout.offsets[cpos],
        ));
    }

    // Pass 0: measure. For each CString field stash [char*, len] and validate the
    // bytes as UTF-8 (§12.4). A NULL char* becomes the empty String, len 0.
    let mut cstr_index: HashMap<usize, usize> = HashMap::new();
    for (rec_idx, ctype, off) in &plan {
        if *ctype != "CString" {
            continue;
        }
        let k = cstr_index.len();
        cstr_index.insert(*rec_idx, k);
        let ptr_slot = cstr_area + k * 16;
        let len_slot = ptr_slot + 8;
        let null_label = format!("{symbol}_sf{rec_idx}_null");
        let len_loop = format!("{symbol}_sf{rec_idx}_len");
        let len_done = format!("{symbol}_sf{rec_idx}_len_done");
        let measured = format!("{symbol}_sf{rec_idx}_measured");
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), *off),
            abi::store_u64("%v9", abi::stack_pointer(), ptr_slot),
            abi::move_immediate("%v10", "Integer", "0"),
            abi::store_u64("%v10", abi::stack_pointer(), len_slot),
            abi::compare_immediate("%v9", "0"),
            abi::branch_eq(&null_label),
            // strlen
            abi::move_register("%v12", "%v9"),
            abi::move_immediate("%v10", "Integer", "0"),
            abi::label(&len_loop),
            abi::load_u8("%v11", "%v12", 0),
            abi::compare_immediate("%v11", "0"),
            abi::branch_eq(&len_done),
            abi::add_immediate("%v12", "%v12", 1),
            abi::add_immediate("%v10", "%v10", 1),
            abi::branch(&len_loop),
            abi::label(&len_done),
            abi::store_u64("%v10", abi::stack_pointer(), len_slot),
            // Validate before anything is copied into the record.
            abi::load_u64(abi::return_register(), abi::stack_pointer(), ptr_slot),
            abi::load_u64(abi::ARG[1], abi::stack_pointer(), len_slot),
        ]);
        emit_call_validate_utf8(symbol, encoding_fail, instructions, relocations);
        instructions.extend([
            abi::branch(&measured),
            abi::label(&null_label),
            abi::label(&measured),
        ]);
    }

    // Pass 1: size = 8*n, then each inlined block (8-aligned, `len + 9` bytes).
    let fixed = 8 * record.len();
    instructions.extend([
        abi::move_immediate("%v9", "Integer", &fixed.to_string()),
        abi::store_u64("%v9", abi::stack_pointer(), total_off),
    ]);
    for (rec_idx, ctype, _) in &plan {
        if *ctype != "CString" {
            continue;
        }
        let len_slot = cstr_area + cstr_index[rec_idx] * 16 + 8;
        instructions.extend([
            // total = align8(total)
            abi::load_u64("%v9", abi::stack_pointer(), total_off),
            abi::add_immediate("%v9", "%v9", 7),
            abi::move_immediate("%v10", "Integer", ALIGN8_MASK),
            abi::and_registers("%v9", "%v9", "%v10"),
            // total += len + 9
            abi::load_u64("%v11", abi::stack_pointer(), len_slot),
            abi::add_immediate("%v11", "%v11", 9),
            abi::add_registers("%v9", "%v9", "%v11"),
            abi::store_u64("%v9", abi::stack_pointer(), total_off),
        ]);
    }
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), total_off),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), REC_OFF));

    // Pass 2: write each field. The record pointer is reloaded per field — it did
    // not survive the allocation (`_mfb_arena_alloc` destroys x0-x17).
    instructions.extend([
        abi::move_immediate("%v9", "Integer", &fixed.to_string()),
        abi::store_u64("%v9", abi::stack_pointer(), cursor_off),
    ]);
    for (rec_idx, ctype, off) in &plan {
        if *ctype == "CString" {
            let k = cstr_index[rec_idx];
            let ptr_slot = cstr_area + k * 16;
            let len_slot = ptr_slot + 8;
            let copy_loop = format!("{symbol}_sf{rec_idx}_copy");
            let copy_done = format!("{symbol}_sf{rec_idx}_copy_done");
            instructions.extend([
                // cursor = align8(cursor)
                abi::load_u64("%v9", abi::stack_pointer(), cursor_off),
                abi::add_immediate("%v9", "%v9", 7),
                abi::move_immediate("%v10", "Integer", ALIGN8_MASK),
                abi::and_registers("%v9", "%v9", "%v10"),
                abi::store_u64("%v9", abi::stack_pointer(), cursor_off),
                // record[8*i] = cursor (the block-relative offset)
                abi::load_u64("%v10", abi::stack_pointer(), REC_OFF),
                abi::store_u64("%v9", "%v10", 8 * rec_idx),
                // dst = record + cursor; [dst] = len
                abi::add_registers("%v11", "%v10", "%v9"),
                abi::load_u64("%v10", abi::stack_pointer(), len_slot),
                abi::store_u64("%v10", "%v11", 0),
                // copy `len` bytes to dst+8, then NUL-terminate.
                abi::load_u64("%v12", abi::stack_pointer(), ptr_slot),
                abi::add_immediate("%v13", "%v11", 8),
                abi::move_immediate("%v14", "Integer", "0"),
                abi::label(&copy_loop),
                abi::compare_registers("%v14", "%v10"),
                abi::branch_eq(&copy_done),
                abi::load_u8("%v15", "%v12", 0),
                abi::store_u8("%v15", "%v13", 0),
                abi::add_immediate("%v12", "%v12", 1),
                abi::add_immediate("%v13", "%v13", 1),
                abi::add_immediate("%v14", "%v14", 1),
                abi::branch(&copy_loop),
                abi::label(&copy_done),
                abi::store_u8(abi::ZERO, "%v13", 0),
                // cursor += len + 9
                abi::load_u64("%v9", abi::stack_pointer(), cursor_off),
                abi::load_u64("%v10", abi::stack_pointer(), len_slot),
                abi::add_immediate("%v10", "%v10", 9),
                abi::add_registers("%v9", "%v9", "%v10"),
                abi::store_u64("%v9", abi::stack_pointer(), cursor_off),
            ]);
            continue;
        }
        instructions.push(load_field(ctype, "%v9", *off));
        match *ctype {
            // Every load zero-extends, so a signed narrow field must be
            // sign-extended or -1 surfaces as its unsigned reading (bug-238).
            "CInt8" | "CInt16" | "CInt32" => {
                let bits = narrow_signed_bits(ctype).unwrap();
                instructions.extend([
                    abi::shift_left_immediate("%v9", "%v9", (64 - bits) as u8),
                    abi::arithmetic_shift_right_immediate("%v9", "%v9", (64 - bits) as u8),
                ]);
            }
            "CBool" => {
                let set = format!("{symbol}_sf{rec_idx}_true");
                let end = format!("{symbol}_sf{rec_idx}_end");
                instructions.extend([
                    abi::compare_immediate("%v9", "0"),
                    abi::branch_ne(&set),
                    abi::move_immediate("%v9", "Integer", "0"),
                    abi::branch(&end),
                    abi::label(&set),
                    abi::move_immediate("%v9", "Integer", "1"),
                    abi::label(&end),
                ]);
            }
            "CDouble" => {
                // An MFBASIC Float is always finite (§3), so reject NaN/Inf at the
                // boundary exactly as the CDouble return path does.
                let finite = format!("{symbol}_sf{rec_idx}_finite");
                instructions.extend([
                    abi::move_immediate("%v10", "Integer", "9218868437227405312"),
                    abi::and_registers("%v11", "%v9", "%v10"),
                    abi::compare_registers("%v11", "%v10"),
                    abi::branch_ne(&finite),
                    abi::move_immediate("%v12", "Integer", "4503599627370495"),
                    abi::and_registers("%v13", "%v9", "%v12"),
                    abi::compare_immediate("%v13", "0"),
                    abi::branch_eq(inf_fail),
                    abi::branch(nan_fail),
                    abi::label(&finite),
                ]);
            }
            _ => {}
        }
        instructions.extend([
            abi::load_u64("%v10", abi::stack_pointer(), REC_OFF),
            abi::store_u64("%v9", "%v10", 8 * rec_idx),
        ]);
    }
    instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        REC_OFF,
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{
        abi_ctype_valid_as_argument, abi_ctype_valid_as_return, IrAbiSlot, IrLinkFunction,
    };

    /// plan-59-A Phase 3: a native `LINK` resource must never reach the
    /// buffer-free path in `emit_resource_block_reclaim`.
    ///
    /// Since plan-59-A every native resource is an 80-byte record, so it now flows
    /// through the same drop-reclamation as a built-in `File`. Words 24..72 of a
    /// native record are zeroed by the thunk and are NOT buffer pointers; handing
    /// them to `arena_free` would free addresses the resource never owned.
    ///
    /// What keeps that from happening is `resource_uses_io_buffers`, which is a
    /// bare name comparison against `"File"`. That makes the guarantee POSITIONAL
    /// — true only because no other type is spelled `File` — and plan-59-A's Open
    /// Decision required it be pinned by a test rather than asserted, precisely
    /// because a positional fact is the kind that drifts.
    ///
    /// This test fails the moment a second type is given I/O buffers, which is the
    /// signal to re-check whether native records can reach that path.
    #[test]
    fn only_the_builtin_file_resource_uses_io_buffers() {
        // The one type that owns the two fixed-capacity I/O buffers.
        assert!(CodeBuilder::resource_uses_io_buffers("File"));
        // A `STATE`-carrying spelling is still the same base type.
        assert!(CodeBuilder::resource_uses_io_buffers("File STATE Cursor"));

        // Every resource type a native `LINK` block declares in-tree, plus the
        // other built-in resources. None may take the buffer-free path.
        for type_ in [
            "Db",
            "Stmt",
            "SoundFile",
            "SoundFile STATE FileInfo",
            "Socket",
            "Listener",
            "UdpSocket",
            "AudioInput",
            "AudioOutput",
        ] {
            assert!(
                !CodeBuilder::resource_uses_io_buffers(type_),
                "{type_} must not take the I/O-buffer free path: its record's \
                 words 24..72 are not buffer pointers"
            );
        }
    }

    /// bug-296: a LINK thunk calls a real C function, so its arguments follow the
    /// target's EXTERNAL C ABI. SysV x86-64 passes six integer arguments in
    /// registers; the backend's `CALL_ARGS` extends the list with rax/rbp for
    /// arguments 7 and 8, which is sound for the compiler's own calls but hands an
    /// external callee two registers it never reads -- it takes those from the
    /// stack, so a >=7-integer-slot native function was called with garbage
    /// trailing arguments, silently and with no diagnostic. aarch64 and riscv64
    /// have 8 real argument registers and are unaffected.
    #[test]
    fn seven_integer_slots_are_rejected_on_x86_and_accepted_on_aarch64() {
        let seven_int_slots = |count: usize| IrLinkFunction {
            alias: "lib".to_string(),
            name: "seven".to_string(),
            library: "demo".to_string(),
            symbol: "demo_seven".to_string(),
            // Each ABI slot is sourced from a wrapper parameter of the same name.
            params: (0..count)
                .map(|i| (format!("a{i}"), "Integer".to_string()))
                .collect(),
            return_type: "Integer".to_string(),
            return_resource: false,
            return_state_type: None,
            abi_slots: (0..count)
                .map(|i| IrAbiSlot {
                    name: format!("a{i}"),
                    ctype: "CInt64".to_string(),
                    direction: crate::ir::AbiDirection::In,
                })
                .collect(),
            abi_return_name: "return".to_string(),
            abi_return_ctype: "CInt64".to_string(),
            consts: vec![],
            bind_in: vec![],
            bind_state: None,
            bind_state_resource: None,
            success_on: None,
            result: None,
            free: None,
            buffers: vec![],
            result_length: None,
        };
        let lower = |count: usize| {
            lower_link_thunk(
                &seven_int_slots(count),
                &[],
                &HashMap::new(),
                TEST_THUNK_CONTEXT,
                &HashSet::new(),
            )
        };

        // x86-64: six is the SysV limit, so six lowers and seven is refused rather
        // than emitting a call that passes arguments the callee never reads.
        mir::set_backend(&crate::arch::x86_64::backend::X86_64_BACKEND);
        assert!(
            lower(6).is_ok(),
            "six integer slots must still lower on x86"
        );
        let err = match lower(7) {
            Ok(_) => panic!("seven integer slots must be refused on x86"),
            Err(err) => err,
        };
        assert!(
            err.contains("integer ABI slots") && err.contains("seven"),
            "unexpected error: {err}"
        );

        // aarch64 has eight real argument registers, so the same function is fine.
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        assert!(
            lower(7).is_ok(),
            "seven integer slots are within AAPCS64's eight argument registers"
        );
        assert!(lower(8).is_ok(), "eight is AAPCS64's limit");
    }

    /// The `maxBuffer` ceiling these tests lower against: the project.json
    /// default of 64 MiB. Named rather than inlined so a test asserting the gate
    /// reads against the same number the default produces.
    const TEST_THUNK_CONTEXT: ThunkContext = ThunkContext {
        index: 0,
        globals_base: 0,
        free_slot: None,
        // The project.json `maxBuffer` default, so a test asserting the size gate
        // reads against the same number a project with no `maxBuffer` gets.
        max_buffer_bytes: 64 * 1024 * 1024,
    };

    /// Every ctype the allow-list accepts must reach a real marshaling arm.
    ///
    /// plan-50-A closed the slot-ctype namespace and turned `emit_return_passthrough`'s
    /// default arm into an `Err`. The allow-list is hand-written (Rust cannot
    /// enumerate match arms), so this walks every accepted name through
    /// `lower_link_thunk` and asserts none of them lands on that arm. Without it,
    /// adding a ctype to the list without a marshaling arm would only fail at the
    /// first binding that used it.
    #[test]
    fn every_known_ctype_lowers() {
        // Lowering reads the active backend for register/instruction shapes; the
        // ctype set under test is backend-independent, so any backend serves.
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);

        // The full accepted set. `ir::link::tests::ctype_list_is_exhaustive` holds
        // this in sync with `abi_slot_ctype_is_known`, so a name added to the
        // authority without an arm here fails there first.
        const CTYPES: &[&str] = &[
            "CPtr", "CString", "CBuffer", "CInt8", "CInt16", "CInt32", "CInt64", "CUInt8",
            "CUInt16", "CUInt32", "CUInt64", "CBool", "CByte", "CFloat", "CDouble", "CVoid",
        ];

        // `CBuffer` is covered by neither loop below, because it fits neither
        // shape: it is invalid as the ABI return (loop 1) and invalid as an IN
        // slot (loop 2). Its only legal position is an OUT slot with a `BUFFER …
        // SIZE` clause, so it gets its own case at the end of this test.
        //
        // plan-58-A shipped that case as an assertion that CBuffer must FAIL to
        // lower, deliberately rigged to break the moment plan-58-B landed the
        // marshaler — which is exactly what happened, rather than the ctype
        // quietly staying uncovered behind a silent filter.
        const OWN_SHAPE_ONLY: &[&str] = &["CBuffer"];

        // Every ctype valid as an ABI *return* must reach a return arm. This is the
        // arm that can `Err`, and it is how the guard caught `CString` having no
        // return meaning at all (a `char *` return is `CPtr` + a `String` wrapper).
        for ctype in CTYPES
            .iter()
            .filter(|c| abi_ctype_valid_as_return(c) && !OWN_SHAPE_ONLY.contains(c))
        {
            let returns_value = *ctype != "CVoid";
            let function = IrLinkFunction {
                alias: "lib".to_string(),
                name: format!("ret_{ctype}"),
                library: "demo".to_string(),
                symbol: "demo_f".to_string(),
                params: vec![],
                // A `CPtr` return with an `Integer` wrapper takes the raw path; the
                // `String` copy-out path is covered by the sqlite3 runtime tests.
                return_type: if returns_value { "Integer" } else { "Nothing" }.to_string(),
                return_resource: false,
                return_state_type: None,
                abi_slots: vec![],
                abi_return_name: if returns_value { "return" } else { "status" }.to_string(),
                abi_return_ctype: (*ctype).to_string(),
                consts: vec![],
                bind_in: vec![],
                bind_state: None,
                bind_state_resource: None,
                success_on: None,
                result: None,
                free: None,
                buffers: vec![],
                result_length: None,
            };
            let lowered = lower_link_thunk(
                &function,
                &[],
                &HashMap::new(),
                TEST_THUNK_CONTEXT,
                &HashSet::new(),
            );
            assert!(
                lowered.is_ok(),
                "accepted return ctype {ctype} does not lower: {:?}",
                lowered.err()
            );
        }

        // Every ctype valid as an ABI *argument* must stage without error. Pinned
        // with CONST so the slot needs no wrapper parameter.
        for ctype in CTYPES.iter().filter(|c| abi_ctype_valid_as_argument(c)) {
            let function = IrLinkFunction {
                alias: "lib".to_string(),
                name: format!("arg_{ctype}"),
                library: "demo".to_string(),
                symbol: "demo_f".to_string(),
                params: vec![],
                return_type: "Nothing".to_string(),
                return_resource: false,
                return_state_type: None,
                abi_slots: vec![IrAbiSlot {
                    name: "pinned".to_string(),
                    ctype: (*ctype).to_string(),
                    direction: crate::ir::AbiDirection::In,
                }],
                abi_return_name: "status".to_string(),
                abi_return_ctype: "CInt32".to_string(),
                consts: vec![("pinned".to_string(), 0)],
                bind_in: vec![],
                bind_state: None,
                bind_state_resource: None,
                success_on: None,
                result: None,
                free: None,
                buffers: vec![],
                result_length: None,
            };
            let lowered = lower_link_thunk(
                &function,
                &[],
                &HashMap::new(),
                TEST_THUNK_CONTEXT,
                &HashSet::new(),
            );
            assert!(
                lowered.is_ok(),
                "accepted argument ctype {ctype} does not lower: {:?}",
                lowered.err()
            );
        }

        // plan-58-B: a well-formed `OUT CBuffer` — one that passes every
        // `check_buffer_slots` rule — must LOWER. The runtime proof that the bytes
        // are right lives in `tests/rt-behavior/native/native-cbuffer-read-rt`;
        // this is the drift guard that keeps the ctype covered at all.
        for ctype in OWN_SHAPE_ONLY {
            let function = IrLinkFunction {
                alias: "lib".to_string(),
                name: format!("out_{ctype}"),
                library: "demo".to_string(),
                symbol: "demo_f".to_string(),
                params: vec![("n".to_string(), "Integer".to_string())],
                return_type: crate::ir::BYTE_LIST_TYPE.to_string(),
                return_resource: false,
                return_state_type: None,
                abi_slots: vec![IrAbiSlot {
                    name: "buf".to_string(),
                    ctype: (*ctype).to_string(),
                    direction: crate::ir::AbiDirection::Out,
                }],
                abi_return_name: "status".to_string(),
                abi_return_ctype: "CInt32".to_string(),
                consts: vec![],
                bind_in: vec![],
                bind_state: None,
                bind_state_resource: None,
                success_on: None,
                result: Some(crate::ir::IrLinkExpr::Var("buf".to_string())),
                free: None,
                buffers: vec![crate::ir::IrBuffer {
                    slot: "buf".to_string(),
                    size: crate::ir::IrLinkExpr::Var("n".to_string()),
                }],
                result_length: Some(crate::ir::IrLinkExpr::Var("status".to_string())),
            };
            let lowered = lower_link_thunk(
                &function,
                &[],
                &HashMap::new(),
                TEST_THUNK_CONTEXT,
                &HashSet::new(),
            );
            assert!(
                lowered.is_ok(),
                "accepted OUT-slot ctype {ctype} does not lower: {:?}",
                lowered.err()
            );
        }
    }
}
