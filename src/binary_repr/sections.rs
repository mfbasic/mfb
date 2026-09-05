use super::*;
use crate::types::ParameterType;

impl StringPool {
    pub(super) fn new() -> Self {
        Self { values: Vec::new() }
    }

    pub(super) fn intern(&mut self, value: &str) -> u32 {
        if let Some(index) = self.values.iter().position(|existing| existing == value) {
            return index as u32;
        }
        let index = self.values.len() as u32;
        self.values.push(value.to_string());
        index
    }

    pub(super) fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.values.len() as u32);
        for value in &self.values {
            put_bytes(&mut bytes, value.as_bytes());
        }
        bytes
    }
}

impl TypeTable {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
            ids: HashMap::new(),
            foreign_types: HashMap::new(),
        }
    }

    pub(super) fn reserve_source_type(
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
        if ir_type.visibility == "export" {
            self.entries[(id - FIRST_TABLE_TYPE_ID) as usize].abi_export_kind =
                Some(abi_export_kind);
        }
        id
    }

    pub(super) fn populate_source_payloads(
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
            || name.starts_with("Result OF ")
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
    pub(super) fn type_id(&mut self, strings: &mut StringPool, type_: &ParameterType) -> u32 {
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

    pub(super) fn result_type(&mut self, strings: &mut StringPool, success_type: u32) -> u32 {
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
    pub(super) fn state_type(
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

    pub(super) fn list_type(&mut self, strings: &mut StringPool, element_type: u32) -> u32 {
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
    pub(super) fn set_type(&mut self, strings: &mut StringPool, element_type: u32) -> u32 {
        let name = format!("Set#{element_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, element_type);
        self.add_entry(strings, "", &name, 13, payload)
    }

    pub(super) fn map_type(
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

    pub(super) fn map_entry_type(
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

    pub(super) fn function_type(&mut self, strings: &mut StringPool, name: &str) -> u32 {
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

    pub(super) fn thread_type(
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

    pub(super) fn thread_worker_type(
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
    pub(super) fn foreign_type(
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

    pub(super) fn add_entry(
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

    pub(super) fn encode(&self) -> Vec<u8> {
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
    pub(super) fn mark_reexported_foreign_types(
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

impl ConstPool {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(super) fn add(&mut self, strings: &mut StringPool, value: &IrValue) -> Result<u32, String> {
        let entry = match value {
            IrValue::Const { type_, value } => match type_ {
                ParameterType::Nothing => ConstEntry {
                    kind: 1,
                    payload: Vec::new(),
                },
                ParameterType::String => {
                    let mut payload = Vec::new();
                    put_u32(&mut payload, strings.intern(value));
                    ConstEntry { kind: 6, payload }
                }
                ParameterType::Integer => ConstEntry {
                    kind: 3,
                    payload: value
                        .parse::<i64>()
                        .map_err(|_| format!("invalid Integer constant `{value}`"))?
                        .to_le_bytes()
                        .to_vec(),
                },
                ParameterType::Float => ConstEntry {
                    kind: 4,
                    payload: value
                        .parse::<f64>()
                        .map_err(|_| format!("invalid Float constant `{value}`"))?
                        .to_bits()
                        .to_le_bytes()
                        .to_vec(),
                },
                ParameterType::Fixed => ConstEntry {
                    kind: 5,
                    payload: fixed_raw_from_decimal(value)?.to_le_bytes().to_vec(),
                },
                // Money's `kind` is its wire type id (`TYPE_MONEY` = 9); the raw
                // is the exact base-10 scaled i64 (plan-29-B §4.3).
                ParameterType::Money => ConstEntry {
                    kind: TYPE_MONEY as u16,
                    payload: crate::numeric::money_raw_from_decimal(value)?
                        .to_le_bytes()
                        .to_vec(),
                },
                ParameterType::Boolean => ConstEntry {
                    kind: 2,
                    payload: vec![if value == "true" { 1 } else { 0 }],
                },
                ParameterType::Byte => ConstEntry {
                    kind: 7,
                    payload: vec![value
                        .parse::<u8>()
                        .map_err(|_| format!("invalid Byte constant `{value}`"))?],
                },
                // Scalar's `kind` is its wire type id (`TYPE_SCALAR` = 10); the
                // payload is the 4-byte LE Unicode codepoint (plan-41-B §3).
                t if t.is_named("Scalar") => ConstEntry {
                    kind: TYPE_SCALAR as u16,
                    payload: value
                        .parse::<u32>()
                        .map_err(|_| format!("invalid Scalar constant `{value}`"))?
                        .to_le_bytes()
                        .to_vec(),
                },
                _ => return Err(format!("unsupported constant type `{type_}`")),
            },
            _ => return Err("only constant IR values can be stored in CONST_POOL".to_string()),
        };

        let id = self.entries.len() as u32;
        self.entries.push(entry);
        Ok(id)
    }

    pub(super) fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.entries.len() as u32);
        for entry in &self.entries {
            put_u16(&mut bytes, entry.kind);
            put_u16(&mut bytes, 0);
            put_bytes(&mut bytes, &entry.payload);
        }
        bytes
    }
}

impl ResourceTable {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(super) fn add_standard_file(&mut self, types: &mut TypeTable, strings: &mut StringPool) {
        let type_ = ParameterType::named(crate::codegen::builtins::fs::FILE_TYPE_ID);
        let type_id = types.type_id(strings, &type_);
        self.entries.push(ResourceEntry {
            type_id,
            close_function_id: BUILTIN_FS_CLOSE_FUNCTION_ID,
            flags: standard_resource_flags(&type_),
        });
    }

    pub(super) fn add_standard_socket(&mut self, types: &mut TypeTable, strings: &mut StringPool) {
        let type_ = ParameterType::named(crate::codegen::builtins::tcp::SOCKET_TYPE_ID);
        let type_id = types.type_id(strings, &type_);
        self.entries.push(ResourceEntry {
            type_id,
            close_function_id: BUILTIN_STREAM_CLOSE_FUNCTION_ID,
            flags: standard_resource_flags(&type_),
        });
    }

    pub(super) fn add_standard_listener(
        &mut self,
        types: &mut TypeTable,
        strings: &mut StringPool,
    ) {
        let type_ = ParameterType::named(crate::codegen::builtins::tcp::LISTENER_TYPE_ID);
        let type_id = types.type_id(strings, &type_);
        self.entries.push(ResourceEntry {
            type_id,
            close_function_id: BUILTIN_STREAM_CLOSE_FUNCTION_ID,
            flags: standard_resource_flags(&type_),
        });
    }

    /// Add the `RESOURCE_TABLE` entry for any OTHER built-in resource — the ones
    /// the three `add_standard_*` helpers above do not name (bug-464 fallout:
    /// `udp::Socket`, `tls::Socket`, `tls::Listener`, `process::Process`, the
    /// audio handles, `canvas::Image`). Its close op resolves from the registry
    /// by type name at decode, via [`BUILTIN_RESOURCE_CLOSE_BY_TYPE`].
    ///
    /// The three legacy types keep their own helpers and their own sentinels so
    /// their encoded bytes do not move.
    pub(super) fn add_standard_other(
        &mut self,
        types: &mut TypeTable,
        strings: &mut StringPool,
        type_: &ParameterType,
    ) {
        let type_id = types.type_id(strings, type_);
        self.entries.push(ResourceEntry {
            type_id,
            close_function_id: BUILTIN_RESOURCE_CLOSE_BY_TYPE,
            flags: standard_resource_flags(type_),
        });
    }

    /// Add a native LINK resource (plan-link-update.md §10). Native resources
    /// carry the `NATIVE` flag *without* `STANDARD`, which is how decode tells a
    /// native LINK resource (whose `close_function_id` is the string id of its
    /// close op name) from a built-in (whose id is a sentinel).
    pub(super) fn add_native(
        &mut self,
        strings: &mut StringPool,
        type_id: u32,
        native: &crate::ir::IrNativeResource,
    ) {
        let mut flags = RESOURCE_FLAG_NATIVE;
        if native.sendable {
            flags |= RESOURCE_FLAG_SENDABLE;
        }
        if native.close_may_fail {
            flags |= RESOURCE_FLAG_CLOSE_MAY_FAIL;
        }
        self.entries.push(ResourceEntry {
            type_id,
            close_function_id: strings.intern(&native.close_function),
            flags,
        });
    }

    pub(super) fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.entries.len() as u32);
        for entry in &self.entries {
            put_u32(&mut bytes, entry.type_id);
            put_u32(&mut bytes, entry.close_function_id);
            put_u32(&mut bytes, entry.flags);
        }
        bytes
    }
}

impl ImportTable {
    pub(super) fn from_metadata(strings: &mut StringPool, metadata: &BinaryReprMetadata) -> Self {
        let entries = metadata
            .dependencies
            .iter()
            .map(|dependency| ImportEntry {
                package_name: strings.intern(&dependency.name),
                package_ident: strings.intern(if dependency.ident.is_empty() {
                    &dependency.name
                } else {
                    &dependency.ident
                }),
                version: strings.intern(&dependency.version),
                pin: dependency.pin,
                flags: dependency.flags,
                used_symbols: Vec::new(),
            })
            .collect();

        Self { entries }
    }

    pub(super) fn record_used_imports(
        &mut self,
        strings: &mut StringPool,
        used_imported_functions: &HashSet<String>,
        external_function_abi_hashes: &HashMap<String, [u8; ABI_HASH_LEN]>,
    ) {
        let import_names = self
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.package_name,
                    strings.values[entry.package_name as usize].clone(),
                )
            })
            .collect::<Vec<_>>();

        for (package_name_id, package_name) in import_names {
            let prefix = format!("{package_name}.");
            let mut symbols = used_imported_functions
                .iter()
                .filter_map(|target| {
                    let symbol_name = target.strip_prefix(&prefix)?;
                    let sig_hash = *external_function_abi_hashes.get(target)?;
                    Some(AbiUsedSymbol {
                        name: strings.intern(symbol_name),
                        sig_hash,
                    })
                })
                .collect::<Vec<_>>();
            symbols.sort_by_key(|symbol| strings.values[symbol.name as usize].clone());
            if let Some(entry) = self
                .entries
                .iter_mut()
                .find(|entry| entry.package_name == package_name_id)
            {
                entry.used_symbols = symbols;
            }
        }
    }

    pub(super) fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.entries.len() as u32);
        for entry in &self.entries {
            put_u32(&mut bytes, entry.package_name);
            put_u32(&mut bytes, entry.package_ident);
            put_u32(&mut bytes, entry.version);
            bytes.push(if entry.pin { 1 } else { 0 });
            put_u32(&mut bytes, entry.flags);
            put_u32(&mut bytes, entry.used_symbols.len() as u32);
            for symbol in &entry.used_symbols {
                put_u32(&mut bytes, symbol.name);
                bytes.extend_from_slice(&symbol.sig_hash);
            }
        }
        bytes
    }
}

impl AbiIndex {
    pub(super) fn from_project(
        strings: &StringPool,
        types: &TypeTable,
        constants: &ConstPool,
        imports: &ImportTable,
        functions: &[Function],
    ) -> Result<Self, String> {
        let mut exports = Vec::new();
        for function in functions {
            if !is_exported_function(function) {
                continue;
            }
            let kind = if function.flags & FUNCTION_FLAG_SUB != 0 {
                BinaryReprExportKind::Sub
            } else {
                BinaryReprExportKind::Func
            };
            exports.push(AbiExport {
                name: function.name,
                kind,
                sig_hash: function_sig_hash(function, kind, &strings.values, types, constants)?,
            });
        }
        for (index, type_) in types.entries.iter().enumerate() {
            let Some(kind) = type_.abi_export_kind else {
                continue;
            };
            exports.push(AbiExport {
                name: type_.name,
                kind,
                sig_hash: type_sig_hash(
                    FIRST_TABLE_TYPE_ID + index as u32,
                    kind,
                    &strings.values,
                    types,
                    constants,
                )?,
            });
        }

        let dep_edges = imports
            .entries
            .iter()
            .map(|entry| AbiDepEdge {
                package_name: entry.package_name,
                package_ident: entry.package_ident,
                version_request: entry.version,
                pin: entry.pin,
                used_symbols: entry.used_symbols.clone(),
            })
            .collect();

        Ok(Self { exports, dep_edges })
    }

    pub(super) fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u16(&mut bytes, ABI_FORMAT_VERSION);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, self.exports.len() as u32);
        for export in &self.exports {
            put_u32(&mut bytes, export.name);
            put_u16(&mut bytes, encode_export_kind(export.kind));
            bytes.extend_from_slice(&export.sig_hash);
        }
        put_u32(&mut bytes, self.dep_edges.len() as u32);
        for edge in &self.dep_edges {
            put_u32(&mut bytes, edge.package_name);
            put_u32(&mut bytes, edge.package_ident);
            put_u32(&mut bytes, edge.version_request);
            bytes.push(if edge.pin { 1 } else { 0 });
            put_u32(&mut bytes, edge.used_symbols.len() as u32);
            for symbol in &edge.used_symbols {
                put_u32(&mut bytes, symbol.name);
                bytes.extend_from_slice(&symbol.sig_hash);
            }
        }
        bytes
    }
}

/// Encode the `NATIVE_LIBRARY_TABLE` (section id 10, plan-46-B §4.1).
///
/// Strings are interned into the shared string pool rather than written inline —
/// `os`, `arch`, and the common sonames repeat across a table's locators, so
/// interning genuinely dedups them. (The `doc` table writes its strings inline
/// instead; it is prose, where interning would buy nothing.)
///
/// Entries are already sorted by logical name and the locators keep manifest
/// order, so the bytes are deterministic — the repo holds a byte-identical
/// self-diff gate.
pub(super) fn encode_native_library_table(
    strings: &mut StringPool,
    table: &NativeLibraryTable,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    put_u32(&mut bytes, table.entries.len() as u32);
    for entry in &table.entries {
        let logical = strings.intern(&entry.logical);
        put_u32(&mut bytes, logical);
        put_u32(&mut bytes, entry.locators.len() as u32);
        for locator in &entry.locators {
            let os = strings.intern(&locator.os);
            // `""` is the any-arch wildcard on the wire; `arch` is never a
            // legitimate empty string (validation rejects a blank token).
            let arch = strings.intern(locator.arch.as_deref().unwrap_or(""));
            let source = strings.intern(&locator.source);
            put_u32(&mut bytes, os);
            put_u32(&mut bytes, arch);
            bytes.push(match locator.libc {
                None => WIRE_LIBC_UNSPECIFIED,
                Some(Libc::Glibc) => WIRE_LIBC_GLIBC,
                Some(Libc::Musl) => WIRE_LIBC_MUSL,
            });
            bytes.push(match locator.lib_type {
                LibType::System => WIRE_LIB_TYPE_SYSTEM,
                LibType::Vendor => WIRE_LIB_TYPE_VENDOR,
            });
            put_u32(&mut bytes, source);
            // The hash is present iff the locator is `vendor` — a system locator
            // names a file we never see, so there is nothing to hash.
            if let Some(hash) = &locator.hash {
                bytes.extend_from_slice(hash);
            }
        }
    }
    bytes
}

/// Decode the `NATIVE_LIBRARY_TABLE` (section id 10).
///
/// The `.mfp` is an **untrusted input** on the consumer side, and plan-46-C feeds
/// `source` straight into a C string and a filesystem path — so every invariant
/// the producer was supposed to uphold is re-checked here rather than assumed:
/// `libc`/`type` in range, `hash` present iff `vendor`, and `source` still a bare
/// filename.
pub(super) fn read_native_library_table(
    bytes: &[u8],
    strings: &[String],
) -> Result<NativeLibraryTable, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    // A locator occupies 14 wire bytes at minimum (4+4+1+1+4); an entry adds its
    // name + count. Bound the pre-allocation against the bytes actually present.
    let mut entries = Vec::with_capacity(bounded_capacity(count, bytes.len() - offset, 22));
    for _ in 0..count {
        let logical = table_string(strings, cursor_u32(bytes, &mut offset)?)?;
        let locator_count = cursor_u32(bytes, &mut offset)? as usize;
        let mut locators =
            Vec::with_capacity(bounded_capacity(locator_count, bytes.len() - offset, 14));
        for _ in 0..locator_count {
            locators.push(read_native_library_locator(bytes, &mut offset, strings)?);
        }
        entries.push(NativeLibraryEntry { logical, locators });
    }
    // bug-282 B3: every other section rejects trailing garbage; this one (and the
    // doc table) were added after audit-1 PKG-05 and missed the invariant, leaving
    // a smuggling nook inside an otherwise strict decoder.
    if offset != bytes.len() {
        return Err("invalid trailing bytes in native library table".to_string());
    }
    Ok(NativeLibraryTable { entries })
}

fn read_native_library_locator(
    bytes: &[u8],
    offset: &mut usize,
    strings: &[String],
) -> Result<NativeLibraryLocator, String> {
    let os = table_string(strings, cursor_u32(bytes, offset)?)?;
    let arch = table_string(strings, cursor_u32(bytes, offset)?)?;
    let libc = cursor_u8(bytes, offset)?;
    let lib_type = cursor_u8(bytes, offset)?;
    let source = table_string(strings, cursor_u32(bytes, offset)?)?;

    let libc = match libc {
        WIRE_LIBC_UNSPECIFIED => None,
        WIRE_LIBC_GLIBC => Some(Libc::Glibc),
        WIRE_LIBC_MUSL => Some(Libc::Musl),
        other => {
            return Err(format!(
                "native library table locator has out-of-range libc {other}"
            ))
        }
    };
    let lib_type = match lib_type {
        WIRE_LIB_TYPE_SYSTEM => LibType::System,
        WIRE_LIB_TYPE_VENDOR => LibType::Vendor,
        other => {
            return Err(format!(
                "native library table locator has out-of-range type {other}"
            ))
        }
    };

    // `source` feeds a `dlopen` C string and a `vendor/` path join downstream. A
    // hostile `.mfp` naming `../../etc/foo` or embedding a NUL must not reach
    // either, so re-validate the producer's rule here.
    if let Err(reason) = crate::manifest::libraries::source_is_bare(&source) {
        return Err(format!(
            "native library table locator source {source:?} is not a bare filename: {reason}"
        ));
    }

    // The hash is present iff the locator is `vendor`.
    let hash = match lib_type {
        LibType::Vendor => {
            let end = offset
                .checked_add(NATIVE_LIBRARY_HASH_LEN)
                .ok_or_else(|| "native library table locator hash overflows".to_string())?;
            let raw = bytes
                .get(*offset..end)
                .ok_or_else(|| "truncated native library table locator hash".to_string())?;
            let mut hash = [0u8; NATIVE_LIBRARY_HASH_LEN];
            hash.copy_from_slice(raw);
            *offset = end;
            Some(hash)
        }
        LibType::System => None,
    };

    Ok(NativeLibraryLocator {
        os,
        // `""` is the any-arch wildcard on the wire.
        arch: if arch.is_empty() { None } else { Some(arch) },
        libc,
        lib_type,
        source,
        hash,
    })
}

/// Resolve a string id against the pool, rejecting an out-of-range id rather than
/// panicking on a hostile `.mfp`.
fn table_string(strings: &[String], id: u32) -> Result<String, String> {
    strings
        .get(id as usize)
        .cloned()
        .ok_or_else(|| format!("native library table references unknown string id {id}"))
}

// === ABI signature hashing + export-kind encoding (bug-335 B1) =============
// The write-side ABI serializer and the `sigHash` builders that section 15
// (ABI_INDEX, encoded just above) is populated from, plus the scalar
// export-kind encoder and the exported-function predicate. Their decoders
// (`decode_export_kind`, `decode_callable_export_kind`) stay in reader.rs.

pub(super) fn encode_export_kind(kind: BinaryReprExportKind) -> u16 {
    match kind {
        BinaryReprExportKind::Func => 1,
        BinaryReprExportKind::Sub => 2,
        BinaryReprExportKind::Type => 3,
        BinaryReprExportKind::Union => 4,
        BinaryReprExportKind::Enum => 5,
    }
}

pub(super) fn is_exported_function(function: &Function) -> bool {
    function.kind == FUNCTION_BINARY_REPR && function.flags & FUNCTION_FLAG_PRIVATE == 0
}

pub(super) fn function_sig_hash(
    function: &Function,
    export_kind: BinaryReprExportKind,
    strings: &[String],
    types: &TypeTable,
    constants: &ConstPool,
) -> Result<[u8; ABI_HASH_LEN], String> {
    let mut serializer = AbiSerializer::new(strings, types, constants);
    serializer.bytes.extend_from_slice(b"MFBABI\0");
    serializer.put_u16(ABI_FORMAT_VERSION);
    serializer.put_str("function");
    serializer.put_u16(encode_export_kind(export_kind));
    serializer.put_u16(function.flags & (FUNCTION_FLAG_ISOLATED | FUNCTION_FLAG_SUB));
    serializer.put_u32(function.params.len() as u32);
    for param in &function.params {
        serializer.serialize_type(param.type_id)?;
        if param.default_const == u32::MAX {
            serializer.put_u8(0);
        } else {
            serializer.put_u8(1);
            serializer.serialize_const(param.default_const)?;
        }
    }
    serializer.serialize_type(function.return_type)?;
    Ok(hash_bytes(&serializer.bytes))
}

pub(super) fn type_sig_hash(
    type_id: u32,
    export_kind: BinaryReprExportKind,
    strings: &[String],
    types: &TypeTable,
    constants: &ConstPool,
) -> Result<[u8; ABI_HASH_LEN], String> {
    let mut serializer = AbiSerializer::new(strings, types, constants);
    serializer.bytes.extend_from_slice(b"MFBABI\0");
    serializer.put_u16(ABI_FORMAT_VERSION);
    serializer.put_str("type");
    serializer.put_u16(encode_export_kind(export_kind));
    serializer.serialize_type(type_id)?;
    Ok(hash_bytes(&serializer.bytes))
}

impl<'a> AbiSerializer<'a> {
    pub(super) fn new(
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

    pub(super) fn serialize_type(&mut self, id: u32) -> Result<(), String> {
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
            // bug-277: a resource carrying `STATE T` (kind 11) is a composite of
            // two type ids and must hash structurally like kinds 4/5/7. Under the
            // opaque fallback it hashed its interned name `State#<baseId>#<stateId>`,
            // which embeds table-position-dependent ids: an unrelated renumber
            // changed the hash with no semantic change, and a change to the STATE
            // record's own shape left the hash identical.
            _ => {
                self.put_str("opaque");
                self.put_str(string_at(self.strings, entry.name)?);
                Ok(())
            }
        }
    }

    pub(super) fn serialize_record_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
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

    pub(super) fn serialize_union_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
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

    pub(super) fn serialize_enum_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
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

    pub(super) fn serialize_function_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
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

    pub(super) fn serialize_const(&mut self, id: u32) -> Result<(), String> {
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

    pub(super) fn put_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(super) fn put_u16(&mut self, value: u16) {
        put_u16(&mut self.bytes, value);
    }

    pub(super) fn put_u32(&mut self, value: u32) {
        put_u32(&mut self.bytes, value);
    }

    pub(super) fn put_str(&mut self, value: &str) {
        put_bytes(&mut self.bytes, value.as_bytes());
    }
}
