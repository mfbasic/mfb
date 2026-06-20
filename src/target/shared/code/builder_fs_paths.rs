use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_fs_path_call(
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
        let parts_slot = self.store_string_pointer("fs_path_join_parts", &parts.location);
        let alloc_ok = self.label("fs_path_join_alloc_ok");
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            parts_slot,
        ));
        self.emit(abi::branch_link(FS_PATH_JOIN_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: FS_PATH_JOIN_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        let result = self.allocate_register()?;
        self.emit(abi::move_register(&result, RESULT_VALUE_REGISTER));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "fs.pathJoin".to_string(),
        })
    }

    fn lower_fs_path_base_name(&mut self, path: &NirValue) -> Result<ValueResult, String> {
        let path = self.lower_value(path)?;
        self.require_string("fs.pathBaseName path", &path)?;
        let path_slot = self.store_string_pointer("fs_path_base_name_path", &path.location);
        let whole_root = self.label("fs_path_base_name_whole_root");
        let trim_loop = self.label("fs_path_base_name_trim_loop");
        let trim_done = self.label("fs_path_base_name_trim_done");
        let scan_loop = self.label("fs_path_base_name_scan_loop");
        let found_slash = self.label("fs_path_base_name_found_slash");
        let range_ready = self.label("fs_path_base_name_range_ready");

        self.emit(abi::load_u64("x9", abi::stack_pointer(), path_slot));
        self.emit(abi::load_u64("x10", "x9", 0));
        self.emit(abi::add_immediate("x11", "x9", 8));
        self.emit(abi::compare_immediate("x10", "1"));
        self.emit(abi::branch_ne(&trim_loop));
        self.emit(abi::load_u8("x12", "x11", 0));
        self.emit(abi::compare_immediate("x12", "47"));
        self.emit(abi::branch_eq(&whole_root));

        self.emit(abi::label(&trim_loop));
        self.emit(abi::compare_immediate("x10", "1"));
        self.emit(abi::branch_le(&trim_done));
        self.emit(abi::add_registers("x12", "x11", "x10"));
        self.emit(abi::subtract_immediate("x12", "x12", 1));
        self.emit(abi::load_u8("x13", "x12", 0));
        self.emit(abi::compare_immediate("x13", "47"));
        self.emit(abi::branch_ne(&trim_done));
        self.emit(abi::subtract_immediate("x10", "x10", 1));
        self.emit(abi::branch(&trim_loop));

        self.emit(abi::label(&trim_done));
        self.emit(abi::move_register("x14", "x10"));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_immediate("x14", "0"));
        self.emit(abi::branch_eq(&range_ready));
        self.emit(abi::subtract_immediate("x14", "x14", 1));
        self.emit(abi::add_registers("x12", "x11", "x14"));
        self.emit(abi::load_u8("x13", "x12", 0));
        self.emit(abi::compare_immediate("x13", "47"));
        self.emit(abi::branch_eq(&found_slash));
        self.emit(abi::branch(&scan_loop));

        self.emit(abi::label(&found_slash));
        self.emit(abi::add_immediate("x14", "x14", 1));
        self.emit(abi::branch(&range_ready));

        self.emit(abi::label(&whole_root));
        self.emit(abi::move_immediate("x14", "Integer", "0"));
        self.emit(abi::move_immediate("x10", "Integer", "1"));
        self.emit(abi::label(&range_ready));
        self.emit(abi::add_registers("x15", "x11", "x14"));
        self.emit(abi::subtract_registers("x16", "x10", "x14"));
        let result = self.emit_materialize_string_from_bytes("x15", "x16")?;
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "fs.pathBaseName".to_string(),
        })
    }

    fn lower_fs_path_dir_name(&mut self, path: &NirValue) -> Result<ValueResult, String> {
        let path = self.lower_value(path)?;
        self.require_string("fs.pathDirName path", &path)?;
        let path_slot = self.store_string_pointer("fs_path_dir_name_path", &path.location);
        let dot = self.label("fs_path_dir_name_dot");
        let root = self.label("fs_path_dir_name_root");
        let trim_loop = self.label("fs_path_dir_name_trim_loop");
        let trim_done = self.label("fs_path_dir_name_trim_done");
        let scan_loop = self.label("fs_path_dir_name_scan_loop");
        let found_slash = self.label("fs_path_dir_name_found_slash");
        let materialize = self.label("fs_path_dir_name_materialize");

        self.emit(abi::load_u64("x9", abi::stack_pointer(), path_slot));
        self.emit(abi::load_u64("x10", "x9", 0));
        self.emit(abi::add_immediate("x11", "x9", 8));
        self.emit(abi::compare_immediate("x10", "0"));
        self.emit(abi::branch_eq(&dot));
        self.emit(abi::compare_immediate("x10", "1"));
        self.emit(abi::branch_ne(&trim_loop));
        self.emit(abi::load_u8("x12", "x11", 0));
        self.emit(abi::compare_immediate("x12", "47"));
        self.emit(abi::branch_eq(&root));

        self.emit(abi::label(&trim_loop));
        self.emit(abi::compare_immediate("x10", "1"));
        self.emit(abi::branch_le(&trim_done));
        self.emit(abi::add_registers("x12", "x11", "x10"));
        self.emit(abi::subtract_immediate("x12", "x12", 1));
        self.emit(abi::load_u8("x13", "x12", 0));
        self.emit(abi::compare_immediate("x13", "47"));
        self.emit(abi::branch_ne(&trim_done));
        self.emit(abi::subtract_immediate("x10", "x10", 1));
        self.emit(abi::branch(&trim_loop));

        self.emit(abi::label(&trim_done));
        self.emit(abi::move_register("x14", "x10"));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_immediate("x14", "0"));
        self.emit(abi::branch_eq(&dot));
        self.emit(abi::subtract_immediate("x14", "x14", 1));
        self.emit(abi::add_registers("x12", "x11", "x14"));
        self.emit(abi::load_u8("x13", "x12", 0));
        self.emit(abi::compare_immediate("x13", "47"));
        self.emit(abi::branch_eq(&found_slash));
        self.emit(abi::branch(&scan_loop));

        self.emit(abi::label(&found_slash));
        self.emit(abi::compare_immediate("x14", "0"));
        self.emit(abi::branch_eq(&root));
        self.emit(abi::move_register("x10", "x14"));
        self.emit(abi::branch(&materialize));

        self.emit(abi::label(&dot));
        let dot_register = self.load_string_constant(".")?;
        self.emit(abi::move_register("x17", &dot_register));
        let done_constant = self.label("fs_path_dir_name_done_constant");
        self.emit(abi::branch(&done_constant));

        self.emit(abi::label(&root));
        let slash_register = self.load_string_constant("/")?;
        self.emit(abi::move_register("x17", &slash_register));
        self.emit(abi::branch(&done_constant));

        self.emit(abi::label(&materialize));
        self.emit(abi::move_register("x15", "x11"));
        let result = self.emit_materialize_string_from_bytes("x15", "x10")?;
        let final_slot = self.allocate_stack_object("fs_path_dir_name_result", 8);
        self.emit(abi::store_u64(&result, abi::stack_pointer(), final_slot));
        let done = self.label("fs_path_dir_name_done");
        self.emit(abi::branch(&done));
        self.emit(abi::label(&done_constant));
        self.emit(abi::store_u64("x17", abi::stack_pointer(), final_slot));
        self.emit(abi::label(&done));
        let out = self.allocate_register()?;
        self.emit(abi::load_u64(&out, abi::stack_pointer(), final_slot));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: out,
            text: "fs.pathDirName".to_string(),
        })
    }

    fn lower_fs_path_extension(&mut self, path: &NirValue) -> Result<ValueResult, String> {
        let path = self.lower_value(path)?;
        self.require_string("fs.pathExtension path", &path)?;
        let path_slot = self.store_string_pointer("fs_path_extension_path", &path.location);
        let empty = self.label("fs_path_extension_empty");
        let trim_loop = self.label("fs_path_extension_trim_loop");
        let trim_done = self.label("fs_path_extension_trim_done");
        let scan_loop = self.label("fs_path_extension_scan_loop");
        let found_dot = self.label("fs_path_extension_found_dot");
        let materialize = self.label("fs_path_extension_materialize");
        let done = self.label("fs_path_extension_done");

        self.emit(abi::load_u64("x9", abi::stack_pointer(), path_slot));
        self.emit(abi::load_u64("x10", "x9", 0));
        self.emit(abi::add_immediate("x11", "x9", 8));
        self.emit(abi::label(&trim_loop));
        self.emit(abi::compare_immediate("x10", "0"));
        self.emit(abi::branch_eq(&empty));
        self.emit(abi::add_registers("x12", "x11", "x10"));
        self.emit(abi::subtract_immediate("x12", "x12", 1));
        self.emit(abi::load_u8("x13", "x12", 0));
        self.emit(abi::compare_immediate("x13", "47"));
        self.emit(abi::branch_ne(&trim_done));
        self.emit(abi::subtract_immediate("x10", "x10", 1));
        self.emit(abi::branch(&trim_loop));
        self.emit(abi::label(&trim_done));
        self.emit(abi::move_register("x14", "x10"));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_immediate("x14", "0"));
        self.emit(abi::branch_eq(&empty));
        self.emit(abi::subtract_immediate("x14", "x14", 1));
        self.emit(abi::add_registers("x12", "x11", "x14"));
        self.emit(abi::load_u8("x13", "x12", 0));
        self.emit(abi::compare_immediate("x13", "47"));
        self.emit(abi::branch_eq(&empty));
        self.emit(abi::compare_immediate("x13", "46"));
        self.emit(abi::branch_eq(&found_dot));
        self.emit(abi::branch(&scan_loop));
        self.emit(abi::label(&found_dot));
        self.emit(abi::add_registers("x15", "x11", "x14"));
        self.emit(abi::subtract_registers("x16", "x10", "x14"));
        self.emit(abi::branch(&materialize));
        self.emit(abi::label(&empty));
        self.emit(abi::move_register("x15", "x11"));
        self.emit(abi::move_immediate("x16", "Integer", "0"));
        self.emit(abi::label(&materialize));
        let result = self.emit_materialize_string_from_bytes("x15", "x16")?;
        self.emit(abi::branch(&done));
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "fs.pathExtension".to_string(),
        })
    }

    fn lower_fs_path_normalize(&mut self, path: &NirValue) -> Result<ValueResult, String> {
        let path = self.lower_value(path)?;
        self.require_string("fs.pathNormalize path", &path)?;
        let path_slot = self.store_string_pointer("fs_path_normalize_path", &path.location);
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
        let pop_scan = self.label("fs_path_normalize_pop_scan");
        let finish = self.label("fs_path_normalize_finish");
        let finish_nonempty = self.label("fs_path_normalize_finish_nonempty");

        self.emit(abi::load_u64("x9", abi::stack_pointer(), path_slot));
        self.emit(abi::load_u64("x10", "x9", 0));
        self.emit(abi::add_immediate("x11", "x9", 8));
        self.emit(abi::add_immediate(abi::return_register(), "x10", 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), path_slot));
        self.emit(abi::load_u64("x10", "x9", 0));
        self.emit(abi::add_immediate("x11", "x9", 8));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::store_u64("x31", "x1", 0));
        self.emit(abi::store_u8("x31", "x1", 8));
        self.emit(abi::store_u64("x31", abi::stack_pointer(), out_len_slot));
        self.emit(abi::store_u64(
            "x31",
            abi::stack_pointer(),
            input_index_slot,
        ));
        self.emit(abi::compare_immediate("x10", "0"));
        self.emit(abi::branch_eq(&empty_path));
        self.emit(abi::load_u8("x12", "x11", 0));
        self.emit(abi::compare_immediate("x12", "47"));
        self.emit(abi::branch_ne(&initial_relative));
        self.emit(abi::load_u64("x13", abi::stack_pointer(), result_slot));
        self.emit(abi::move_immediate("x12", "Byte", "47"));
        self.emit(abi::store_u8("x12", "x13", 8));
        self.emit(abi::move_immediate("x12", "Integer", "1"));
        self.emit(abi::store_u64("x12", abi::stack_pointer(), out_len_slot));
        self.emit(abi::store_u64(
            "x12",
            abi::stack_pointer(),
            input_index_slot,
        ));
        self.emit(abi::label(&skip_initial_slashes));
        self.emit(abi::load_u64("x14", abi::stack_pointer(), input_index_slot));
        self.emit(abi::compare_registers("x14", "x10"));
        self.emit(abi::branch_ge(&component_loop));
        self.emit(abi::add_registers("x15", "x11", "x14"));
        self.emit(abi::load_u8("x16", "x15", 0));
        self.emit(abi::compare_immediate("x16", "47"));
        self.emit(abi::branch_ne(&component_loop));
        self.emit(abi::add_immediate("x14", "x14", 1));
        self.emit(abi::store_u64(
            "x14",
            abi::stack_pointer(),
            input_index_slot,
        ));
        self.emit(abi::branch(&skip_initial_slashes));

        self.emit(abi::label(&initial_relative));
        self.emit(abi::store_u64(
            "x31",
            abi::stack_pointer(),
            input_index_slot,
        ));

        self.emit(abi::label(&component_loop));
        self.emit(abi::label(&skip_slashes));
        self.emit(abi::load_u64("x14", abi::stack_pointer(), input_index_slot));
        self.emit(abi::compare_registers("x14", "x10"));
        self.emit(abi::branch_ge(&finish));
        self.emit(abi::add_registers("x15", "x11", "x14"));
        self.emit(abi::load_u8("x16", "x15", 0));
        self.emit(abi::compare_immediate("x16", "47"));
        self.emit(abi::branch_ne(&scan_component));
        self.emit(abi::add_immediate("x14", "x14", 1));
        self.emit(abi::store_u64(
            "x14",
            abi::stack_pointer(),
            input_index_slot,
        ));
        self.emit(abi::branch(&skip_slashes));

        self.emit(abi::label(&scan_component));
        self.emit(abi::store_u64(
            "x14",
            abi::stack_pointer(),
            component_start_slot,
        ));
        self.emit(abi::label(&scan_component_loop));
        self.emit(abi::compare_registers("x14", "x10"));
        self.emit(abi::branch_ge(&component_ready));
        self.emit(abi::add_registers("x15", "x11", "x14"));
        self.emit(abi::load_u8("x16", "x15", 0));
        self.emit(abi::compare_immediate("x16", "47"));
        self.emit(abi::branch_eq(&component_ready));
        self.emit(abi::add_immediate("x14", "x14", 1));
        self.emit(abi::branch(&scan_component_loop));

        self.emit(abi::label(&component_ready));
        self.emit(abi::store_u64(
            "x14",
            abi::stack_pointer(),
            input_index_slot,
        ));
        self.emit(abi::load_u64(
            "x15",
            abi::stack_pointer(),
            component_start_slot,
        ));
        self.emit(abi::subtract_registers("x16", "x14", "x15"));
        self.emit(abi::store_u64(
            "x16",
            abi::stack_pointer(),
            component_len_slot,
        ));
        self.emit(abi::compare_immediate("x16", "1"));
        self.emit(abi::branch_ne(&check_dot_dot));
        self.emit(abi::add_registers("x17", "x11", "x15"));
        self.emit(abi::load_u8("x8", "x17", 0));
        self.emit(abi::compare_immediate("x8", "46"));
        self.emit(abi::branch_eq(&component_loop));
        self.emit(abi::branch(&append_component));

        self.emit(abi::label(&check_dot_dot));
        self.emit(abi::compare_immediate("x16", "2"));
        self.emit(abi::branch_ne(&append_component));
        self.emit(abi::add_registers("x17", "x11", "x15"));
        self.emit(abi::load_u8("x8", "x17", 0));
        self.emit(abi::compare_immediate("x8", "46"));
        self.emit(abi::branch_ne(&append_component));
        self.emit(abi::load_u8("x8", "x17", 1));
        self.emit(abi::compare_immediate("x8", "46"));
        self.emit(abi::branch_eq(&handle_dot_dot));
        self.emit(abi::branch(&append_component));

        self.emit(abi::label(&handle_dot_dot));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), out_len_slot));
        self.emit(abi::compare_immediate("x8", "0"));
        self.emit(abi::branch_eq(&append_dot_dot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x12", "x9", 8));
        self.emit(abi::compare_immediate("x8", "1"));
        self.emit(abi::branch_ne(&maybe_dot_dot));
        self.emit(abi::load_u8("x13", "x12", 0));
        self.emit(abi::compare_immediate("x13", "47"));
        self.emit(abi::branch_eq(&component_loop));
        self.emit(abi::label(&maybe_dot_dot));
        self.emit(abi::move_register("x13", "x8"));
        self.emit(abi::label(&previous_scan));
        self.emit(abi::compare_immediate("x13", "0"));
        self.emit(abi::branch_eq(&previous_ready));
        self.emit(abi::subtract_immediate("x13", "x13", 1));
        self.emit(abi::add_registers("x14", "x12", "x13"));
        self.emit(abi::load_u8("x15", "x14", 0));
        self.emit(abi::compare_immediate("x15", "47"));
        self.emit(abi::branch_eq(&previous_ready));
        self.emit(abi::branch(&previous_scan));
        self.emit(abi::label(&previous_ready));
        self.emit(abi::move_register("x14", "x13"));
        self.emit(abi::compare_immediate("x13", "0"));
        self.emit(abi::branch_eq(&pop_previous));
        self.emit(abi::add_immediate("x14", "x13", 1));
        self.emit(abi::subtract_registers("x15", "x8", "x14"));
        self.emit(abi::compare_immediate("x15", "2"));
        self.emit(abi::branch_ne(&pop_previous));
        self.emit(abi::add_registers("x16", "x12", "x14"));
        self.emit(abi::load_u8("x17", "x16", 0));
        self.emit(abi::compare_immediate("x17", "46"));
        self.emit(abi::branch_ne(&pop_previous));
        self.emit(abi::load_u8("x17", "x16", 1));
        self.emit(abi::compare_immediate("x17", "46"));
        self.emit(abi::branch_eq(&append_dot_dot));

        self.emit(abi::label(&pop_previous));
        self.emit(abi::move_register("x13", "x8"));
        self.emit(abi::label(&pop_scan));
        self.emit(abi::compare_immediate("x13", "0"));
        self.emit(abi::branch_eq(&component_loop));
        self.emit(abi::subtract_immediate("x13", "x13", 1));
        self.emit(abi::add_registers("x14", "x12", "x13"));
        self.emit(abi::load_u8("x15", "x14", 0));
        self.emit(abi::compare_immediate("x15", "47"));
        self.emit(abi::branch_ne(&pop_scan));
        self.emit(abi::compare_immediate("x13", "0"));
        self.emit(abi::branch_eq(&component_loop));
        self.emit(abi::store_u64("x13", abi::stack_pointer(), out_len_slot));
        self.emit(abi::branch(&component_loop));

        self.emit(abi::label(&append_dot_dot));
        self.emit(abi::move_immediate("x16", "Integer", "2"));
        self.emit(abi::store_u64(
            "x16",
            abi::stack_pointer(),
            component_len_slot,
        ));

        self.emit(abi::label(&append_component));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), out_len_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x12", "x9", 8));
        self.emit(abi::compare_immediate("x8", "0"));
        self.emit(abi::branch_eq(&append_copy_loop));
        self.emit(abi::add_registers("x13", "x12", "x8"));
        self.emit(abi::subtract_immediate("x13", "x13", 1));
        self.emit(abi::load_u8("x14", "x13", 0));
        self.emit(abi::compare_immediate("x14", "47"));
        self.emit(abi::branch_ne(&append_separator));
        self.emit(abi::branch(&append_copy_loop));
        self.emit(abi::label(&append_separator));
        self.emit(abi::move_immediate("x14", "Byte", "47"));
        self.emit(abi::add_registers("x13", "x12", "x8"));
        self.emit(abi::store_u8("x14", "x13", 0));
        self.emit(abi::add_immediate("x8", "x8", 1));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), out_len_slot));

        self.emit(abi::label(&append_copy_loop));
        self.emit(abi::load_u64(
            "x16",
            abi::stack_pointer(),
            component_len_slot,
        ));
        self.emit(abi::compare_immediate("x16", "0"));
        self.emit(abi::branch_eq(&append_copy_done));
        self.emit(abi::load_u64(
            "x15",
            abi::stack_pointer(),
            component_start_slot,
        ));
        self.emit(abi::add_registers("x17", "x11", "x15"));
        self.emit(abi::load_u8("x14", "x17", 0));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), out_len_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x12", "x9", 8));
        self.emit(abi::add_registers("x13", "x12", "x8"));
        self.emit(abi::store_u8("x14", "x13", 0));
        self.emit(abi::add_immediate("x15", "x15", 1));
        self.emit(abi::store_u64(
            "x15",
            abi::stack_pointer(),
            component_start_slot,
        ));
        self.emit(abi::subtract_immediate("x16", "x16", 1));
        self.emit(abi::store_u64(
            "x16",
            abi::stack_pointer(),
            component_len_slot,
        ));
        self.emit(abi::add_immediate("x8", "x8", 1));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), out_len_slot));
        self.emit(abi::branch(&append_copy_loop));

        self.emit(abi::label(&append_copy_done));
        self.emit(abi::branch(&component_loop));

        self.emit(abi::label(&empty_path));
        self.emit(abi::branch(&finish));

        self.emit(abi::label(&finish));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), out_len_slot));
        self.emit(abi::compare_immediate("x8", "0"));
        self.emit(abi::branch_ne(&finish_nonempty));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), result_slot));
        self.emit(abi::move_immediate("x12", "Byte", "46"));
        self.emit(abi::store_u8("x12", "x9", 8));
        self.emit(abi::move_immediate("x8", "Integer", "1"));
        self.emit(abi::label(&finish_nonempty));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), result_slot));
        self.emit(abi::store_u64("x8", "x9", 0));
        self.emit(abi::add_registers("x12", "x9", "x8"));
        self.emit(abi::store_u8("x31", "x12", 8));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "fs.pathNormalize".to_string(),
        })
    }
}
