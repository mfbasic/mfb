//! Purely-syntactic `path*` string `fs` code generation: the call-site path lowering, the five abi_inline_self members, and the standalone pathJoin runtime helper.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;

impl CodeBuilder<'_> {
    /// Emit the shared trailing-`/` trim loop (bug-331 §J): walk `length` down
    /// while the last byte (`bytes[length-1]`) is `/` (47), stopping at length 1.
    /// `cursor`/`byte` are scratch; the labels are created by the caller so each
    /// path op keeps its own `fs_path_*_trim_{loop,done}` names in the goldens.
    fn emit_trailing_slash_trim(
        &mut self,
        length: impl Into<Operand>,
        bytes: impl Into<Operand>,
        cursor: impl Into<Operand>,
        byte: impl Into<Operand>,
        trim_loop: &str,
        trim_done: &str,
    ) {
        let length = length.into();
        let cursor = cursor.into();
        let byte = byte.into();
        self.emit(abi::label(trim_loop));
        self.emit(abi::compare_immediate(length.clone(), "1"));
        self.emit(abi::branch_le(trim_done));
        self.emit(abi::add_registers(cursor.clone(), bytes, length.clone()));
        self.emit(abi::subtract_immediate(cursor.clone(), cursor.clone(), 1));
        self.emit(abi::load_u8(byte.clone(), cursor.clone(), 0));
        self.emit(abi::compare_immediate(byte.clone(), "47"));
        self.emit(abi::branch_ne(trim_done));
        self.emit(abi::subtract_immediate(length.clone(), length.clone(), 1));
        self.emit(abi::branch(trim_loop));
        self.emit(abi::label(trim_done));
    }

    pub(crate) fn lower_fs_path_call(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Result<Option<ValueResult>, String> {
        let result = match target {
            "fs.pathJoin" if args.len() == 1 => self.lower_fs_path_join(&args[0])?,
            "fs.pathBaseName" if args.len() == 1 => self.lower_fs_path_base_name(&args[0])?,
            "fs.pathDirName" if args.len() == 1 => self.lower_fs_path_dir_name(&args[0])?,
            "fs.pathExtension" if args.len() == 1 => self.lower_fs_path_extension(&args[0])?,
            "fs.pathNormalize" if args.len() == 1 => self.lower_fs_path_normalize(&args[0])?,
            _ => return Ok(None),
        };
        Ok(Some(result))
    }

    /// Join path components with the host separator following normal path-join
    /// rules: empty components are skipped, a component that is absolute (begins
    /// with the separator) discards everything joined so far, and exactly one
    /// separator is inserted between components without producing duplicates.
    ///
    /// The work is delegated to the shared [`FS_PATH_JOIN_SYMBOL`] runtime helper
    /// so that root native code and imported-package binary_repr lower `pathJoin`
    /// identically.
    fn lower_fs_path_join(&mut self, parts: &NirValue) -> Result<ValueResult, String> {
        let parts = self.lower_value(parts)?;
        if list_element_type(&parts.type_).as_deref() != Some("String") {
            return Err(format!(
                "fs.pathJoin parts must be List OF String, got {}",
                parts.type_
            ));
        }
        let parts_slot = self.spill_to_slot("fs_path_join_parts", &parts.location);
        let alloc_ok = self.label("fs_path_join_alloc_ok");
        // plan-71-C Family-1a: the parts pointer is arg 0 of the fs_path_join call → `%arg0`.
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            parts_slot,
        ));
        self.emit(abi::branch_link(FS_PATH_JOIN_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: FS_PATH_JOIN_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        let result = self.allocate_register()?;
        self.emit(abi::move_register(&result, RESULT_VALUE_REGISTER));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: Operand::from(result.render()),
            text: "fs.pathJoin".to_string(),
        })
    }

    fn lower_fs_path_base_name(&mut self, path: &NirValue) -> Result<ValueResult, String> {
        let path = self.lower_value(path)?;
        self.require_string("fs.pathBaseName path", &path)?;
        let path_slot = self.spill_to_slot("fs_path_base_name_path", &path.location);
        let whole_root = self.label("fs_path_base_name_whole_root");
        let trim_loop = self.label("fs_path_base_name_trim_loop");
        let trim_done = self.label("fs_path_base_name_trim_done");
        let scan_start = self.label("fs_path_base_name_scan_start");
        let scan_loop = self.label("fs_path_base_name_scan_loop");
        let found_slash = self.label("fs_path_base_name_found_slash");
        let range_ready = self.label("fs_path_base_name_range_ready");
        let path_ptr = self.temporary_vreg();
        let length = self.temporary_vreg();
        let bytes = self.temporary_vreg();
        let cursor = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let index = self.temporary_vreg();
        let start = self.temporary_vreg();
        let span = self.temporary_vreg();

        self.emit(abi::load_u64(&path_ptr, abi::stack_pointer(), path_slot));
        self.emit(abi::load_u64(&length, &path_ptr, 0));
        self.emit(abi::add_immediate(&bytes, &path_ptr, 8));

        self.emit_trailing_slash_trim(&length, &bytes, &cursor, &byte, &trim_loop, &trim_done);
        // An all-separator path (for example "/", "//", "///") collapses under the
        // trailing-slash trim to a lone remaining "/"; route it to the root shortcut
        // so the result is "/" rather than an empty span. Gating this on the trimmed
        // length (not the original) is what distinguishes it from paths like "/a".
        self.emit(abi::compare_immediate(&length, "1"));
        self.emit(abi::branch_ne(&scan_start));
        self.emit(abi::load_u8(&cursor, &bytes, 0));
        self.emit(abi::compare_immediate(&cursor, "47"));
        self.emit(abi::branch_eq(&whole_root));
        self.emit(abi::label(&scan_start));
        self.emit(abi::move_register(&index, &length));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_immediate(&index, "0"));
        self.emit(abi::branch_eq(&range_ready));
        self.emit(abi::subtract_immediate(&index, &index, 1));
        self.emit(abi::add_registers(&cursor, &bytes, &index));
        self.emit(abi::load_u8(&byte, &cursor, 0));
        self.emit(abi::compare_immediate(&byte, "47"));
        self.emit(abi::branch_eq(&found_slash));
        self.emit(abi::branch(&scan_loop));

        self.emit(abi::label(&found_slash));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&range_ready));

        self.emit(abi::label(&whole_root));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&length, "Integer", "1"));
        self.emit(abi::label(&range_ready));
        self.emit(abi::add_registers(&start, &bytes, &index));
        self.emit(abi::subtract_registers(&span, &length, &index));
        let result = self.emit_materialize_string_from_bytes(&start, &span)?;
        Ok(ValueResult {
            type_: "String".to_string(),
            location: Operand::from(result.render()),
            text: "fs.pathBaseName".to_string(),
        })
    }

    fn lower_fs_path_dir_name(&mut self, path: &NirValue) -> Result<ValueResult, String> {
        let path = self.lower_value(path)?;
        self.require_string("fs.pathDirName path", &path)?;
        let path_slot = self.spill_to_slot("fs_path_dir_name_path", &path.location);
        let dot = self.label("fs_path_dir_name_dot");
        let root = self.label("fs_path_dir_name_root");
        let trim_loop = self.label("fs_path_dir_name_trim_loop");
        let trim_done = self.label("fs_path_dir_name_trim_done");
        let scan_loop = self.label("fs_path_dir_name_scan_loop");
        let found_slash = self.label("fs_path_dir_name_found_slash");
        let materialize = self.label("fs_path_dir_name_materialize");
        let path_ptr = self.temporary_vreg();
        let length = self.temporary_vreg();
        let bytes = self.temporary_vreg();
        let cursor = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let index = self.temporary_vreg();
        let start = self.temporary_vreg();
        let constant_ptr = self.temporary_vreg();

        self.emit(abi::load_u64(&path_ptr, abi::stack_pointer(), path_slot));
        self.emit(abi::load_u64(&length, &path_ptr, 0));
        self.emit(abi::add_immediate(&bytes, &path_ptr, 8));
        self.emit(abi::compare_immediate(&length, "0"));
        self.emit(abi::branch_eq(&dot));
        self.emit(abi::compare_immediate(&length, "1"));
        self.emit(abi::branch_ne(&trim_loop));
        self.emit(abi::load_u8(&cursor, &bytes, 0));
        self.emit(abi::compare_immediate(&cursor, "47"));
        self.emit(abi::branch_eq(&root));

        self.emit_trailing_slash_trim(&length, &bytes, &cursor, &byte, &trim_loop, &trim_done);
        self.emit(abi::move_register(&index, &length));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_immediate(&index, "0"));
        self.emit(abi::branch_eq(&dot));
        self.emit(abi::subtract_immediate(&index, &index, 1));
        self.emit(abi::add_registers(&cursor, &bytes, &index));
        self.emit(abi::load_u8(&byte, &cursor, 0));
        self.emit(abi::compare_immediate(&byte, "47"));
        self.emit(abi::branch_eq(&found_slash));
        self.emit(abi::branch(&scan_loop));

        self.emit(abi::label(&found_slash));
        self.emit(abi::compare_immediate(&index, "0"));
        self.emit(abi::branch_eq(&root));
        self.emit(abi::move_register(&length, &index));
        self.emit(abi::branch(&materialize));

        self.emit(abi::label(&dot));
        let dot_register = self.load_string_constant(".")?;
        self.emit(abi::move_register(&constant_ptr, &dot_register));
        let done_constant = self.label("fs_path_dir_name_done_constant");
        self.emit(abi::branch(&done_constant));

        self.emit(abi::label(&root));
        let slash_register = self.load_string_constant("/")?;
        self.emit(abi::move_register(&constant_ptr, &slash_register));
        self.emit(abi::branch(&done_constant));

        self.emit(abi::label(&materialize));
        self.emit(abi::move_register(&start, &bytes));
        let result = self.emit_materialize_string_from_bytes(&start, &length)?;
        let final_slot = self.allocate_stack_object("fs_path_dir_name_result", 8);
        self.emit(abi::store_u64(&result, abi::stack_pointer(), final_slot));
        let done = self.label("fs_path_dir_name_done");
        self.emit(abi::branch(&done));
        self.emit(abi::label(&done_constant));
        self.emit(abi::store_u64(
            &constant_ptr,
            abi::stack_pointer(),
            final_slot,
        ));
        self.emit(abi::label(&done));
        let out = self.allocate_register()?;
        self.emit(abi::load_u64(&out, abi::stack_pointer(), final_slot));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: Operand::from(out.render()),
            text: "fs.pathDirName".to_string(),
        })
    }

    fn lower_fs_path_extension(&mut self, path: &NirValue) -> Result<ValueResult, String> {
        let path = self.lower_value(path)?;
        self.require_string("fs.pathExtension path", &path)?;
        let path_slot = self.spill_to_slot("fs_path_extension_path", &path.location);
        let empty = self.label("fs_path_extension_empty");
        let trim_loop = self.label("fs_path_extension_trim_loop");
        let trim_done = self.label("fs_path_extension_trim_done");
        let scan_loop = self.label("fs_path_extension_scan_loop");
        let found_dot = self.label("fs_path_extension_found_dot");
        let materialize = self.label("fs_path_extension_materialize");
        let done = self.label("fs_path_extension_done");
        let path_ptr = self.temporary_vreg();
        let length = self.temporary_vreg();
        let bytes = self.temporary_vreg();
        let cursor = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let index = self.temporary_vreg();
        let start = self.temporary_vreg();
        let span = self.temporary_vreg();

        self.emit(abi::load_u64(&path_ptr, abi::stack_pointer(), path_slot));
        self.emit(abi::load_u64(&length, &path_ptr, 0));
        self.emit(abi::add_immediate(&bytes, &path_ptr, 8));
        self.emit(abi::label(&trim_loop));
        self.emit(abi::compare_immediate(&length, "0"));
        self.emit(abi::branch_eq(&empty));
        self.emit(abi::add_registers(&cursor, &bytes, &length));
        self.emit(abi::subtract_immediate(&cursor, &cursor, 1));
        self.emit(abi::load_u8(&byte, &cursor, 0));
        self.emit(abi::compare_immediate(&byte, "47"));
        self.emit(abi::branch_ne(&trim_done));
        self.emit(abi::subtract_immediate(&length, &length, 1));
        self.emit(abi::branch(&trim_loop));
        self.emit(abi::label(&trim_done));
        self.emit(abi::move_register(&index, &length));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_immediate(&index, "0"));
        self.emit(abi::branch_eq(&empty));
        self.emit(abi::subtract_immediate(&index, &index, 1));
        self.emit(abi::add_registers(&cursor, &bytes, &index));
        self.emit(abi::load_u8(&byte, &cursor, 0));
        self.emit(abi::compare_immediate(&byte, "47"));
        self.emit(abi::branch_eq(&empty));
        self.emit(abi::compare_immediate(&byte, "46"));
        self.emit(abi::branch_eq(&found_dot));
        self.emit(abi::branch(&scan_loop));
        self.emit(abi::label(&found_dot));
        self.emit(abi::add_registers(&start, &bytes, &index));
        self.emit(abi::subtract_registers(&span, &length, &index));
        self.emit(abi::branch(&materialize));
        self.emit(abi::label(&empty));
        self.emit(abi::move_register(&start, &bytes));
        self.emit(abi::move_immediate(&span, "Integer", "0"));
        self.emit(abi::label(&materialize));
        let result = self.emit_materialize_string_from_bytes(&start, &span)?;
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: Operand::from(result.render()),
            text: "fs.pathExtension".to_string(),
        })
    }

    fn lower_fs_path_normalize(&mut self, path: &NirValue) -> Result<ValueResult, String> {
        let path = self.lower_value(path)?;
        self.require_string("fs.pathNormalize path", &path)?;
        let path_slot = self.spill_to_slot("fs_path_normalize_path", &path.location);
        let result_slot = self.allocate_stack_object("fs_path_normalize_result", 8);
        let out_len_slot = self.allocate_stack_object("fs_path_normalize_out_len", 8);
        let input_index_slot = self.allocate_stack_object("fs_path_normalize_input_index", 8);
        let component_start_slot =
            self.allocate_stack_object("fs_path_normalize_component_start", 8);
        let component_len_slot = self.allocate_stack_object("fs_path_normalize_component_len", 8);

        let alloc_ok = self.label("fs_path_normalize_alloc_ok");
        let empty_path = self.label("fs_path_normalize_empty_path");
        let initial_relative = self.label("fs_path_normalize_initial_relative");
        let skip_initial_slashes = self.label("fs_path_normalize_skip_initial_slashes");
        let component_loop = self.label("fs_path_normalize_component_loop");
        let skip_slashes = self.label("fs_path_normalize_skip_slashes");
        let scan_component = self.label("fs_path_normalize_scan_component");
        let scan_component_loop = self.label("fs_path_normalize_scan_component_loop");
        let component_ready = self.label("fs_path_normalize_component_ready");
        let check_dot_dot = self.label("fs_path_normalize_check_dot_dot");
        let maybe_dot_dot = self.label("fs_path_normalize_maybe_dot_dot");
        let handle_dot_dot = self.label("fs_path_normalize_handle_dot_dot");
        let append_component = self.label("fs_path_normalize_append_component");
        let append_separator = self.label("fs_path_normalize_append_separator");
        let append_copy_loop = self.label("fs_path_normalize_append_copy_loop");
        let append_copy_done = self.label("fs_path_normalize_append_copy_done");
        let previous_scan = self.label("fs_path_normalize_previous_scan");
        let previous_ready = self.label("fs_path_normalize_previous_ready");
        let append_dot_dot = self.label("fs_path_normalize_append_dot_dot");
        let pop_previous = self.label("fs_path_normalize_pop_previous");
        let leading_component = self.label("fs_path_normalize_leading_component");
        let pop_scan = self.label("fs_path_normalize_pop_scan");
        let pop_store = self.label("fs_path_normalize_pop_store");
        let finish = self.label("fs_path_normalize_finish");
        let finish_nonempty = self.label("fs_path_normalize_finish_nonempty");
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();

        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), path_slot));
        self.emit(abi::load_u64(&scratch10, &scratch9, 0));
        self.emit(abi::add_immediate(&scratch11, &scratch9, 8));
        // Buffer = 8-byte header + normalized content + 1 NUL. The normalized output
        // is never longer than the input, so `length + 9` suffices for every non-empty
        // path. The one exception is the empty input: the `.` fallback (see `finish`)
        // manufactures a 1-byte content from a 0-byte input, so its NUL would land at
        // offset 9 -- one past a `length + 9` request. Reserve `length + 10` so that
        // fallback's terminator stays in-bounds without relying on arena size rounding.
        // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not return_register().
        self.emit(abi::add_immediate(abi::c_arg(0), &scratch10, 10));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), path_slot));
        self.emit(abi::load_u64(&scratch10, &scratch9, 0));
        self.emit(abi::add_immediate(&scratch11, &scratch9, 8));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::store_u64(abi::ZERO, abi::mfb_return(1), 0));
        self.emit(abi::store_u8(abi::ZERO, abi::mfb_return(1), 8));
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            out_len_slot,
        ));
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            input_index_slot,
        ));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_eq(&empty_path));
        self.emit(abi::load_u8(&scratch12, &scratch11, 0));
        self.emit(abi::compare_immediate(&scratch12, "47"));
        self.emit(abi::branch_ne(&initial_relative));
        self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), result_slot));
        self.emit(abi::move_immediate(&scratch12, "Byte", "47"));
        self.emit(abi::store_u8(&scratch12, &scratch13, 8));
        self.emit(abi::move_immediate(&scratch12, "Integer", "1"));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            out_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            input_index_slot,
        ));
        self.emit(abi::label(&skip_initial_slashes));
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            input_index_slot,
        ));
        self.emit(abi::compare_registers(&scratch14, &scratch10));
        self.emit(abi::branch_ge(&component_loop));
        self.emit(abi::add_registers(&scratch15, &scratch11, &scratch14));
        self.emit(abi::load_u8(&scratch16, &scratch15, 0));
        self.emit(abi::compare_immediate(&scratch16, "47"));
        self.emit(abi::branch_ne(&component_loop));
        self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            input_index_slot,
        ));
        self.emit(abi::branch(&skip_initial_slashes));

        self.emit(abi::label(&initial_relative));
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            input_index_slot,
        ));

        self.emit(abi::label(&component_loop));
        self.emit(abi::label(&skip_slashes));
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            input_index_slot,
        ));
        self.emit(abi::compare_registers(&scratch14, &scratch10));
        self.emit(abi::branch_ge(&finish));
        self.emit(abi::add_registers(&scratch15, &scratch11, &scratch14));
        self.emit(abi::load_u8(&scratch16, &scratch15, 0));
        self.emit(abi::compare_immediate(&scratch16, "47"));
        self.emit(abi::branch_ne(&scan_component));
        self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            input_index_slot,
        ));
        self.emit(abi::branch(&skip_slashes));

        self.emit(abi::label(&scan_component));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            component_start_slot,
        ));
        self.emit(abi::label(&scan_component_loop));
        self.emit(abi::compare_registers(&scratch14, &scratch10));
        self.emit(abi::branch_ge(&component_ready));
        self.emit(abi::add_registers(&scratch15, &scratch11, &scratch14));
        self.emit(abi::load_u8(&scratch16, &scratch15, 0));
        self.emit(abi::compare_immediate(&scratch16, "47"));
        self.emit(abi::branch_eq(&component_ready));
        self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        self.emit(abi::branch(&scan_component_loop));

        self.emit(abi::label(&component_ready));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            input_index_slot,
        ));
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            component_start_slot,
        ));
        self.emit(abi::subtract_registers(&scratch16, &scratch14, &scratch15));
        self.emit(abi::store_u64(
            &scratch16,
            abi::stack_pointer(),
            component_len_slot,
        ));
        self.emit(abi::compare_immediate(&scratch16, "1"));
        self.emit(abi::branch_ne(&check_dot_dot));
        self.emit(abi::add_registers(&scratch17, &scratch11, &scratch15));
        self.emit(abi::load_u8(&scratch8, &scratch17, 0));
        self.emit(abi::compare_immediate(&scratch8, "46"));
        self.emit(abi::branch_eq(&component_loop));
        self.emit(abi::branch(&append_component));

        self.emit(abi::label(&check_dot_dot));
        self.emit(abi::compare_immediate(&scratch16, "2"));
        self.emit(abi::branch_ne(&append_component));
        self.emit(abi::add_registers(&scratch17, &scratch11, &scratch15));
        self.emit(abi::load_u8(&scratch8, &scratch17, 0));
        self.emit(abi::compare_immediate(&scratch8, "46"));
        self.emit(abi::branch_ne(&append_component));
        self.emit(abi::load_u8(&scratch8, &scratch17, 1));
        self.emit(abi::compare_immediate(&scratch8, "46"));
        self.emit(abi::branch_eq(&handle_dot_dot));
        self.emit(abi::branch(&append_component));

        self.emit(abi::label(&handle_dot_dot));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), out_len_slot));
        self.emit(abi::compare_immediate(&scratch8, "0"));
        self.emit(abi::branch_eq(&append_dot_dot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch12, &scratch9, 8));
        self.emit(abi::compare_immediate(&scratch8, "1"));
        self.emit(abi::branch_ne(&maybe_dot_dot));
        self.emit(abi::load_u8(&scratch13, &scratch12, 0));
        self.emit(abi::compare_immediate(&scratch13, "47"));
        self.emit(abi::branch_eq(&component_loop));
        self.emit(abi::label(&maybe_dot_dot));
        self.emit(abi::move_register(&scratch13, &scratch8));
        self.emit(abi::label(&previous_scan));
        self.emit(abi::compare_immediate(&scratch13, "0"));
        self.emit(abi::branch_eq(&previous_ready));
        self.emit(abi::subtract_immediate(&scratch13, &scratch13, 1));
        self.emit(abi::add_registers(&scratch14, &scratch12, &scratch13));
        self.emit(abi::load_u8(&scratch15, &scratch14, 0));
        self.emit(abi::compare_immediate(&scratch15, "47"));
        self.emit(abi::branch_eq(&previous_ready));
        self.emit(abi::branch(&previous_scan));
        self.emit(abi::label(&previous_ready));
        self.emit(abi::move_register(&scratch14, &scratch13));
        self.emit(abi::compare_immediate(&scratch13, "0"));
        self.emit(abi::branch_eq(&leading_component));
        self.emit(abi::add_immediate(&scratch14, &scratch13, 1));
        self.emit(abi::subtract_registers(&scratch15, &scratch8, &scratch14));
        self.emit(abi::compare_immediate(&scratch15, "2"));
        self.emit(abi::branch_ne(&pop_previous));
        self.emit(abi::add_registers(&scratch16, &scratch12, &scratch14));
        self.emit(abi::load_u8(&scratch17, &scratch16, 0));
        self.emit(abi::compare_immediate(&scratch17, "46"));
        self.emit(abi::branch_ne(&pop_previous));
        self.emit(abi::load_u8(&scratch17, &scratch16, 1));
        self.emit(abi::compare_immediate(&scratch17, "46"));
        self.emit(abi::branch_eq(&append_dot_dot));
        self.emit(abi::branch(&pop_previous));

        // `previous_scan` left `scratch13 == 0`, which is ambiguous: either it found
        // the root '/' at index 0 (absolute path), or it ran off the front without
        // finding any slash, in which case the previous component is the *leading*
        // one and spans the whole output `[0, out_len)`.
        //
        // The block above only recognizes an un-poppable `".."` when a real slash
        // precedes it, so before this the leading case fell straight into
        // `pop_previous` and a leading `".."` was cancelled by the next one:
        // `"../.."` collapsed to `"."` and `"../../a"` to `"a"` (bug-318). That
        // silently strips parent-directory traversal, defeating any caller using
        // `pathNormalize` to test whether a path escapes a root.
        //
        // Same two-dot test as above, with `prev_start = 0` and `prev_len = out_len`.
        // An absolute path is routed to `pop_previous` unchanged so bug-132's
        // keep-the-root-slash behavior still applies.
        self.emit(abi::label(&leading_component));
        self.emit(abi::load_u8(&scratch17, &scratch12, 0));
        self.emit(abi::compare_immediate(&scratch17, "47"));
        self.emit(abi::branch_eq(&pop_previous));
        self.emit(abi::compare_immediate(&scratch8, "2"));
        self.emit(abi::branch_ne(&pop_previous));
        self.emit(abi::compare_immediate(&scratch17, "46"));
        self.emit(abi::branch_ne(&pop_previous));
        self.emit(abi::load_u8(&scratch17, &scratch12, 1));
        self.emit(abi::compare_immediate(&scratch17, "46"));
        self.emit(abi::branch_eq(&append_dot_dot));

        self.emit(abi::label(&pop_previous));
        self.emit(abi::move_register(&scratch13, &scratch8));
        self.emit(abi::label(&pop_scan));
        self.emit(abi::compare_immediate(&scratch13, "0"));
        // No preceding '/': the popped component was the whole (relative) output,
        // so truncate it to length 0 and let `finish` emit the `"."` fallback.
        // Routing to `component_loop` here left the cancelled component in place —
        // `"a/.."` stayed `"a"` instead of collapsing to `"."` (bug-79). Absolute
        // paths find their root '/' at index 0 before this guard fires, so they
        // still keep the leading slash via the bug-132 path below.
        self.emit(abi::branch_eq(&pop_store));
        self.emit(abi::subtract_immediate(&scratch13, &scratch13, 1));
        self.emit(abi::add_registers(&scratch14, &scratch12, &scratch13));
        self.emit(abi::load_u8(&scratch15, &scratch14, 0));
        self.emit(abi::compare_immediate(&scratch15, "47"));
        self.emit(abi::branch_ne(&pop_scan));
        // Found the preceding '/' at index `scratch13`; truncate the output
        // there. When that slash sits at index 0 it is the root slash of an
        // absolute path: keep it (out_len = 1) rather than skipping the store,
        // which left the popped component in place — `"/a/.."` stayed `"/a"`
        // instead of collapsing to `"/"` (bug-132).
        self.emit(abi::compare_immediate(&scratch13, "0"));
        self.emit(abi::branch_ne(&pop_store));
        self.emit(abi::move_immediate(&scratch13, "Integer", "1"));
        self.emit(abi::label(&pop_store));
        self.emit(abi::store_u64(
            &scratch13,
            abi::stack_pointer(),
            out_len_slot,
        ));
        self.emit(abi::branch(&component_loop));

        self.emit(abi::label(&append_dot_dot));
        self.emit(abi::move_immediate(&scratch16, "Integer", "2"));
        self.emit(abi::store_u64(
            &scratch16,
            abi::stack_pointer(),
            component_len_slot,
        ));

        self.emit(abi::label(&append_component));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), out_len_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch12, &scratch9, 8));
        self.emit(abi::compare_immediate(&scratch8, "0"));
        self.emit(abi::branch_eq(&append_copy_loop));
        self.emit(abi::add_registers(&scratch13, &scratch12, &scratch8));
        self.emit(abi::subtract_immediate(&scratch13, &scratch13, 1));
        self.emit(abi::load_u8(&scratch14, &scratch13, 0));
        self.emit(abi::compare_immediate(&scratch14, "47"));
        self.emit(abi::branch_ne(&append_separator));
        self.emit(abi::branch(&append_copy_loop));
        self.emit(abi::label(&append_separator));
        self.emit(abi::move_immediate(&scratch14, "Byte", "47"));
        self.emit(abi::add_registers(&scratch13, &scratch12, &scratch8));
        self.emit(abi::store_u8(&scratch14, &scratch13, 0));
        self.emit(abi::add_immediate(&scratch8, &scratch8, 1));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            out_len_slot,
        ));

        self.emit(abi::label(&append_copy_loop));
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            component_len_slot,
        ));
        self.emit(abi::compare_immediate(&scratch16, "0"));
        self.emit(abi::branch_eq(&append_copy_done));
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            component_start_slot,
        ));
        self.emit(abi::add_registers(&scratch17, &scratch11, &scratch15));
        self.emit(abi::load_u8(&scratch14, &scratch17, 0));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), out_len_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch12, &scratch9, 8));
        self.emit(abi::add_registers(&scratch13, &scratch12, &scratch8));
        self.emit(abi::store_u8(&scratch14, &scratch13, 0));
        self.emit(abi::add_immediate(&scratch15, &scratch15, 1));
        self.emit(abi::store_u64(
            &scratch15,
            abi::stack_pointer(),
            component_start_slot,
        ));
        self.emit(abi::subtract_immediate(&scratch16, &scratch16, 1));
        self.emit(abi::store_u64(
            &scratch16,
            abi::stack_pointer(),
            component_len_slot,
        ));
        self.emit(abi::add_immediate(&scratch8, &scratch8, 1));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            out_len_slot,
        ));
        self.emit(abi::branch(&append_copy_loop));

        self.emit(abi::label(&append_copy_done));
        self.emit(abi::branch(&component_loop));

        self.emit(abi::label(&empty_path));
        self.emit(abi::branch(&finish));

        self.emit(abi::label(&finish));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), out_len_slot));
        self.emit(abi::compare_immediate(&scratch8, "0"));
        self.emit(abi::branch_ne(&finish_nonempty));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), result_slot));
        self.emit(abi::move_immediate(&scratch12, "Byte", "46"));
        self.emit(abi::store_u8(&scratch12, &scratch9, 8));
        self.emit(abi::move_immediate(&scratch8, "Integer", "1"));
        self.emit(abi::label(&finish_nonempty));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), result_slot));
        self.emit(abi::store_u64(&scratch8, &scratch9, 0));
        self.emit(abi::add_registers(&scratch12, &scratch9, &scratch8));
        self.emit(abi::store_u8(abi::ZERO, &scratch12, 8));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: Operand::from(result.render()),
            text: "fs.pathNormalize".to_string(),
        })
    }
}

