// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    /// Dispatch an `astrings::` package call to its native lowering, or `None`
    /// when `target` is not a native `astrings` member (so the caller falls
    /// through to the generic call path — the `Attribute`-model and Tier-C members
    /// are `.mfb` source-companion bodies, not native).
    pub(crate) fn lower_astrings_package_call(
        &mut self,
        target: &str,
        args: &[ValueResult],
    ) -> Result<Option<ValueResult>, String> {
        if target == "astrings.fromString" && args.len() == 1 {
            return Ok(Some(self.lower_astrings_from_string(args)?));
        }
        if target == "astrings.readSpans" && args.len() == 1 {
            return Ok(Some(self.lower_astrings_read_spans(args)?));
        }
        if target == "astrings.writeSpans" && args.len() == 2 {
            return Ok(Some(self.lower_astrings_write_spans(args)?));
        }
        if target == "astrings.scalarLen" && args.len() == 1 {
            return Ok(Some(self.lower_astrings_scalar_len(args)?));
        }
        Ok(None)
    }

    /// `astrings::fromString(text)` — build an `AttributedString` whose visible
    /// `text` field is a deep copy of the argument String and whose `spans`
    /// overlay is an empty `List OF AttrSpan`. Both fields are constructed through
    /// the generic inlined-record builder (`emit_build_inlined_record`), so the
    /// value's byte layout is identical to every other `AttributedString` and its
    /// value-semantic copy/drop reuse the generic record machinery.
    fn lower_astrings_from_string(&mut self, args: &[ValueResult]) -> Result<ValueResult, String> {
        let text = args[0].clone();
        if text.type_ != ParameterType::String {
            return Err(format!(
                "astrings::fromString expects a String argument, got {}",
                text.type_
            ));
        }
        let text = self.materialize_value(text)?;
        let text_slot = self.allocate_stack_object("astrings_from_string_text", 8);
        self.emit(abi::store_u64(
            &text.location,
            abi::stack_pointer(),
            text_slot,
        ));

        // An empty overlay: `List OF AttrSpan`. `emit_build_inlined_record` inlines
        // this flat list into the record's data region.
        let spans = self.lower_empty_collection("List OF AttrSpan")?;
        let spans_slot = self.allocate_stack_object("astrings_from_string_spans", 8);
        self.emit(abi::store_u64(
            &spans.location,
            abi::stack_pointer(),
            spans_slot,
        ));

        let register =
            self.emit_build_inlined_record("AttributedString", &[text_slot, spans_slot])?;
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::parse("AttributedString"),
            location: Operand::from(register.render()),
            text: format!("astrings::fromString({})", text.text),
        })
    }

    /// Read the inlined field at `field_index` of an `AttributedString` record as
    /// an alias pointer. Every `AttributedString` field is inlined (the visible
    /// `text` String and the flat `spans` list), so the slot holds a block-relative
    /// offset and the sub-block pointer is `record + offset` (plan-02 §4.2).
    fn emit_attributed_string_field_ptr(
        &mut self,
        record: &Operand,
        field_index: usize,
    ) -> Result<VirtualRegister, String> {
        let ptr = self.allocate_register();
        self.emit(abi::load_u64(&ptr, record, 8 * field_index));
        self.emit(abi::add_registers(&ptr, record, &ptr));
        Ok(ptr)
    }

    /// `astrings::readSpans(a)` (internal) — return an owned deep copy of the
    /// overlay list (`List OF AttrSpan`). The overlay is inlined in the record, so
    /// the alias is deep-copied out; the companion mutates the copy freely and
    /// writes it back with `writeSpans`.
    fn lower_astrings_read_spans(&mut self, args: &[ValueResult]) -> Result<ValueResult, String> {
        let value = args[0].clone();
        let record_slot = self.allocate_stack_object("astrings_read_spans_record", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            record_slot,
        ));
        let record = self.allocate_register();
        self.emit(abi::load_u64(&record, abi::stack_pointer(), record_slot));
        let record_op = Operand::from(record.render());
        let spans_alias = self.emit_attributed_string_field_ptr(&record_op, 1)?;
        let copied = self.copy_value_to_current_arena("List OF AttrSpan", &spans_alias)?;
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::parse("List OF AttrSpan"),
            location: Operand::from(copied.render()),
            text: format!("astrings::readSpans({})", value.text),
        })
    }

    /// `astrings::scalarLen(a)` (internal) — the number of Unicode scalars in the
    /// visible text, for inclusive-range bounds validation in the companion. Counts
    /// non-continuation UTF-8 bytes in the inlined `text` String, reusing the shared
    /// scalar-count walk (no strings/encoding companion dependency).
    fn lower_astrings_scalar_len(&mut self, args: &[ValueResult]) -> Result<ValueResult, String> {
        let value = args[0].clone();
        let record_slot = self.allocate_stack_object("astrings_scalar_len_record", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            record_slot,
        ));
        let record = self.allocate_register();
        self.emit(abi::load_u64(&record, abi::stack_pointer(), record_slot));
        let record_op = Operand::from(record.render());
        let text_ptr = self.emit_attributed_string_field_ptr(&record_op, 0)?;

        let length = self.temporary_vreg();
        let data = self.temporary_vreg();
        let index = self.temporary_vreg();
        let count = self.allocate_register();
        let addr = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let mask = self.temporary_vreg();
        // String object: `{ U64 byteLength; utf8Bytes; U8 nul }`.
        self.emit(abi::load_u64(&length, &text_ptr, 0));
        self.emit(abi::add_immediate(&data, &text_ptr, 8));
        let loop_label = self.label("astrings_scalar_len_loop");
        let not_cont = self.label("astrings_scalar_len_not_cont");
        let after = self.label("astrings_scalar_len_after");
        let done = self.label("astrings_scalar_len_done");
        self.emit_scalar_count_loop(
            &data,
            &index,
            &count,
            &addr,
            &byte,
            &mask,
            &length,
            &loop_label,
            &not_cont,
            &after,
            &done,
        );
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Integer,
            location: Operand::from(count.render()),
            text: format!("astrings::scalarLen({})", value.text),
        })
    }

    /// `astrings::writeSpans(a, spans)` (internal) — build a new `AttributedString`
    /// carrying `a`'s visible text (deep-copied) and the supplied overlay list.
    fn lower_astrings_write_spans(&mut self, args: &[ValueResult]) -> Result<ValueResult, String> {
        let value = args[0].clone();
        let record_slot = self.allocate_stack_object("astrings_write_spans_record", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            record_slot,
        ));
        let record = self.allocate_register();
        self.emit(abi::load_u64(&record, abi::stack_pointer(), record_slot));
        let record_op = Operand::from(record.render());
        let text_alias = self.emit_attributed_string_field_ptr(&record_op, 0)?;
        let text_slot = self.allocate_stack_object("astrings_write_spans_text", 8);
        self.emit(abi::store_u64(&text_alias, abi::stack_pointer(), text_slot));

        let spans = args[1].clone();
        if spans.type_.name() != "List OF AttrSpan" {
            return Err(format!(
                "astrings::writeSpans expects a List OF AttrSpan, got {}",
                spans.type_
            ));
        }
        let spans = self.materialize_value(spans)?;
        let spans_slot = self.allocate_stack_object("astrings_write_spans_spans", 8);
        self.emit(abi::store_u64(
            &spans.location,
            abi::stack_pointer(),
            spans_slot,
        ));

        let register =
            self.emit_build_inlined_record("AttributedString", &[text_slot, spans_slot])?;
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::parse("AttributedString"),
            location: Operand::from(register.render()),
            text: format!("astrings::writeSpans({}, {})", value.text, spans.text),
        })
    }
}
