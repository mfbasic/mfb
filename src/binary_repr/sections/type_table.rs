//! plan-176-O: the `.mfp` type table — interning source types and their payloads. The other
//! section tables and the native-library table codec live in `binary_repr/sections/mod.rs`.

use super::*;

impl TypeTable {
    pub(in crate::binary_repr) fn new() -> Self {
        Self {
            entries: Vec::new(),
            ids: HashMap::new(),
            foreign_types: HashMap::new(),
        }
    }

    pub(in crate::binary_repr) fn reserve_source_type(
        &mut self,
        strings: &mut StringPool,
        package: &str,
        ir_type: &IrType,
    ) -> u32 {
        let (kind, abi_export_kind) = match ir_type.kind.as_str() {
            "type" => (1, BinaryReprExportKind::Type),
            "union" => (2, BinaryReprExportKind::Union),
            "enum" => (3, BinaryReprExportKind::Enum),
            _ => (1, BinaryReprExportKind::Type),
        };
        let id = self.add_entry(strings, package, &ir_type.name, kind, Vec::new());
        // A builtin package's type rides in this IR because the source imports that
        // package; its declaration is `EXPORT` in the builtin's own source, but this
        // package does not own it and must not export it (bug-750). Every importer
        // gets the type from its own builtin import under the same spelling.
        if ir_type.visibility == "export" && !crate::ir::is_builtin_type_name(&ir_type.name) {
            self.entries[(id - FIRST_TABLE_TYPE_ID) as usize].abi_export_kind =
                Some(abi_export_kind);
        }
        id
    }

    pub(in crate::binary_repr) fn populate_source_payloads(
        &mut self,
        strings: &mut StringPool,
        ir_types: &[IrType],
    ) -> Result<(), String> {
        let source_types = ir_types
            .iter()
            .map(|ir_type| (ir_type.name.as_str(), ir_type))
            .collect::<HashMap<_, _>>();

        for ir_type in ir_types {
            let id = *self
                .ids
                .get(&ir_type.name)
                .ok_or_else(|| format!("source type `{}` was not reserved", ir_type.name))?;
            let payload = source_type_payload(strings, self, &source_types, ir_type)?;
            self.entries[(id - FIRST_TABLE_TYPE_ID) as usize].payload = payload;
        }

        Ok(())
    }

    /// The wire kind for an opaque entry.
    ///
    /// A name that OPENS a structural shape without parsing as one keeps the kind
    /// its old non-splitting `else` branch wrote (7/10/9); everything else is the
    /// plain record kind 1. Kept for wire compatibility — see [`Self::type_id`].
    fn opaque_entry_kind(name: &str) -> u16 {
        if name.starts_with("Thread OF ") {
            7
        } else if name.starts_with("ThreadWorker OF ") {
            10
        } else if name.starts_with("MapEntry OF ") {
            9
        } else if name.starts_with("Map OF ")
            || name.starts_with("List OF ")
            || name.starts_with("Set OF ")
            || name.starts_with("#Result OF ")
            || name.starts_with("FUNC(")
            || name.starts_with("ISOLATED FUNC(")
        {
            5
        } else {
            1
        }
    }