/// `Body::abi_inline_self` [`crate::codegen::registry::AbiInlineSelf`] wrappers for
/// the five `path*` members — the self-lowering successor to the former `common`
/// `NativeLower` slot (byte-identical: the raw `NirValue` args are lowered by the
/// same dispatcher). Each delegates to the shared
/// [`CodeBuilder::lower_fs_path_call`] dispatcher (kept because the same lowering
/// also serves the `RuntimeCall` node and the standalone `pathJoin` helper), which
/// always lowers these single-arg path calls. The `AbiCtx` is unused (a `path*`
/// member is purely syntactic). A free fn per member so the HRTB fn-pointer coerces
/// (a method would E0308).
fn dispatch_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder
        .lower_fs_path_call(target, args)?
        .ok_or_else(|| format!("native code cannot lower runtime call '{target}'"))
}

pub(crate) fn lower_fs_path_join_nl(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &crate::codegen::registry::AbiCtx,
) -> Result<ValueResult, String> {
    dispatch_path(builder, "fs.pathJoin", args)
}

pub(crate) fn lower_fs_path_base_name_nl(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &crate::codegen::registry::AbiCtx,
) -> Result<ValueResult, String> {
    dispatch_path(builder, "fs.pathBaseName", args)
}

pub(crate) fn lower_fs_path_dir_name_nl(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &crate::codegen::registry::AbiCtx,
) -> Result<ValueResult, String> {
    dispatch_path(builder, "fs.pathDirName", args)
}

