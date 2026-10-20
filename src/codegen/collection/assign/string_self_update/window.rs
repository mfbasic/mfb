//! plan-176-Q: window (`mid`/trim) and identity self-updates. The arm tables and the
//! shadow/regrow helpers live in `collection/assign/string_self_update/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// plan-146-C: `s = f(s, …)` for a builtin whose result is a contiguous window
    /// of `s`'s own bytes ([`STRING_WINDOW_FNS`]). The window step is the code the
    /// copying lowering runs (every check and raise happens there, before a byte
    /// moves); then the window is moved down to offset 8 of `s`'s own block, the
    /// length and NUL are stored, and the bytes it gave up become spare capacity in
    /// the binding's shadow.
    pub(crate) fn try_inplace_string_window_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let Some((target, _)) = self_update_call_parts(value) else {
            return Ok(false);
        };
        let Some(bare) = self_update_builtin(target) else {
            return Ok(false);
        };
        let Some((name, arity, kind)) = STRING_WINDOW_FNS
            .iter()
            .find(|(name, _, _)| *name == bare)
            .map(|(name, arity, kind)| (*name, arity.clone(), *kind))
        else {
            return Ok(false);
        };
        let Some(args) = self.resolve_string_self_update(site, value, name, arity, true) else {
            return Ok(false);
        };
        let args: Vec<NirValue> = args.to_vec();

        let block_slot = site.dest.block_slot();
        let shadow = self
            .string_shadow_slot(site)?
            .ok_or("native String window self-update lost its capacity shadow")?;
        // The window step reads the binding's block and the remaining arguments,
        // which `G21-string` proved do not read the binding.
        let mut rest = Vec::new();
        for arg in &args[1..] {
            rest.push(self.lower_value(arg)?);
        }
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
        let value_result = ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(block),
            text: String::new(),
        };
        let (ptr, len) = self.emit_string_window(kind, &value_result, &rest)?;
        // The marker slot: its presence proves this arm fired (`ArmId::markers`).
        let ptr_slot = self.allocate_stack_object("inplace_str_window", 8);
        let len_slot = self.allocate_stack_object("inplace_str_window_len", 8);
        self.emit(abi::store_u64(&ptr, abi::stack_pointer(), ptr_slot));
        self.emit(abi::store_u64(&len, abi::stack_pointer(), len_slot));
        // `fs::pathDirName("")` is `.` — one byte where the binding had none — and
        // its window points at the constant, never into the block, so a regrow
        // cannot invalidate it. Every other row's window is inside the block and no
        // longer than it.
        if kind == WindowKind::PathDirName {
            self.emit_string_reserve(block_slot, shadow.slot, len_slot)?;
        }
        // Move the window to `block + 8`. The destination is never after the
        // source, so a forward copy reads each byte before it is overwritten.
        // bug-732: a window that already starts there (`stripSuffix`, a no-match
        // `stripPrefix`, `trimEnd`, `left`/`mid` from 0) does not move, so the call
        // costs only the bytes it gives up, not a copy of every byte it keeps.
        let dst = self.temporary_vreg();
        let src = self.temporary_vreg();
        let count = self.temporary_vreg();
        let in_place = self.label("inplace_str_window_in_place");
        self.emit(abi::load_u64(&dst, abi::stack_pointer(), block_slot));
        self.emit(abi::add_immediate(&dst, &dst, 8));
        self.emit(abi::load_u64(&src, abi::stack_pointer(), ptr_slot));
        self.emit(abi::compare_registers(&src, &dst));
        self.emit(abi::branch_eq(&in_place));
        self.emit(abi::load_u64(&count, abi::stack_pointer(), len_slot));
        self.emit_copy_bytes(&dst, &src, &count, "inplace_str_window_move");
        self.emit(abi::label(&in_place));
        self.emit_string_set_len(block_slot, shadow.slot, len_slot);
        self.publish_string_shadow(&shadow)?;
        if let Some(local) = self.locals.get_mut(site.name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// The window step of one [`STRING_WINDOW_FNS`] row: the copying lowering's own
    /// half, except `graphemeAt`'s, whose copying path reads its span out of a
    /// freshly built `List OF String` (plan-146-C Correction C1).
    fn emit_string_window(
        &mut self,
        kind: WindowKind,
        value: &ValueResult,
        rest: &[ValueResult],
    ) -> Result<(VirtualRegister, VirtualRegister), String> {
        use crate::codegen::builtins::strings::{
            gen_graphemes, gen_left_right, gen_strip, gen_trim,
        };
        let arg = |i: usize| -> Result<&ValueResult, String> {
            rest.get(i)
                .ok_or_else(|| "native String window self-update lost an argument".to_string())
        };
        match kind {
            WindowKind::LeftRight { right } => {
                gen_left_right::lower_strings_left_right_window(self, value, arg(0)?, right)
            }
            WindowKind::Strip { suffix } => {
                gen_strip::lower_strings_strip_window(self, value, arg(0)?, suffix)
            }
            WindowKind::Trim { start, end } => {
                gen_trim::lower_strings_trim_window(self, value, start, end)
            }
            WindowKind::TrimChars => {
                crate::codegen::builtins::strings::func_trim_chars::window(self, value, arg(0)?)
            }
            WindowKind::GraphemeAt => gen_graphemes::grapheme_at_window(self, value, arg(0)?),
            WindowKind::Mid => self.emit_string_mid_window(value, arg(0)?, arg(1)?),
            WindowKind::PathBaseName => self.fs_path_base_name_window(value),
            WindowKind::PathDirName => self.fs_path_dir_name_window(value),
            WindowKind::PathExtension => {
                let (start, span, _done) = self.fs_path_extension_window(value)?;
                Ok((start, span))
            }
        }
    }

    /// `strings::mid`'s window, with its `ErrIndexOutOfRange` raise emitted right
    /// after it (the copying lowering emits the same raise after its copy; nothing
    /// has written to the binding at either point).
    fn emit_string_mid_window(
        &mut self,
        value: &ValueResult,
        start: &ValueResult,
        count: &ValueResult,
    ) -> Result<(VirtualRegister, VirtualRegister), String> {
        use crate::codegen::collection::search::builder_search::MidStringRegs;
        let value_slot = self.spill_to_slot("inplace_str_mid_value", &value.location);
        let start_slot = self.spill_to_slot("inplace_str_mid_start", &start.location);
        let count_slot = self.spill_to_slot("inplace_str_mid_count", &count.location);
        let regs: Vec<VirtualRegister> = (0..13).map(|_| self.temporary_vreg()).collect();
        let window = self.lower_mid_string_window(
            value_slot,
            start_slot,
            count_slot,
            &MidStringRegs {
                value_ptr: &regs[0],
                string_len: &regs[1],
                cursor: &regs[2],
                remaining: &regs[3],
                scalar_index: &regs[4],
                start_index: &regs[5],
                count_value: &regs[6],
                end_index: &regs[7],
                byte: &regs[8],
                mask: &regs[9],
                start_ptr: &regs[10],
                end_ptr: &regs[11],
                byte_len: &regs[12],
            },
        )?;
        let ok = self.label("inplace_str_mid_ok");
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&window.invalid_range));
        self.raise_error("strings.mid", "ErrIndexOutOfRange")?;
        self.emit(abi::label(&ok));
        let ptr = self.temporary_vreg();
        let len = self.temporary_vreg();
        self.emit(abi::load_u64(
            &ptr,
            abi::stack_pointer(),
            window.start_ptr_slot,
        ));
        self.emit(abi::load_u64(
            &len,
            abi::stack_pointer(),
            window.byte_len_slot,
        ));
        Ok((ptr, len))
    }

    /// plan-146-B: `s = toString(s)` on a `String`. `toString` of a `String` is the
    /// identity (`.ai/codegen-invariants.md`: "`toString(String)` is the IDENTITY
    /// arm — it hands back its own argument"), so in place it is a no-op: the block
    /// stays, nothing is freed, nothing is stored. The copying path allocated a
    /// block, copied the bytes into it (bug-667) and freed the old one, every
    /// statement. Emits one marker slot, so the matrix can see the arm fired.
    pub(crate) fn try_inplace_string_identity_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        // No `G-shadow`: the length does not change, so the block keeps whatever
        // spare bytes it had, and the shadow still describes it.
        if self
            .resolve_string_self_update(site, value, "toString", 1..=1, false)
            .is_none()
        {
            return Ok(false);
        }
        self.allocate_stack_object("inplace_str_identity", 8);
        if let Some(local) = self.locals.get_mut(site.name) {
            local.constant = None;
        }
        Ok(true)
    }
}