    /// The wire type id for `type_`, interning whatever the entry needs.
    ///
    /// plan-111-G: takes the TYPE, and the match is FLAT — the nested
    /// `match parse(name)` inside a `match name` collapses to one match on the
    /// variant, and `is_structural` (which only ever answered "which arm claims
    /// this spelling") goes with it.
    ///
    /// `opaque_structural_kind` does NOT go with it, and the first version of
    /// this change was wrong to delete it. A spelling that OPENS a structural
    /// shape but does not parse as one — `Thread OF Garbage`, `Map OF Garbage` —
    /// arrives here as a `Named`, and it used to intern with kind 7/10/9/5, not
    /// the plain kind 1 the fallback writes. That is a WIRE change, which
    /// plan-111-A §1 forbids, and `type_id_falls_back_for_malformed_composites`
    /// did not catch it because it asserted only that an id came back. The rule
    /// is kept, as the name-domain compatibility rule it always was, and that
    /// test now asserts the KINDS.
    ///
    /// Arm order is not observable: every arm is a distinct variant or a distinct
    /// nominal, except the leading STATE guard, which must stay first for the
    /// same reason it always did — `File STATE Cursor` is a composite of two ids,
    /// not an opaque name (plan-52-D §4). Erasing it would compile the exporter
    /// and silently degrade every importer to a bare `File`, because a consumer
    /// reads an imported signature from the ABI exports, not from the `.mfp`'s IR
    /// section.
    ///
    /// The ids are unchanged and pinned by
    /// `wire_type_ids_are_unchanged_by_the_typed_encoder`.
    pub(in crate::binary_repr) fn type_id(
        &mut self,
        strings: &mut StringPool,
        type_: &ParameterType,
    ) -> u32 {
        let rendered = type_.name();
        let name = rendered.as_ref();
        match type_ {
            // A resource carrying `STATE T` is a composite of two type ids (see
            // the doc comment above). Must stay the FIRST arm.
            _ if type_.state().is_some() => {
                let (base, state) = type_.split_state();
                let base = self.type_id(strings, &base);
                let state = self.type_id(strings, &state.expect("state() is Some here"));
                self.state_type(strings, base, state)
            }
            ParameterType::Nothing => TYPE_NOTHING,
            ParameterType::Boolean => TYPE_BOOLEAN,
            ParameterType::Integer => TYPE_INTEGER,
            ParameterType::Float => TYPE_FLOAT,
            ParameterType::Fixed => TYPE_FIXED,
            ParameterType::String => TYPE_STRING,
            ParameterType::Byte => TYPE_BYTE,
            ParameterType::Money => TYPE_MONEY,
            // Bare nominals: no variant, so matched by name.
            t if t.is_named("Scalar") => TYPE_SCALAR,
            t if t.is_named("fs.File") => TYPE_FILE_HANDLE,
            t if t.is_named("tcp.Socket") => TYPE_SOCKET_HANDLE,
            t if t.is_named("tcp.Listener") => TYPE_LISTENER_HANDLE,
            // plan-89-A: an opaque primitive-like type, identified on the wire by
            // its id alone (like `Scalar`/`Money`); its internal field layout is a
            // compiler-side hardcoded table, never serialized.
            t if t.is_named("AttributedString") => TYPE_ATTRIBUTED_STRING,
            t if t.is_named("Error") => {
                strings.intern("code");
                strings.intern("message");
                TYPE_ERROR
            }
            // Both spellings (bug-483). `term`'s registry row states the contract
            // this arm implements — "its wire id stays the reserved high-band
            // `TYPE_TERM_SIZE`, name-keyed in `binary_repr::sections`" — and
            // bug-480 Phase 4b started delivering `term.TermSize` from every member
            // signature. Matching the bare leaf alone dropped those through to the
            // opaque zero-field fallback below, so a package exporting a
            // `term::TermSize` encoded a record with no `columns`/`rows` at all.
            //
            // plan-122-F deleted the sibling `TermColor` arm along with the type.
            // `TYPE_TERM_COLOR` is RETIRED, not recycled: nothing encodes it any
            // more, but `binary_repr::reader` still decodes it so a `.mfp` published
            // before this change reports a recognizable type rather than failing
            // opaquely. `no_encoder_emits_the_retired_term_color_id` pins that.
            t if t.is_builtin_named("term", "TermSize") => {
                strings.intern("columns");
                strings.intern("rows");
                TYPE_TERM_SIZE
            }
            ParameterType::ListOf(element) => {
                let element = self.type_id(strings, element);
                self.list_type(strings, element)
            }
            // `Set OF T` (plan-63): a single element type id, kind 13. Distinct
            // from `List` (kind 4) so a decoded signature keeps the `Set`
            // spelling every front-end stage pattern-matches on.
            ParameterType::SetOf(element) => {
                let element = self.type_id(strings, element);
                self.set_type(strings, element)
            }
            ParameterType::ResultOf(success) => {
                let success = self.type_id(strings, success);
                self.result_type(strings, success)
            }
            ParameterType::ThreadHandle {
                worker,
                msg,
                res,
                out,
            } => {
                let message = self.type_id(strings, msg);
                // An absent resource plane is `Nothing`, which the wire encodes
                // as no plane at all (`thread_parts_full` returned `None`).
                let resource = match res.as_ref() {
                    ParameterType::Nothing => None,
                    res => Some(self.type_id(strings, res)),
                };
                let output = self.type_id(strings, out);
                if *worker {
                    self.thread_worker_type(strings, message, resource, output)
                } else {
                    self.thread_type(strings, message, resource, output)
                }
            }
            ParameterType::Func(_, _, _) => self.function_type(strings, name),
            // The `Map`/`MapEntry` split is the CANONICAL grammar's, not a local
            // `split_once(" TO ")` — which takes the LEFTMOST separator, the same
            // mis-split bug-108.2 fixed in the front end.
            // `Map OF Map OF String TO Integer TO Boolean` encoded key
            // `Map OF String` and value `Integer TO Boolean`, two types that do
            // not exist, and the table did not decode at all.
            ParameterType::MapOf(key, value) => {
                let key = self.type_id(strings, key);
                let value = self.type_id(strings, value);
                self.map_type(strings, key, value)
            }
            ParameterType::MapEntryOf(key, value) => {
                let key = self.type_id(strings, key);
                let value = self.type_id(strings, value);
                self.map_entry_type(strings, key, value)
            }
            _ => {
                if let Some(id) = self.ids.get(name) {
                    *id
                } else if let Some(fref) = self.foreign_types.get(name).cloned() {
                    // bug-390: a type owned by an imported dependency, named in this
                    // package's own API. Encode a foreign reference carrying the
                    // owning package's identity instead of degrading it to an
                    // empty-record placeholder (the old fallback below, which then
                    // failed with `truncated binary representation`).
                    self.foreign_type(strings, name, &fref)
                } else if let Some((bare, fref)) = name
                    .rsplit_once('.')
                    .and_then(|(_, bare)| self.foreign_types.get(bare).cloned().map(|f| (bare, f)))
                {
                    // bug-436: a package-qualified imported type (`leaf435::Node`)
                    // lowers to the dotted IR type name `leaf435.Node`, but the
                    // foreign-type identities are keyed by bare exported name
                    // (`Node`). Resolve the dotted reference to the same foreign
                    // entry the unqualified spelling produces — interned under the
                    // bare name so both spellings emit an identical type table —
                    // rather than degrading to an empty-record placeholder that
                    // fails read-back with `truncated binary representation`.
                    // (The composite-type keys use `#`, never `.`, so only a
                    // qualified `pkg.Type` reference reaches this arm.)
                    self.foreign_type(strings, bare, &fref)
                } else {
                    let kind = Self::opaque_entry_kind(name);
                    // bug-464 fallout: a kind-1 (record) entry's payload MUST
                    // begin with a u32 field count. An empty payload made the
                    // very first `checked_u32_at` on read-back overrun, which is
                    // the `truncated binary representation` that bug-390 and
                    // bug-436 each hit and each fixed only for their own case by
                    // routing around this arm.
                    //
                    // Everything still reaching it is opaque and genuinely
                    // field-less -- every BUILT-IN RESOURCE except the three
                    // `fs.File`/`tcp.Socket`/`tcp.Listener` spellings the match
                    // above names, so `udp::Socket`, `tls::Socket`,
                    // `tls::Listener`, `process::Process`, the audio handles and
                    // `canvas::Image` all landed here. A package exporting any of
                    // them failed to build, on clean main (fc5c8a6db), with that
                    // opaque error and no mention of the type.
                    //
                    // Zero-field record is exactly how `add_native` already
                    // encodes an opaque LINK resource ("so the type table
                    // round-trips"; its resource-ness comes from the
                    // RESOURCE_TABLE, not the type kind). This makes the two
                    // agree. No package that builds today changes bytes: the
                    // only entries affected are ones that could not be read back
                    // at all.
                    let payload = if kind == 1 {
                        let mut payload = Vec::new();
                        put_u32(&mut payload, 0);
                        payload
                    } else {
                        Vec::new()
                    };
                    self.add_entry(strings, "", name, kind, payload)
                }
            }
        }
    }