pub(crate) fn lower_fs_path_extension_nl(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &crate::codegen::registry::AbiCtx,
) -> Result<ValueResult, String> {
    dispatch_path(builder, "fs.pathExtension", args)
}

pub(crate) fn lower_fs_path_normalize_nl(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &crate::codegen::registry::AbiCtx,
) -> Result<ValueResult, String> {
    dispatch_path(builder, "fs.pathNormalize", args)
}

/// Symbol of the shared standalone `fs::pathJoin` runtime helper.
pub(crate) const FS_PATH_JOIN_SYMBOL: &str = "_mfb_rt_fs_path_join";

/// Lower the standalone `fs::pathJoin` helper. It takes a `List OF String`
/// collection pointer in `x0` and returns a `Result`-shaped value: `x0` holds
/// the tag (`RESULT_OK_TAG`/`RESULT_ERR_TAG`) and, on success, `x1` holds the
/// resulting `String` pointer (on allocation failure it returns `ErrOutOfMemory`).
/// Implementing it as a shared `bl`-reachable helper lets both root native code
/// and imported-package binary_repr lower `pathJoin` identically. Components are
/// joined with `/`, empty components are skipped, an absolute component discards
/// everything accumulated so far, and duplicate separators are avoided.
pub(crate) fn lower_fs_path_join_helper() -> CodeFunction {
    // Vreg-allocated (plan-00-G Phase 2). `parts` (the input List) is held across
    // the `arena_alloc` (spilled); the second pass builds into the allocated string
    // with no further call, so its working registers stay in registers.
    // Pure string joining — no syscall — so it needs no `platform` (bug-331 §J).
    const SEP: &str = "47";
    let symbol = FS_PATH_JOIN_SYMBOL;

    let length_loop = format!("{symbol}_length_loop");
    let length_done = format!("{symbol}_length_done");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let build_loop = format!("{symbol}_build_loop");
    let build_done = format!("{symbol}_build_done");
    let skip_part = format!("{symbol}_skip_part");
    let absolute = format!("{symbol}_absolute");
    let copy_part = format!("{symbol}_copy_part");
    let no_separator = format!("{symbol}_no_separator");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let done = format!("{symbol}_done");

    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let mut vregs = Vregs::new();
    let parts = vregs.next();
    let result = vregs.next();
    let count = vregs.next();
    let total = vregs.next();
    let index = vregs.next();
    let entry = vregs.next();
    let part_len = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&parts, abi::return_register()),
        // Pass 1: upper-bound length = sum(component lengths) + count separators.
        abi::load_u64(&count, &parts, COLLECTION_OFFSET_COUNT),
        abi::move_immediate(&total, "Integer", "0"),
        abi::move_immediate(&index, "Integer", "0"),
        abi::add_immediate(&entry, &parts, COLLECTION_HEADER_SIZE),
        abi::label(&length_loop),
        abi::compare_registers(&index, &count),
        abi::branch_ge(&length_done),
        abi::load_u64(&part_len, &entry, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_registers(&total, &total, &part_len),
        abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&length_loop),
        abi::label(&length_done),
        abi::add_registers(abi::return_register(), &total, &count),
        abi::add_immediate(abi::return_register(), abi::return_register(), 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(symbol, ARENA_ALLOC_SYMBOL)];
    let data_base = vregs.next();
    let capacity = vregs.next();
    let lookup = vregs.next();
    let out_base = vregs.next();
    let cursor = vregs.next();
    let scratch = vregs.next();
    let value_off = vregs.next();
    let value_len = vregs.next();
    let byte = vregs.next();
    let prev = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&result, abi::mfb_return(1)),
        // Pass 2: build the joined path.
        abi::load_u64(&count, &parts, COLLECTION_OFFSET_COUNT),
        // data base = collection + header + capacity * entry_size (plan-01 §4.2:
        // a grown list has capacity > count, so the data region sits past the
        // full lookup capacity, not just the live entries).
        abi::load_u64(&capacity, &parts, COLLECTION_OFFSET_CAPACITY),
        abi::add_immediate(&data_base, &parts, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&scratch, "Integer", &entry_size),
        abi::multiply_registers(&scratch, &capacity, &scratch),
        abi::add_registers(&data_base, &data_base, &scratch),
        abi::add_immediate(&lookup, &parts, COLLECTION_HEADER_SIZE),
        abi::add_immediate(&out_base, &result, 8),
        abi::move_register(&cursor, &out_base),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&build_loop),
        abi::compare_registers(&index, &count),
        abi::branch_ge(&build_done),
        abi::load_u64(&value_len, &lookup, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::compare_immediate(&value_len, "0"),
        abi::branch_eq(&skip_part),
        abi::load_u64(&value_off, &lookup, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers(&value_off, &data_base, &value_off),
        abi::load_u8(&byte, &value_off, 0),
        abi::compare_immediate(&byte, SEP),
        abi::branch_eq(&absolute),
        abi::compare_registers(&cursor, &out_base),
        abi::branch_eq(&no_separator),
        abi::subtract_immediate(&prev, &cursor, 1),
        abi::load_u8(&scratch, &prev, 0),
        abi::compare_immediate(&scratch, SEP),
        abi::branch_eq(&no_separator),
        abi::move_immediate(&scratch, "Byte", SEP),
        abi::store_u8(&scratch, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::branch(&copy_part),
        abi::label(&absolute),
        abi::move_register(&cursor, &out_base),
        abi::label(&no_separator),
        abi::label(&copy_part),
        abi::label(&copy_loop),
        abi::compare_immediate(&value_len, "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &value_off, 0),
        abi::store_u8(&byte, &cursor, 0),
        abi::add_immediate(&value_off, &value_off, 1),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::subtract_immediate(&value_len, &value_len, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::label(&skip_part),
        abi::add_immediate(&lookup, &lookup, COLLECTION_ENTRY_SIZE),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&build_loop),
        abi::label(&build_done),
        abi::subtract_registers(&scratch, &cursor, &out_base),
        abi::store_u64(&scratch, &result, 0),
        abi::move_immediate(&byte, "Integer", "0"),
        abi::store_u8(&byte, &cursor, 0),
        abi::move_register(RESULT_VALUE_REGISTER, &result),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    finalize_vreg_helper(
        "runtime.fsPathJoin",
        symbol,
        "String",
        instructions,
        relocations,
    )
}
