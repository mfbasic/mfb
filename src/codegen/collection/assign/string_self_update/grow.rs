//! plan-176-Q: length-growing string self-updates — measure, write, pad and move right. The arm
//! tables and the shadow/regrow helpers live in `collection/assign/string_self_update/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// plan-146-D: `s = f(s, …)` for a builtin whose result is `s` with bytes
    /// added ([`STRING_GROW_FNS`]). The measure step computes the result's length
    /// and raises everything the row can raise; then the block is reserved (which
    /// allocates only when the binding outgrows its spare capacity, so a loop
    /// allocates `O(log n)` times), the added bytes are written — after `s`'s for
    /// an append, before them for a prefix, which moves `s`'s bytes right first —
    /// and the length is stored.
    pub(crate) fn try_inplace_string_grow_assign(
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
        let Some((name, arity, kind)) = STRING_GROW_FNS
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
            .ok_or("native String grow self-update lost its capacity shadow")?;
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
        let value_slot = self.spill_to_slot("inplace_str_grow_value", &value_result.location);
        // The marker slot: its presence proves this arm fired (`ArmId::markers`).
        let newlen_slot = self.allocate_stack_object("inplace_str_grow", 8);
        let grow = self.emit_string_grow_measure(kind, value_slot, &rest, newlen_slot)?;
        // Every raise is behind us; the reserve is the only remaining failure, and
        // it allocates before it writes.
        self.emit_string_reserve(block_slot, shadow.slot, newlen_slot)?;
        self.emit_string_grow_write(kind, block_slot, newlen_slot, &grow);
        self.emit_string_set_len(block_slot, shadow.slot, newlen_slot);
        self.publish_string_shadow(&shadow)?;
        if let Some(local) = self.locals.get_mut(site.name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// What a grow row's measure step leaves for its write step.
    fn emit_string_grow_measure(
        &mut self,
        kind: GrowKind,
        value_slot: usize,
        rest: &[ValueResult],
        newlen_slot: usize,
    ) -> Result<GrowWrite, String> {
        let arg = |i: usize| -> Result<&ValueResult, String> {
            rest.get(i)
                .ok_or_else(|| "native String grow self-update lost an argument".to_string())
        };
        match kind {
            GrowKind::Pad { right } => {
                use crate::codegen::builtins::strings::gen_pad::{pad_measure, PadScratch};
                let width_slot = self.spill_to_slot("inplace_str_grow_width", &arg(0)?.location);
                let pad_slot = self.string_pad_char_slot(rest.get(1))?;
                let regs: Vec<VirtualRegister> = (0..9).map(|_| self.temporary_vreg()).collect();
                let measure = pad_measure(
                    self,
                    value_slot,
                    width_slot,
                    pad_slot,
                    &PadScratch {
                        scratch9: &regs[0],
                        scratch10: &regs[1],
                        scratch11: &regs[2],
                        scratch12: &regs[3],
                        scratch13: &regs[4],
                        scratch14: &regs[5],
                        scratch15: &regs[6],
                        scratch16: &regs[7],
                        scratch17: &regs[8],
                    },
                )?;
                let member = if right {
                    "strings.padRight"
                } else {
                    "strings.padLeft"
                };
                self.emit_grow_raise_here(&measure.invalid, member)?;
                let total = self.temporary_vreg();
                self.emit(abi::load_u64(
                    &total,
                    abi::stack_pointer(),
                    measure.total_slot,
                ));
                self.emit(abi::store_u64(&total, abi::stack_pointer(), newlen_slot));
                Ok(GrowWrite::Pad {
                    pad_slot,
                    pad_len_slot: measure.pad_len_slot,
                    pad_count_slot: measure.pad_count_slot,
                })
            }
            GrowKind::PadToWidth { right } => {
                let pad_slot = self.string_pad_char_slot(rest.get(1))?;
                let member = if right {
                    "strings.padRightToWidth"
                } else {
                    "strings.padLeftToWidth"
                };
                let (pad_len_slot, pad_count_slot) = self.emit_pad_to_width_measure(
                    value_slot,
                    arg(0)?,
                    pad_slot,
                    newlen_slot,
                    member,
                )?;
                Ok(GrowWrite::Pad {
                    pad_slot,
                    pad_len_slot,
                    pad_count_slot,
                })
            }
            GrowKind::AppResourcePath => {
                // The prefix is built in the function's self-update scratch, which
                // the prescan gives every function holding this self-update.
                if self.self_update_scratch.is_none() {
                    return Err(
                        "native os.appResourcePath self-update in a function without a scratch"
                            .to_string(),
                    );
                }
                // The relative path is checked first: `os::appResourcePath` rejects a
                // `.` or `..` component before it builds anything.
                self.emit_reject_dot_components(value_slot, "os.appResourcePath")?;
                let (prefix_ptr_slot, prefix_len_slot) = self.emit_app_resource_prefix()?;
                let prefix_len = self.temporary_vreg();
                let value_ptr = self.temporary_vreg();
                let value_len = self.temporary_vreg();
                let total = self.temporary_vreg();
                let overflow = self.label("inplace_str_respath_overflow");
                let ok = self.label("inplace_str_respath_total_ok");
                self.emit(abi::load_u64(
                    &prefix_len,
                    abi::stack_pointer(),
                    prefix_len_slot,
                ));
                self.emit(abi::load_u64(&value_ptr, abi::stack_pointer(), value_slot));
                self.emit(abi::load_u64(&value_len, &value_ptr, 0));
                // plan-157-A §4.3: the prefix ends in the joining `/`, which exists
                // only for a non-empty value — `appResourcePath("")` is the bare
                // base. For an empty value copy one byte less. The adjusted length
                // goes to its own slot: `prefix_len_slot` is the prefix builder's.
                let used_len_slot = self.allocate_stack_object("str_respath_used_len", 8);
                let used_ready = self.label("inplace_str_respath_used_ready");
                self.emit(abi::compare_immediate(&value_len, "0"));
                self.emit(abi::branch_ne(&used_ready));
                self.emit(abi::subtract_immediate(&prefix_len, &prefix_len, 1));
                self.emit(abi::label(&used_ready));
                self.emit(abi::store_u64(
                    &prefix_len,
                    abi::stack_pointer(),
                    used_len_slot,
                ));
                self.emit_checked_size_add(&total, &prefix_len, &value_len, &overflow);
                self.emit(abi::store_u64(&total, abi::stack_pointer(), newlen_slot));
                self.emit(abi::branch(&ok));
                self.emit(abi::label(&overflow));
                self.raise_error_bare("ErrOutOfMemory")?;
                self.emit(abi::label(&ok));
                Ok(GrowWrite::Prefix {
                    ptr_slot: prefix_ptr_slot,
                    len_slot: used_len_slot,
                })
            }
            GrowKind::HostPath(member) => {
                // The same check the copying helper makes, before anything else.
                self.emit_reject_dot_components(value_slot, member)?;
                // The base comes from the member's internal `*Base` helper, which
                // writes it into a caller buffer and returns its length (-1 when the
                // host cannot answer) WITHOUT allocating: the function's
                // self-update scratch, reused and grown geometrically across
                // statements, is that buffer. So the arm allocates only when the
                // scratch grows, and — since the base is re-read on every statement,
                // never cached — follows an `os::setEnv("HOME", …)` between two
                // statements exactly as the copying call does. The first call asks
                // with no buffer (`cap` 0) for the size; if the base grows between
                // the size and the fill (another thread's `setEnv`), the fill
                // reports the larger size and the loop reserves again.
                let base_call = format!("{member}Base");
                let spec = crate::target::shared::runtime::catalog::spec_for_call(&base_call)
                    .ok_or_else(|| format!("{base_call} has no runtime helper spec"))?;
                let symbol =
                    crate::target::shared::runtime::symbol_for_call(spec.helper, spec.call);
                let need_slot = self.allocate_stack_object("inplace_str_hostpath_need", 8);
                let len_slot = self.allocate_stack_object("inplace_str_hostpath_len", 8);
                let ptr_slot = self.allocate_stack_object("inplace_str_hostpath_ptr", 8);
                let fail = self.label("inplace_str_hostpath_fail");
                let measured = self.label("inplace_str_hostpath_measured");
                let fill = self.label("inplace_str_hostpath_fill");
                let filled = self.label("inplace_str_hostpath_filled");
                let got = self.temporary_vreg();
                self.emit(abi::move_immediate(abi::c_arg(0), "Integer", "0"));
                self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
                self.emit_symbol_call(&symbol);
                self.emit(abi::move_register(&got, RESULT_VALUE_REGISTER));
                self.emit(abi::compare_immediate(&got, "0"));
                self.emit(abi::branch_ge(&measured));
                self.emit(abi::label(&fail));
                self.raise_error(member, "ErrUnsupported")?;
                self.emit(abi::label(&measured));
                self.emit(abi::store_u64(&got, abi::stack_pointer(), need_slot));
                self.emit(abi::label(&fill));
                let dest = self.emit_reserve_self_update_scratch(need_slot)?;
                let dst = self.temporary_vreg();
                let cap = self.temporary_vreg();
                self.emit(abi::load_u64(&dst, abi::stack_pointer(), dest));
                self.emit(abi::store_u64(&dst, abi::stack_pointer(), ptr_slot));
                self.emit(abi::load_u64(&cap, abi::stack_pointer(), need_slot));
                self.emit(abi::move_register(abi::c_arg(0), &dst));
                self.emit(abi::move_register(abi::c_arg(1), &cap));
                self.emit_symbol_call(&symbol);
                let got = self.temporary_vreg();
                let cap = self.temporary_vreg();
                self.emit(abi::move_register(&got, RESULT_VALUE_REGISTER));
                self.emit(abi::compare_immediate(&got, "0"));
                self.emit(abi::branch_lt(&fail));
                self.emit(abi::load_u64(&cap, abi::stack_pointer(), need_slot));
                self.emit(abi::compare_registers(&got, &cap));
                self.emit(abi::branch_le(&filled));
                self.emit(abi::store_u64(&got, abi::stack_pointer(), need_slot));
                self.emit(abi::branch(&fill));
                self.emit(abi::label(&filled));
                self.emit(abi::store_u64(&got, abi::stack_pointer(), len_slot));

                let base_len = self.temporary_vreg();
                let value_ptr = self.temporary_vreg();
                let value_len = self.temporary_vreg();
                let total = self.temporary_vreg();
                let overflow = self.label("inplace_str_hostpath_overflow");
                let ok = self.label("inplace_str_hostpath_total_ok");
                let counted = self.label("inplace_str_hostpath_counted");
                let counted_slash = self.label("inplace_str_hostpath_counted_slash");
                self.emit(abi::load_u64(&base_len, abi::stack_pointer(), len_slot));
                self.emit(abi::load_u64(&value_ptr, abi::stack_pointer(), value_slot));
                self.emit(abi::load_u64(&value_len, &value_ptr, 0));
                // The joining `/` exists only for a non-empty value, and not after
                // a base that already ends in `/` (the root, `/`: `/k`, not
                // `//k` — `gen_host_paths.rs:emit_root_elided_len`).
                let last = self.temporary_vreg();
                let base_ptr = self.temporary_vreg();
                self.emit(abi::compare_immediate(&value_len, "0"));
                self.emit(abi::branch_eq(&counted));
                self.emit(abi::compare_immediate(&base_len, "0"));
                self.emit(abi::branch_eq(&counted_slash));
                self.emit(abi::load_u64(&base_ptr, abi::stack_pointer(), ptr_slot));
                self.emit(abi::add_registers(&last, &base_ptr, &base_len));
                self.emit(abi::subtract_immediate(&last, &last, 1));
                self.emit(abi::load_u8(&last, &last, 0));
                self.emit(abi::compare_immediate(&last, "47"));
                self.emit(abi::branch_eq(&counted));
                self.emit(abi::label(&counted_slash));
                self.emit(abi::add_immediate(&base_len, &base_len, 1));
                self.emit(abi::label(&counted));
                self.emit_checked_size_add(&total, &base_len, &value_len, &overflow);
                self.emit(abi::store_u64(&total, abi::stack_pointer(), newlen_slot));
                self.emit(abi::branch(&ok));
                self.emit(abi::label(&overflow));
                self.raise_error_bare("ErrOutOfMemory")?;
                self.emit(abi::label(&ok));
                Ok(GrowWrite::HostBase { ptr_slot, len_slot })
            }
            GrowKind::Repeat => {
                use crate::codegen::builtins::strings::func_repeat::repeat_measure;
                let times_slot = self.spill_to_slot("inplace_str_grow_times", &arg(0)?.location);
                let measure = repeat_measure(self, value_slot, times_slot)?;
                self.emit_grow_raise_here(&measure.invalid, "strings.repeat")?;
                let total = self.temporary_vreg();
                self.emit(abi::load_u64(
                    &total,
                    abi::stack_pointer(),
                    measure.total_slot,
                ));
                self.emit(abi::store_u64(&total, abi::stack_pointer(), newlen_slot));
                Ok(GrowWrite::Repeat { times_slot })
            }
        }
    }

    /// Raise `ErrInvalidArgument` at a measure half's `invalid` label, which the
    /// copying lowering emits after its result instead. Nothing has been written to
    /// the binding at either point (failure atomicity).
    fn emit_grow_raise_here(&mut self, invalid: &str, label: &str) -> Result<(), String> {
        let ok = self.label("inplace_str_grow_ok");
        self.emit(abi::branch(&ok));
        self.emit(abi::label(invalid));
        self.raise_error(label, "ErrInvalidArgument")?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    /// The padChar argument's block, or — when the call defaults it — a one-byte
    /// `" "` `String` block built in the frame. The copying lowering materializes
    /// an arena block for the default; an arm must allocate nothing per statement,
    /// and every reader of a `String` only wants `[len][bytes][NUL]` at the
    /// pointer, which a stack object provides.
    fn string_pad_char_slot(&mut self, pad: Option<&ValueResult>) -> Result<usize, String> {
        if let Some(pad) = pad {
            return Ok(self.spill_to_slot("inplace_str_grow_pad", &pad.location));
        }
        let block = self.allocate_stack_object("inplace_str_grow_space", 16);
        let one = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let address = self.temporary_vreg();
        self.emit(abi::move_immediate(&one, "Integer", "1"));
        self.emit(abi::store_u64(&one, abi::stack_pointer(), block));
        self.emit(abi::move_immediate(&byte, "Byte", "32"));
        self.emit(abi::add_immediate(&address, abi::stack_pointer(), block));
        self.emit(abi::store_u8(&byte, &address, 8));
        self.emit(abi::store_u8(abi::ZERO, &address, 9));
        let slot = self.allocate_stack_object("inplace_str_grow_padptr", 8);
        self.emit(abi::store_u64(&address, abi::stack_pointer(), slot));
        Ok(slot)
    }

    /// The `*ToWidth` measure: `__strings_padToWidthCopies`' rule, natively — a
    /// negative `columns`, a `padChar` that is not exactly one scalar, and a
    /// zero-column `padChar` all raise; then `copies = (columns − displayWidth(value))
    /// / displayWidth(padChar)`, truncating (the undershoot rule), and the result's
    /// byte length is `byteLen(value) + copies × byteLen(padChar)`.
    /// Returns `(pad_len_slot, pad_count_slot)`.
    fn emit_pad_to_width_measure(
        &mut self,
        value_slot: usize,
        columns: &ValueResult,
        pad_slot: usize,
        newlen_slot: usize,
        member: &str,
    ) -> Result<(usize, usize), String> {
        use crate::codegen::builtins::term::func_measure_text::{measure_text_of, scalar_width_of};
        if columns.type_ != ParameterType::Integer {
            return Err(format!(
                "strings.padToWidth columns must be Integer, got {}",
                columns.type_
            ));
        }
        let columns_slot = self.spill_to_slot("inplace_str_width_columns", &columns.location);
        let pad_len_slot = self.allocate_stack_object("inplace_str_width_padlen", 8);
        let pad_count_slot = self.allocate_stack_object("inplace_str_width_copies", 8);
        let invalid = self.label("inplace_str_width_invalid");
        let ok = self.label("inplace_str_width_ok");
        let no_pad = self.label("inplace_str_width_no_pad");
        let have_pad = self.label("inplace_str_width_have_pad");

        let cols = self.temporary_vreg();
        let pad_ptr = self.temporary_vreg();
        let pad_len = self.temporary_vreg();
        let scalars = self.temporary_vreg();
        let unit = self.temporary_vreg();
        let have = self.temporary_vreg();
        let copies = self.temporary_vreg();
        let value_ptr = self.temporary_vreg();
        let value_len = self.temporary_vreg();
        let total = self.temporary_vreg();

        // columns >= 0.
        self.emit(abi::load_u64(&cols, abi::stack_pointer(), columns_slot));
        self.emit(abi::compare_immediate(&cols, "0"));
        self.emit(abi::branch_lt(&invalid));
        // `len(padChar) = 1` — one scalar, as the MFBASIC helper counts it.
        self.emit(abi::load_u64(&pad_ptr, abi::stack_pointer(), pad_slot));
        self.emit(abi::load_u64(&pad_len, &pad_ptr, 0));
        self.emit(abi::store_u64(&pad_len, abi::stack_pointer(), pad_len_slot));
        self.emit(abi::compare_immediate(&pad_len, "0"));
        self.emit(abi::branch_eq(&invalid));
        {
            let loop_label = self.label("inplace_str_width_scalars_loop");
            let not_cont = self.label("inplace_str_width_scalars_not_cont");
            let after = self.label("inplace_str_width_scalars_after");
            let done = self.label("inplace_str_width_scalars_done");
            let cursor = self.temporary_vreg();
            let byte = self.temporary_vreg();
            let masked = self.temporary_vreg();
            let mask = self.temporary_vreg();
            self.emit(abi::add_immediate(&cursor, &pad_ptr, 8));
            self.emit_scalar_count_loop(
                &cursor,
                &byte,
                &scalars,
                &masked,
                &mask,
                &unit,
                &pad_len,
                &loop_label,
                &not_cont,
                &after,
                &done,
            );
        }
        self.emit(abi::compare_immediate(&scalars, "1"));
        self.emit(abi::branch_ne(&invalid));
        // Raw scalar width keeps combining marks and ZWJ invalid as pad units.
        let pad_value = ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(pad_ptr),
            text: String::new(),
        };
        self.emit(abi::load_u64(&pad_ptr, abi::stack_pointer(), pad_slot));
        let unit_value = scalar_width_of(self, &pad_value)?;
        self.emit(abi::move_register(&unit, &unit_value.location));
        self.emit(abi::compare_immediate(&unit, "1"));
        self.emit(abi::branch_lt(&invalid));
        // Current width uses the terminal-cell clamp; no padding when it already fills the width.
        let value_value = ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(value_ptr),
            text: String::new(),
        };
        self.emit(abi::load_u64(&value_ptr, abi::stack_pointer(), value_slot));
        let have_value = measure_text_of(self, &value_value)?;
        self.emit(abi::move_register(&have, &have_value.location));
        self.emit(abi::load_u64(&cols, abi::stack_pointer(), columns_slot));
        self.emit(abi::compare_registers(&have, &cols));
        self.emit(abi::branch_ge(&no_pad));
        // `copies = (columns - have) / unit`, truncating toward zero.
        self.emit(abi::subtract_registers(&copies, &cols, &have));
        self.emit(abi::signed_divide_registers(&copies, &copies, &unit));
        self.emit(abi::branch(&have_pad));
        self.emit(abi::label(&no_pad));
        self.emit(abi::move_immediate(&copies, "Integer", "0"));
        self.emit(abi::label(&have_pad));
        self.emit(abi::store_u64(
            &copies,
            abi::stack_pointer(),
            pad_count_slot,
        ));
        // `newLen = byteLen(value) + copies * byteLen(padChar)`.
        self.emit(abi::load_u64(&value_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&value_len, &value_ptr, 0));
        self.emit(abi::load_u64(&pad_len, abi::stack_pointer(), pad_len_slot));
        self.emit_checked_size_multiply(&total, &copies, &pad_len, &invalid);
        self.emit_checked_size_add(&total, &value_len, &total, &invalid);
        self.emit(abi::store_u64(&total, abi::stack_pointer(), newlen_slot));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&invalid));
        self.raise_error(member, "ErrInvalidArgument")?;
        self.emit(abi::label(&ok));
        Ok((pad_len_slot, pad_count_slot))
    }

    /// Write a grow row's added bytes into the (already reserved) block.
    fn emit_string_grow_write(
        &mut self,
        kind: GrowKind,
        block_slot: usize,
        newlen_slot: usize,
        write: &GrowWrite,
    ) {
        match (kind, write) {
            (
                GrowKind::Pad { right } | GrowKind::PadToWidth { right },
                GrowWrite::Pad {
                    pad_slot,
                    pad_len_slot,
                    pad_count_slot,
                },
            ) => {
                let block = self.temporary_vreg();
                let len = self.temporary_vreg();
                let newlen = self.temporary_vreg();
                let dst = self.temporary_vreg();
                self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                self.emit(abi::load_u64(&len, &block, 0));
                self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
                if right {
                    // Append: the pads start where the bytes end.
                    self.emit(abi::add_immediate(&dst, &block, 8));
                    self.emit(abi::add_registers(&dst, &dst, &len));
                } else {
                    // Prefix: move the bytes right by the pad width first, from the
                    // far end, so the copy never overwrites a byte it has yet to
                    // read; then the pads go at the front.
                    self.emit_string_move_right(&block, &len, &newlen);
                    self.emit(abi::add_immediate(&dst, &block, 8));
                }
                self.emit_pad_copies(&dst, *pad_slot, *pad_len_slot, *pad_count_slot);
            }
            (GrowKind::Repeat, GrowWrite::Repeat { times_slot }) => {
                // The block already holds copy 1; write copies 2..times after it,
                // each from the fixed prefix `[8, 8 + len)`.
                let block = self.temporary_vreg();
                let len = self.temporary_vreg();
                let times = self.temporary_vreg();
                let dst = self.temporary_vreg();
                let src = self.temporary_vreg();
                let count = self.temporary_vreg();
                let outer = self.label("inplace_str_repeat_outer");
                let outer_done = self.label("inplace_str_repeat_done");
                self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                self.emit(abi::load_u64(&len, &block, 0));
                self.emit(abi::load_u64(&times, abi::stack_pointer(), *times_slot));
                self.emit(abi::add_immediate(&dst, &block, 8));
                self.emit(abi::add_registers(&dst, &dst, &len));
                self.emit(abi::label(&outer));
                self.emit(abi::compare_immediate(&times, "1"));
                self.emit(abi::branch_le(&outer_done));
                self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                self.emit(abi::add_immediate(&src, &block, 8));
                self.emit(abi::move_register(&count, &len));
                self.emit_copy_bytes(&dst, &src, &count, "inplace_str_repeat_copy");
                self.emit(abi::subtract_immediate(&times, &times, 1));
                self.emit(abi::branch(&outer));
                self.emit(abi::label(&outer_done));
            }
            (
                GrowKind::AppResourcePath,
                GrowWrite::Prefix {
                    ptr_slot,
                    len_slot: prefix_len_slot,
                },
            ) => {
                let block = self.temporary_vreg();
                let len = self.temporary_vreg();
                let newlen = self.temporary_vreg();
                let dst = self.temporary_vreg();
                let src = self.temporary_vreg();
                let count = self.temporary_vreg();
                self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                self.emit(abi::load_u64(&len, &block, 0));
                self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
                self.emit_string_move_right(&block, &len, &newlen);
                self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                self.emit(abi::add_immediate(&dst, &block, 8));
                self.emit(abi::load_u64(&src, abi::stack_pointer(), *ptr_slot));
                self.emit(abi::load_u64(
                    &count,
                    abi::stack_pointer(),
                    *prefix_len_slot,
                ));
                self.emit_copy_bytes(&dst, &src, &count, "inplace_str_respath_prefix");
            }
            (GrowKind::HostPath(_), GrowWrite::HostBase { ptr_slot, len_slot }) => {
                let block = self.temporary_vreg();
                let len = self.temporary_vreg();
                let newlen = self.temporary_vreg();
                let dst = self.temporary_vreg();
                let src = self.temporary_vreg();
                let count = self.temporary_vreg();
                let slash = self.temporary_vreg();
                let joined = self.label("inplace_str_hostpath_joined");
                self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                self.emit(abi::load_u64(&len, &block, 0));
                self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
                self.emit_string_move_right(&block, &len, &newlen);
                self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                self.emit(abi::add_immediate(&dst, &block, 8));
                self.emit(abi::load_u64(&src, abi::stack_pointer(), *ptr_slot));
                self.emit(abi::load_u64(&count, abi::stack_pointer(), *len_slot));
                self.emit_copy_bytes(&dst, &src, &count, "inplace_str_hostpath_base");
                // The value's bytes were moved to end at `newlen`; for a non-empty
                // value the `/` fills the one byte between them and the base. The
                // block's length field still holds the value's own length here
                // (`emit_string_set_len` runs after this step).
                // The `/` is there exactly when the measure step counted it: the
                // result is one byte longer than base + value.
                self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                self.emit(abi::load_u64(&len, &block, 0));
                self.emit(abi::load_u64(&count, abi::stack_pointer(), *len_slot));
                self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
                self.emit(abi::add_registers(&slash, &count, &len));
                self.emit(abi::compare_registers(&slash, &newlen));
                self.emit(abi::branch_eq(&joined));
                self.emit(abi::add_immediate(&dst, &block, 8));
                self.emit(abi::add_registers(&dst, &dst, &count));
                self.emit(abi::move_immediate(&slash, "Byte", "47"));
                self.emit(abi::store_u8(&slash, &dst, 0));
                self.emit(abi::label(&joined));
            }
            _ => unreachable!("a grow row's write step matches its measure step"),
        }
    }

    /// Move the block's `len` bytes right so they end at `newlen`, copying from the
    /// far end (the regions overlap).
    fn emit_string_move_right(
        &mut self,
        block: &VirtualRegister,
        len: &VirtualRegister,
        newlen: &VirtualRegister,
    ) {
        let src = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let left = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let loop_label = self.label("inplace_str_grow_move_loop");
        let done = self.label("inplace_str_grow_move_done");
        // src = block + 8 + len, dst = block + 8 + newlen, both one past the end.
        self.emit(abi::add_immediate(&src, block, 8));
        self.emit(abi::add_registers(&src, &src, len));
        self.emit(abi::add_immediate(&dst, block, 8));
        self.emit(abi::add_registers(&dst, &dst, newlen));
        self.emit(abi::move_register(&left, len));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&left, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::subtract_immediate(&src, &src, 1));
        self.emit(abi::subtract_immediate(&dst, &dst, 1));
        self.emit(abi::load_u8(&byte, &src, 0));
        self.emit(abi::store_u8(&byte, &dst, 0));
        self.emit(abi::subtract_immediate(&left, &left, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
    }

    /// Write `pad_count` copies of the padChar block's bytes at `dst`.
    fn emit_pad_copies(
        &mut self,
        dst: &VirtualRegister,
        pad_slot: usize,
        pad_len_slot: usize,
        pad_count_slot: usize,
    ) {
        let pad = self.temporary_vreg();
        let pad_len = self.temporary_vreg();
        let count = self.temporary_vreg();
        let src = self.temporary_vreg();
        let left = self.temporary_vreg();
        let dst = *dst;
        let outer = self.label("inplace_str_pad_outer");
        let outer_done = self.label("inplace_str_pad_outer_done");
        let inner = self.label("inplace_str_pad_inner");
        let inner_done = self.label("inplace_str_pad_inner_done");
        let byte = self.temporary_vreg();
        self.emit(abi::load_u64(&count, abi::stack_pointer(), pad_count_slot));
        self.emit(abi::load_u64(&pad, abi::stack_pointer(), pad_slot));
        self.emit(abi::load_u64(&pad_len, abi::stack_pointer(), pad_len_slot));
        self.emit(abi::label(&outer));
        self.emit(abi::compare_immediate(&count, "0"));
        self.emit(abi::branch_eq(&outer_done));
        self.emit(abi::add_immediate(&src, &pad, 8));
        self.emit(abi::move_register(&left, &pad_len));
        self.emit(abi::label(&inner));
        self.emit(abi::compare_immediate(&left, "0"));
        self.emit(abi::branch_eq(&inner_done));
        self.emit(abi::load_u8(&byte, &src, 0));
        self.emit(abi::store_u8(&byte, &dst, 0));
        self.emit(abi::add_immediate(&src, &src, 1));
        self.emit(abi::add_immediate(&dst, &dst, 1));
        self.emit(abi::subtract_immediate(&left, &left, 1));
        self.emit(abi::branch(&inner));
        self.emit(abi::label(&inner_done));
        self.emit(abi::subtract_immediate(&count, &count, 1));
        self.emit(abi::branch(&outer));
        self.emit(abi::label(&outer_done));
    }
}