    pub(in crate::binary_repr) fn result_type(
        &mut self,
        strings: &mut StringPool,
        success_type: u32,
    ) -> u32 {
        let name = format!("Result#{success_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, success_type);
        self.add_entry(strings, "", &name, 6, payload)
    }

    /// A resource carrying a `STATE` payload: `{base_type, state_type}`, kind 11.
    /// Decodes back to `"<base> STATE <state>"` so an imported signature keeps the
    /// STATE its exporter declared.
    pub(in crate::binary_repr) fn state_type(
        &mut self,
        strings: &mut StringPool,
        base_type: u32,
        state_type: u32,
    ) -> u32 {
        let name = format!("State#{base_type}#{state_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, base_type);
        put_u32(&mut payload, state_type);
        self.add_entry(strings, "", &name, 11, payload)
    }

    pub(in crate::binary_repr) fn list_type(
        &mut self,
        strings: &mut StringPool,
        element_type: u32,
    ) -> u32 {
        let name = format!("List#{element_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, element_type);
        self.add_entry(strings, "", &name, 4, payload)
    }

    /// `Set OF T` (plan-63): a single element type id, kind 13. Mirrors
    /// [`list_type`] structurally (one payload id) but keeps a distinct kind so
    /// the decoder can reconstruct the `Set` spelling.
    pub(in crate::binary_repr) fn set_type(
        &mut self,
        strings: &mut StringPool,
        element_type: u32,
    ) -> u32 {
        let name = format!("Set#{element_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, element_type);
        self.add_entry(strings, "", &name, 13, payload)
    }

    pub(in crate::binary_repr) fn map_type(
        &mut self,
        strings: &mut StringPool,
        key_type: u32,
        value_type: u32,
    ) -> u32 {
        let name = format!("Map#{key_type}#{value_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, key_type);
        put_u32(&mut payload, value_type);
        self.add_entry(strings, "", &name, 5, payload)
    }

    pub(in crate::binary_repr) fn map_entry_type(
        &mut self,
        strings: &mut StringPool,
        key_type: u32,
        value_type: u32,
    ) -> u32 {
        let name = format!("MapEntry#{key_type}#{value_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, key_type);
        put_u32(&mut payload, value_type);
        self.add_entry(strings, "", &name, 9, payload)
    }

    pub(in crate::binary_repr) fn function_type(
        &mut self,
        strings: &mut StringPool,
        name: &str,
    ) -> u32 {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let mut payload = Vec::new();
        if let Some(signature) = parse_function_type(name) {
            put_u32(&mut payload, if signature.isolated { 1 } else { 0 });
            put_u32(&mut payload, signature.params.len() as u32);
            let return_type = self.type_id(strings, &ParameterType::declared(&signature.returns));
            put_u32(&mut payload, return_type);
            for param in signature.params {
                let param_type = self.type_id(strings, &ParameterType::declared(&param));
                put_u32(&mut payload, param_type);
            }
        }
        self.add_entry(strings, "", name, 8, payload)
    }

    pub(in crate::binary_repr) fn thread_type(
        &mut self,
        strings: &mut StringPool,
        message_type: u32,
        resource_type: Option<u32>,
        output_type: u32,
    ) -> u32 {
        // A data-only thread encodes exactly as before (message, output); the
        // resource type-id is appended only when the resource plane is present,
        // keeping data-only packages byte-compatible.
        let resource_key = resource_type.map_or(String::new(), |id| format!("#r{id}"));
        let name = format!("Thread#{message_type}#{output_type}{resource_key}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, message_type);
        put_u32(&mut payload, output_type);
        if let Some(resource_type) = resource_type {
            put_u32(&mut payload, resource_type);
        }
        self.add_entry(strings, "thread", &name, 7, payload)
    }

    pub(in crate::binary_repr) fn thread_worker_type(
        &mut self,
        strings: &mut StringPool,
        message_type: u32,
        resource_type: Option<u32>,
        output_type: u32,
    ) -> u32 {
        let resource_key = resource_type.map_or(String::new(), |id| format!("#r{id}"));
        let name = format!("ThreadWorker#{message_type}#{output_type}{resource_key}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, message_type);
        put_u32(&mut payload, output_type);
        if let Some(resource_type) = resource_type {
            put_u32(&mut payload, resource_type);
        }
        self.add_entry(strings, "thread", &name, 10, payload)
    }

    /// bug-390: intern a reference to a dependency's exported type. The entry's
    /// `owner_package` is the declaring dependency and its payload is
    /// `[u16 underlying-export-kind][32-byte owning ABI hash]` — enough to
    /// re-export the type by the owning package's original identity and to
    /// reconstruct its name on decode, without carrying (absent) field data.
    pub(in crate::binary_repr) fn foreign_type(
        &mut self,
        strings: &mut StringPool,
        name: &str,
        fref: &ForeignTypeRef,
    ) -> u32 {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let mut payload = Vec::new();
        put_u16(&mut payload, encode_export_kind(fref.export_kind));
        payload.extend_from_slice(&fref.abi_hash);
        self.add_entry(strings, &fref.package, name, FOREIGN_TYPE_KIND, payload)
    }

    pub(in crate::binary_repr) fn add_entry(
        &mut self,
        strings: &mut StringPool,
        package: &str,
        name: &str,
        kind: u16,
        payload: Vec<u8>,
    ) -> u32 {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let id = FIRST_TABLE_TYPE_ID + self.entries.len() as u32;
        self.ids.insert(name.to_string(), id);
        self.entries.push(TypeEntry {
            kind,
            name: strings.intern(name),
            owner_package: strings.intern(package),
            abi_export_kind: None,
            payload,
        });
        id
    }

    pub(in crate::binary_repr) fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        let entry_bytes = 20usize;
        let mut payload_offset = 4 + self.entries.len() * entry_bytes;
        put_u32(&mut bytes, self.entries.len() as u32);
        for entry in &self.entries {
            put_u16(&mut bytes, entry.kind);
            put_u16(&mut bytes, 0);
            put_u32(&mut bytes, entry.name);
            put_u32(&mut bytes, entry.owner_package);
            put_u32(&mut bytes, payload_offset as u32);
            put_u32(&mut bytes, entry.payload.len() as u32);
            payload_offset += entry.payload.len();
        }
        for entry in &self.entries {
            bytes.extend_from_slice(&entry.payload);
        }
        bytes
    }

    /// bug-390: mark every foreign-reference entry (kind 12) reachable from one of
    /// this package's own exported symbols as re-exported, so a consumer importing
    /// this package sees the dependency's type under its original identity (true
    /// namespace re-export). A foreign type reached only through an imported
    /// function's signature — interned for table-order stability but never named
    /// in this package's own API — is deliberately left unexported (the acceptance
    /// model's "pB never re-exports the unused `C`").
    pub(in crate::binary_repr) fn mark_reexported_foreign_types(
        &mut self,
        functions: &[Function],
    ) -> Result<(), String> {
        let mut reachable = HashSet::new();
        for function in functions {
            if !is_exported_function(function) {
                continue;
            }
            self.collect_reachable(function.return_type, &mut reachable)?;
            for param in &function.params {
                self.collect_reachable(param.type_id, &mut reachable)?;
            }
        }
        // An exported record/union surfaces its field types too, so a foreign type
        // reached only through an exported type's field is still re-exported.
        for index in 0..self.entries.len() {
            if self.entries[index].abi_export_kind.is_some() {
                self.collect_reachable(FIRST_TABLE_TYPE_ID + index as u32, &mut reachable)?;
            }
        }
        for id in reachable {
            let index = (id - FIRST_TABLE_TYPE_ID) as usize;
            let Some(entry) = self.entries.get_mut(index) else {
                continue;
            };
            if entry.kind == FOREIGN_TYPE_KIND && entry.abi_export_kind.is_none() {
                entry.abi_export_kind =
                    Some(decode_export_kind(checked_u16_at(&entry.payload, 0)?)?);
            }
        }
        Ok(())
    }

    /// Collect every table type id reachable from `id` (including `id` itself),
    /// mirroring `AbiSerializer::serialize_type_inner`'s payload traversal.
    fn collect_reachable(&self, id: u32, acc: &mut HashSet<u32>) -> Result<(), String> {
        let Some(index) = id.checked_sub(FIRST_TABLE_TYPE_ID) else {
            return Ok(()); // primitive / handle sentinel: no table entry.
        };
        let index = index as usize;
        if index >= self.entries.len() || !acc.insert(id) {
            return Ok(());
        }
        let payload = self.entries[index].payload.clone();
        match self.entries[index].kind {
            1 => {
                let mut offset = 0;
                let field_count = cursor_u32(&payload, &mut offset)?;
                for _ in 0..field_count {
                    let _name = cursor_u32(&payload, &mut offset)?;
                    let type_id = cursor_u32(&payload, &mut offset)?;
                    let _visibility = cursor_u32(&payload, &mut offset)?;
                    self.collect_reachable(type_id, acc)?;
                }
            }
            2 => {
                let mut offset = 0;
                let variant_count = cursor_u32(&payload, &mut offset)?;
                for _ in 0..variant_count {
                    let _name = cursor_u32(&payload, &mut offset)?;
                    let field_count = cursor_u32(&payload, &mut offset)?;
                    for _ in 0..field_count {
                        let _field_name = cursor_u32(&payload, &mut offset)?;
                        let field_type = cursor_u32(&payload, &mut offset)?;
                        self.collect_reachable(field_type, acc)?;
                    }
                }
            }
            4 | 6 => self.collect_reachable(checked_u32_at(&payload, 0)?, acc)?,
            5 | 9 | 11 => {
                self.collect_reachable(checked_u32_at(&payload, 0)?, acc)?;
                self.collect_reachable(checked_u32_at(&payload, 4)?, acc)?;
            }
            7 | 10 => {
                self.collect_reachable(checked_u32_at(&payload, 0)?, acc)?;
                self.collect_reachable(checked_u32_at(&payload, 4)?, acc)?;
                if payload.len() >= 12 {
                    self.collect_reachable(checked_u32_at(&payload, 8)?, acc)?;
                }
            }
            8 => {
                let mut offset = 0;
                let _isolated = cursor_u32(&payload, &mut offset)?;
                let param_count = cursor_u32(&payload, &mut offset)?;
                let return_type = cursor_u32(&payload, &mut offset)?;
                self.collect_reachable(return_type, acc)?;
                for _ in 0..param_count {
                    let param_type = cursor_u32(&payload, &mut offset)?;
                    self.collect_reachable(param_type, acc)?;
                }
            }
            // enum (3), foreign (12), and any leaf kind: no outgoing type refs.
            _ => {}
        }
        Ok(())
    }
}
