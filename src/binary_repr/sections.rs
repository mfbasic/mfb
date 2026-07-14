use super::*;

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

    pub(super) fn type_id(&mut self, strings: &mut StringPool, name: &str) -> u32 {
        match name {
            "Nothing" => TYPE_NOTHING,
            "Boolean" => TYPE_BOOLEAN,
            "Integer" => TYPE_INTEGER,
            "Float" => TYPE_FLOAT,
            "Fixed" => TYPE_FIXED,
            "String" => TYPE_STRING,
            "Scalar" => TYPE_SCALAR,
            "File" => TYPE_FILE_HANDLE,
            "Socket" => TYPE_SOCKET_HANDLE,
            "Listener" => TYPE_LISTENER_HANDLE,
            name if name.starts_with("List OF ") => {
                // `strip_prefix` (not `trim_start_matches`, which is greedy and
                // would collapse `List OF List OF X` to `List OF X`).
                let element = self.type_id(strings, name.strip_prefix("List OF ").unwrap_or(name));
                self.list_type(strings, element)
            }
            name if name.starts_with("Result OF ") => {
                let success =
                    self.type_id(strings, name.strip_prefix("Result OF ").unwrap_or(name));
                self.result_type(strings, success)
            }
            name if name.starts_with("Thread OF ") => {
                if let Some((_, message, resource, output)) =
                    builtins::thread::thread_parts_full(name)
                {
                    let message = self.type_id(strings, message);
                    let resource = resource.map(|resource| self.type_id(strings, resource));
                    let output = self.type_id(strings, output);
                    self.thread_type(strings, message, resource, output)
                } else {
                    self.add_entry(strings, "", name, 7, Vec::new())
                }
            }
            name if name.starts_with("ThreadWorker OF ") => {
                if let Some((_, message, resource, output)) =
                    builtins::thread::thread_parts_full(name)
                {
                    let message = self.type_id(strings, message);
                    let resource = resource.map(|resource| self.type_id(strings, resource));
                    let output = self.type_id(strings, output);
                    self.thread_worker_type(strings, message, resource, output)
                } else {
                    self.add_entry(strings, "", name, 10, Vec::new())
                }
            }
            name if name.starts_with("FUNC(") => self.function_type(strings, name),
            name if name.starts_with("ISOLATED FUNC(") => self.function_type(strings, name),
            name if name.starts_with("Map OF ") => {
                let rest = name.strip_prefix("Map OF ").unwrap_or(name);
                if let Some((key, value)) = rest.split_once(" TO ") {
                    let key = self.type_id(strings, key);
                    let value = self.type_id(strings, value);
                    self.map_type(strings, key, value)
                } else {
                    self.add_entry(strings, "", name, 5, Vec::new())
                }
            }
            name if name.starts_with("MapEntry OF ") => {
                let rest = name.strip_prefix("MapEntry OF ").unwrap_or(name);
                if let Some((key, value)) = rest.split_once(" TO ") {
                    let key = self.type_id(strings, key);
                    let value = self.type_id(strings, value);
                    self.map_entry_type(strings, key, value)
                } else {
                    self.add_entry(strings, "", name, 9, Vec::new())
                }
            }
            "Byte" => TYPE_BYTE,
            "Money" => TYPE_MONEY,
            "Error" => {
                strings.intern("code");
                strings.intern("message");
                TYPE_ERROR
            }
            "TermColor" => {
                strings.intern("r");
                strings.intern("g");
                strings.intern("b");
                TYPE_TERM_COLOR
            }
            "TermSize" => {
                strings.intern("columns");
                strings.intern("rows");
                TYPE_TERM_SIZE
            }
            _ => {
                if let Some(id) = self.ids.get(name) {
                    *id
                } else {
                    self.add_entry(strings, "", name, 1, Vec::new())
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

    pub(super) fn list_type(&mut self, strings: &mut StringPool, element_type: u32) -> u32 {
        let name = format!("List#{element_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, element_type);
        self.add_entry(strings, "", &name, 4, payload)
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
            let return_type = self.type_id(strings, &signature.returns);
            put_u32(&mut payload, return_type);
            for param in signature.params {
                let param_type = self.type_id(strings, &param);
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
}

impl ConstPool {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(super) fn add(&mut self, strings: &mut StringPool, value: &IrValue) -> Result<u32, String> {
        let entry = match value {
            IrValue::Const { type_, value } => match type_.as_str() {
                "Nothing" => ConstEntry {
                    kind: 1,
                    payload: Vec::new(),
                },
                "String" => {
                    let mut payload = Vec::new();
                    put_u32(&mut payload, strings.intern(value));
                    ConstEntry { kind: 6, payload }
                }
                "Integer" => ConstEntry {
                    kind: 3,
                    payload: value
                        .parse::<i64>()
                        .map_err(|_| format!("invalid Integer constant `{value}`"))?
                        .to_le_bytes()
                        .to_vec(),
                },
                "Float" => ConstEntry {
                    kind: 4,
                    payload: value
                        .parse::<f64>()
                        .map_err(|_| format!("invalid Float constant `{value}`"))?
                        .to_bits()
                        .to_le_bytes()
                        .to_vec(),
                },
                "Fixed" => ConstEntry {
                    kind: 5,
                    payload: fixed_raw_from_decimal(value)?.to_le_bytes().to_vec(),
                },
                // Money's `kind` is its wire type id (`TYPE_MONEY` = 9); the raw
                // is the exact base-10 scaled i64 (plan-29-B §4.3).
                "Money" => ConstEntry {
                    kind: TYPE_MONEY as u16,
                    payload: crate::numeric::money_raw_from_decimal(value)?
                        .to_le_bytes()
                        .to_vec(),
                },
                "Boolean" => ConstEntry {
                    kind: 2,
                    payload: vec![if value == "true" { 1 } else { 0 }],
                },
                "Byte" => ConstEntry {
                    kind: 7,
                    payload: vec![value
                        .parse::<u8>()
                        .map_err(|_| format!("invalid Byte constant `{value}`"))?],
                },
                // Scalar's `kind` is its wire type id (`TYPE_SCALAR` = 10); the
                // payload is the 4-byte LE Unicode codepoint (plan-41-B §3).
                "Scalar" => ConstEntry {
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
        let type_id = types.type_id(strings, builtins::fs::FILE_TYPE);
        self.entries.push(ResourceEntry {
            type_id,
            close_function_id: BUILTIN_FS_CLOSE_FUNCTION_ID,
            flags: standard_resource_flags(builtins::fs::FILE_TYPE),
        });
    }

    pub(super) fn add_standard_socket(&mut self, types: &mut TypeTable, strings: &mut StringPool) {
        let type_id = types.type_id(strings, builtins::net::SOCKET_TYPE);
        self.entries.push(ResourceEntry {
            type_id,
            close_function_id: BUILTIN_NET_CLOSE_FUNCTION_ID,
            flags: standard_resource_flags(builtins::net::SOCKET_TYPE),
        });
    }

    pub(super) fn add_standard_listener(
        &mut self,
        types: &mut TypeTable,
        strings: &mut StringPool,
    ) {
        let type_id = types.type_id(strings, builtins::net::LISTENER_TYPE);
        self.entries.push(ResourceEntry {
            type_id,
            close_function_id: BUILTIN_NET_CLOSE_FUNCTION_ID,
            flags: standard_resource_flags(builtins::net::LISTENER_TYPE),
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
