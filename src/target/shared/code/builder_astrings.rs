use super::*;

impl CodeBuilder<'_> {
    /// Dispatch an `astrings::` package call to its native lowering, or `None`
    /// when `target` is not an `astrings` member (so the caller falls through to
    /// the generic call path). plan-89-A ships only `fromString`; letters B–E add
    /// the mutation/query/render members here.
    pub(super) fn lower_astrings_package_call(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Result<Option<ValueResult>, String> {
        if target == "astrings.fromString" && args.len() == 1 {
            return Ok(Some(self.lower_astrings_from_string(args)?));
        }
        Ok(None)
    }

    /// `astrings::fromString(text)` — build an `AttributedString` whose visible
    /// `text` field is a deep copy of the argument String and whose `spans`
    /// overlay is an empty list. Both fields are constructed through the generic
    /// inlined-record builder (`emit_build_inlined_record`), so the value's byte
    /// layout is identical to every other `AttributedString` and its
    /// value-semantic copy/drop reuse the generic record machinery.
    fn lower_astrings_from_string(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let text = self.lower_value(&args[0])?;
        if text.type_ != "String" {
            return Err(format!(
                "astrings::fromString expects a String argument, got {}",
                text.type_
            ));
        }
        let text = self.materialize_value(text)?;
        let text_slot = self.allocate_stack_object("astrings_from_string_text", 8);
        self.emit(abi::store_u64(&text.location, abi::stack_pointer(), text_slot));

        // The `spans` overlay field: an empty `List OF Integer` (the plan-89-A
        // placeholder element type; plan-89-B refines it). `emit_build_inlined_record`
        // inlines this flat list into the record's data region.
        let spans = self.lower_empty_collection("List OF Integer")?;
        let spans_slot = self.allocate_stack_object("astrings_from_string_spans", 8);
        self.emit(abi::store_u64(&spans.location, abi::stack_pointer(), spans_slot));

        let register = self.emit_build_inlined_record("AttributedString", &[text_slot, spans_slot])?;
        Ok(ValueResult {
            type_: "AttributedString".to_string(),
            location: Operand::from(register.render()),
            text: format!("astrings::fromString({})", text.text),
        })
    }
}
