//! plan-176-O: the ABI-index serializer — reachable types and their encoding. The other section
//! tables and the native-library table codec live in `binary_repr/sections/mod.rs`.

use super::*;

impl<'a> AbiSerializer<'a> {
    pub(in crate::binary_repr) fn new(
        strings: &'a [String],
        types: &'a TypeTable,
        constants: &'a ConstPool,
    ) -> Self {
        Self {
            strings,
            types,
            constants,
            bytes: Vec::new(),
            type_refs: HashMap::new(),
            next_ref: 0,
            depth: 0,
        }
    }

    pub(in crate::binary_repr) fn serialize_type(&mut self, id: u32) -> Result<(), String> {
        // Depth cap (bug-153): reject a deep acyclic type chain before it
        // overflows the native stack. The `type_refs` cycle guard only rejects
        // repeated ids, so a separate counter is needed. Balanced decrement on
        // the success path; an over-deep graph aborts the whole serialization.
        self.depth += 1;
        if self.depth > MAX_TYPE_GRAPH_DEPTH {
            return Err(format!(
                "type graph too deep (exceeds {MAX_TYPE_GRAPH_DEPTH})"
            ));
        }
        let result = self.serialize_type_inner(id);
        self.depth -= 1;
        result
    }

    fn serialize_type_inner(&mut self, id: u32) -> Result<(), String> {
        if let Some(primitive) = primitive_type_name(id) {
            self.put_u8(1);
            self.put_u32(id);
            self.put_str(primitive);
            return Ok(());
        }

        if let Some(ref_id) = self.type_refs.get(&id).copied() {
            self.put_u8(2);
            self.put_u32(ref_id);
            return Ok(());
        }

        let entry = id
            .checked_sub(FIRST_TABLE_TYPE_ID)
            .and_then(|index| self.types.entries.get(index as usize))
            .ok_or_else(|| format!("unknown type id {id}"))?;
        let ref_id = self.next_ref;
        self.next_ref = self
            .next_ref
            .checked_add(1)
            .ok_or_else(|| "ABI type graph has too many nodes".to_string())?;
        self.type_refs.insert(id, ref_id);

        self.put_u8(3);
        self.put_u32(ref_id);
        self.put_u16(entry.kind);
        match entry.kind {
            1 => self.serialize_record_type(entry),
            2 => self.serialize_union_type(entry),
            3 => self.serialize_enum_type(entry),
            4 => {
                self.put_str("list");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)
            }
            5 => {
                self.put_str("map");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)?;
                self.serialize_type(checked_u32_at(&entry.payload, 4)?)
            }
            6 => {
                self.put_str("result");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)
            }
            7 => {
                self.put_str("thread");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)?;
                self.serialize_type(checked_u32_at(&entry.payload, 4)?)?;
                // The resource plane (if present) is part of the signature hash.
                if entry.payload.len() >= 12 {
                    self.serialize_type(checked_u32_at(&entry.payload, 8)?)?;
                }
                Ok(())
            }
            8 => self.serialize_function_type(entry),
            // bug-390: a reference to a dependency's type. Hash it by the owning
            // package's identity — its dependency name, original type name, and the
            // owning package's ABI hash carried in the payload — rather than
            // re-walking fields it does not have. Two intermediary packages that
            // surface the same `pA::A` therefore contribute identical bytes here, so
            // a consumer unifies them; and an intermediary built against an
            // ABI-incompatible owner carries a different hash, which the consumer's
            // `validate_abi_index` recompute rejects.
            FOREIGN_TYPE_KIND => {
                self.put_str("foreign");
                self.put_str(string_at(self.strings, entry.owner_package)?);
                self.put_str(string_at(self.strings, entry.name)?);
                // payload = [u16 underlying-export-kind][32-byte owning ABI hash].
                let hash = entry
                    .payload
                    .get(2..2 + ABI_HASH_LEN)
                    .ok_or("truncated binary representation")?;
                self.bytes.extend_from_slice(hash);
                Ok(())
            }
            // bug-277: a resource carrying `STATE T` (kind 11) is a composite of
            // two type ids and must hash structurally like kinds 4/5/7. Under the
            // opaque fallback it hashed its interned name `State#<baseId>#<stateId>`,
            // which embeds table-position-dependent ids: an unrelated renumber
            // changed the hash with no semantic change, and a change to the STATE
            // record's own shape left the hash identical.
            11 => {
                self.put_str("state");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)?;
                self.serialize_type(checked_u32_at(&entry.payload, 4)?)
            }
            // bug-785: MapEntry (9), ThreadWorker (10) and Set (13) are composites
            // too, and fell to the opaque fallback the same way kind 11 did before
            // bug-277: their interned names (`Set#<elemId>`, ...) embed
            // table-position ids, so an unrelated renumber changed the hash and a
            // change to a component's shape did not.
            9 => {
                self.put_str("mapentry");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)?;
                self.serialize_type(checked_u32_at(&entry.payload, 4)?)
            }
            10 => {
                self.put_str("threadworker");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)?;
                self.serialize_type(checked_u32_at(&entry.payload, 4)?)?;
                // The resource plane (if present) is part of the signature hash.
                if entry.payload.len() >= 12 {
                    self.serialize_type(checked_u32_at(&entry.payload, 8)?)?;
                }
                Ok(())
            }
            13 => {
                self.put_str("set");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)
            }
            // Every kind the table defines is hashed above; an unknown kind from a
            // decoded table can only be named.
            _ => {
                self.put_str("opaque");
                self.put_str(string_at(self.strings, entry.name)?);
                Ok(())
            }
        }
    }

    pub(in crate::binary_repr) fn serialize_record_type(
        &mut self,
        entry: &TypeEntry,
    ) -> Result<(), String> {
        self.put_str("record");
        let mut offset = 0;
        let field_count = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(field_count);
        for _ in 0..field_count {
            let name = cursor_u32(&entry.payload, &mut offset)?;
            let type_id = cursor_u32(&entry.payload, &mut offset)?;
            let _visibility = cursor_u32(&entry.payload, &mut offset)?;
            self.put_str(string_at(self.strings, name)?);
            self.serialize_type(type_id)?;
            self.put_u32(_visibility);
        }
        Ok(())
    }

    pub(in crate::binary_repr) fn serialize_union_type(
        &mut self,
        entry: &TypeEntry,
    ) -> Result<(), String> {
        self.put_str("union");
        let mut offset = 0;
        let variant_count = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(variant_count);
        for _ in 0..variant_count {
            let name = cursor_u32(&entry.payload, &mut offset)?;
            self.put_str(string_at(self.strings, name)?);
            let field_count = cursor_u32(&entry.payload, &mut offset)?;
            self.put_u32(field_count);
            for _ in 0..field_count {
                let field_name = cursor_u32(&entry.payload, &mut offset)?;
                let field_type = cursor_u32(&entry.payload, &mut offset)?;
                self.put_str(string_at(self.strings, field_name)?);
                self.serialize_type(field_type)?;
            }
        }
        Ok(())
    }

    pub(in crate::binary_repr) fn serialize_enum_type(
        &mut self,
        entry: &TypeEntry,
    ) -> Result<(), String> {
        self.put_str("enum");
        let mut offset = 0;
        let member_count = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(member_count);
        for _ in 0..member_count {
            let name = cursor_u32(&entry.payload, &mut offset)?;
            let ordinal = cursor_u32(&entry.payload, &mut offset)?;
            self.put_str(string_at(self.strings, name)?);
            self.put_u32(ordinal);
        }
        Ok(())
    }

    pub(in crate::binary_repr) fn serialize_function_type(
        &mut self,
        entry: &TypeEntry,
    ) -> Result<(), String> {
        self.put_str("function-type");
        let mut offset = 0;
        let isolated = cursor_u32(&entry.payload, &mut offset)?;
        let param_count = cursor_u32(&entry.payload, &mut offset)?;
        let return_type = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(isolated);
        self.put_u32(param_count);
        self.serialize_type(return_type)?;
        for _ in 0..param_count {
            self.serialize_type(cursor_u32(&entry.payload, &mut offset)?)?;
        }
        Ok(())
    }

    pub(in crate::binary_repr) fn serialize_const(&mut self, id: u32) -> Result<(), String> {
        let constant = self
            .constants
            .entries
            .get(id as usize)
            .ok_or_else(|| format!("unknown const id {id}"))?;
        self.put_u16(constant.kind);
        match constant.kind {
            6 => {
                let string_id = checked_u32_at(&constant.payload, 0)?;
                self.put_str(string_at(self.strings, string_id)?);
            }
            _ => {
                self.put_u32(constant.payload.len() as u32);
                self.bytes.extend_from_slice(&constant.payload);
            }
        }
        Ok(())
    }

    pub(in crate::binary_repr) fn put_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(in crate::binary_repr) fn put_u16(&mut self, value: u16) {
        put_u16(&mut self.bytes, value);
    }

    pub(in crate::binary_repr) fn put_u32(&mut self, value: u32) {
        put_u32(&mut self.bytes, value);
    }

    pub(in crate::binary_repr) fn put_str(&mut self, value: &str) {
        put_bytes(&mut self.bytes, value.as_bytes());
    }
}
