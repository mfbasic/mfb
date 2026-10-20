use super::*;

mod functions;
mod resource_scan;
mod type_payload;
pub(in crate::binary_repr) use functions::*;
pub(in crate::binary_repr) use resource_scan::*;
pub(in crate::binary_repr) use type_payload::*;

pub(super) fn lower_project(
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
) -> Result<BinaryReprProject, String> {
    lower_project_with_external_functions(
        ir,
        metadata,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    )
}

pub(super) fn lower_package_project(
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
    package_paths: &[PathBuf],
) -> Result<BinaryReprProject, String> {
    let packages = package_paths
        .iter()
        .map(|path| read_package_binary_repr(path))
        .collect::<Result<Vec<_>, _>>()?;
    let (external_function_ids, external_function_returns, external_function_abi_hashes) =
        external_function_metadata(ir.functions.len() as u32, &packages)?;
    let external_types = external_type_metadata(&packages)?;
    lower_project_with_external_functions(
        ir,
        metadata,
        &external_function_ids,
        &external_function_returns,
        &external_function_abi_hashes,
        &external_types,
    )
}

/// bug-390: per dependency, the exported types this build may re-surface in its
/// own public API, keyed by original type name. For a type the dependency
/// *defines* the identity is (dep name, dep's ABI hash for it); for a type the
/// dependency itself *re-exports* (a `FOREIGN_TYPE_KIND` entry) the original
/// owner and owning ABI hash are read back from the entry, so the same underlying
/// `pA::A` keeps one identity however many intermediaries it passes through.
pub(super) fn external_type_metadata(
    packages: &[PackageBinaryRepr],
) -> Result<HashMap<String, ForeignTypeRef>, String> {
    let mut external_types = HashMap::new();
    for package in packages {
        let strings = &package.project.strings.values;
        let package_name = string_at(strings, package.project.manifest.package_name)?;
        let type_names = type_entry_names(&package.project.types, strings)?;
        let type_by_name = package
            .project
            .types
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let id = FIRST_TABLE_TYPE_ID + index as u32;
                type_name(&type_names, id).ok().map(|name| (name, entry))
            })
            .collect::<HashMap<_, _>>();
        for export in &package.project.abi.exports {
            if !matches!(
                export.kind,
                BinaryReprExportKind::Type
                    | BinaryReprExportKind::Union
                    | BinaryReprExportKind::Enum
            ) {
                continue;
            }
            let name = string_at(strings, export.name)?;
            let fref = match type_by_name.get(name) {
                // The dependency re-exports a type it does not own: carry the
                // original owner and owning ABI hash through unchanged.
                Some(entry) if entry.kind == FOREIGN_TYPE_KIND => {
                    let owner = string_at(strings, entry.owner_package)?.to_string();
                    let export_kind = decode_export_kind(checked_u16_at(&entry.payload, 0)?)?;
                    let hash_slice = entry
                        .payload
                        .get(2..2 + ABI_HASH_LEN)
                        .ok_or("truncated binary representation")?;
                    let mut abi_hash = [0u8; ABI_HASH_LEN];
                    abi_hash.copy_from_slice(hash_slice);
                    ForeignTypeRef {
                        package: owner,
                        export_kind,
                        abi_hash,
                    }
                }
                // The dependency owns the type: it is the identity source.
                _ => ForeignTypeRef {
                    package: package_name.to_string(),
                    export_kind: export.kind,
                    abi_hash: export.sig_hash,
                },
            };
            external_types.insert(name.to_string(), fref);
        }
    }
    Ok(external_types)
}

pub(super) fn external_function_metadata(
    base_function_id: u32,
    packages: &[PackageBinaryRepr],
) -> Result<
    (
        HashMap<String, u32>,
        HashMap<String, String>,
        HashMap<String, [u8; ABI_HASH_LEN]>,
    ),
    String,
> {
    let mut external_function_ids = HashMap::new();
    let mut external_function_returns = HashMap::new();
    let mut external_function_abi_hashes = HashMap::new();
    let mut next_function_id = base_function_id;
    for package in packages {
        let package_name = string_at(
            &package.project.strings.values,
            package.project.manifest.package_name,
        )?;
        let type_names = type_entry_names(&package.project.types, &package.project.strings.values)?;
        for export in &package.exports {
            let function = package
                .project
                .functions
                .get(export.function_id as usize)
                .ok_or_else(|| {
                    format!("export references missing function {}", export.function_id)
                })?;
            let export_name = string_at(&package.project.strings.values, export.name)?;
            // `export.function_id` is decoded from an attacker-influenced `.mfp`;
            // use checked_add (like the `next_function_id` bump below) so an
            // overflow errors instead of silently wrapping (bug-215).
            let global_function_id = next_function_id
                .checked_add(export.function_id)
                .ok_or_else(|| "merged binary representation has too many functions".to_string())?;
            external_function_ids
                .insert(format!("{package_name}.{export_name}"), global_function_id);
            external_function_returns.insert(
                format!("{package_name}.{export_name}"),
                type_name(&type_names, function.return_type)?.to_string(),
            );
            let abi_export =
                abi_export_for_decoded(&package.project.abi, export).ok_or_else(|| {
                    format!("ABI_INDEX is missing EXPORT_TABLE entry `{export_name}`")
                })?;
            external_function_abi_hashes
                .insert(format!("{package_name}.{export_name}"), abi_export.sig_hash);
        }
        next_function_id = next_function_id
            .checked_add(package.project.functions.len() as u32)
            .ok_or_else(|| "merged binary representation has too many functions".to_string())?;
    }
    Ok((
        external_function_ids,
        external_function_returns,
        external_function_abi_hashes,
    ))
}

pub(super) fn lower_project_with_external_functions(
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
    external_function_ids: &HashMap<String, u32>,
    external_function_returns: &HashMap<String, String>,
    external_function_abi_hashes: &HashMap<String, [u8; ABI_HASH_LEN]>,
    external_types: &HashMap<String, ForeignTypeRef>,
) -> Result<BinaryReprProject, String> {
    let mut strings = StringPool::new();
    let ident = if metadata.ident.is_empty() {
        &metadata.name
    } else {
        &metadata.ident
    };
    let manifest = BinaryReprManifest {
        package_name: strings.intern(&metadata.name),
        package_ident: strings.intern(ident),
        package_version: strings.intern(&metadata.version),
        ident_key: strings.intern(&metadata.ident_key),
        ident_fingerprint: strings.intern(&metadata.ident_fingerprint),
        signing_fingerprint: strings.intern(&metadata.signing_fingerprint),
        author: strings.intern(&metadata.author),
        url: strings.intern(&metadata.url),
        // Derived at encode time from the tables themselves (`encode_manifest`),
        // not from these fields; they exist so the decoder can cross-check a
        // manifest against its own tables (bug-282 B4).
        dependency_count: 0,
        export_count: 0,
    };
    let mut imports = ImportTable::from_metadata(&mut strings, metadata);

    let mut types = TypeTable::new();
    // bug-390: seed the imported-type identities before any `type_id` call so a
    // dependency type named in this package's own API encodes as a foreign
    // reference rather than an empty-record placeholder.
    types.foreign_types = external_types.clone();
    for ir_type in &ir.types {
        types.reserve_source_type(&mut strings, &metadata.name, ir_type);
    }
    types.populate_source_payloads(&mut strings, &ir.types)?;
    let mut resources = ResourceTable::new();
    if ir_uses_resource_type(ir) {
        let mut used = HashSet::new();
        collect_resource_type_names(ir, &mut used);
        if used.contains("fs.File") {
            resources.add_standard_file(&mut types, &mut strings);
        }
        // plan-110-E: the stream handles live in `tcp` now. Before this the
        // branches still named `net.Socket`/`net.Listener`, types that no longer
        // exist -- so a package exporting a stream resource silently got NO
        // resource-table entry.
        if used.contains("tcp.Socket") {
            resources.add_standard_socket(&mut types, &mut strings);
        }
        if used.contains("tcp.Listener") {
            resources.add_standard_listener(&mut types, &mut strings);
        }
        // Every OTHER built-in resource (bug-464 fallout). The three branches
        // above were the WHOLE table: a package exporting a `udp::Socket`, a
        // `tls::Socket`/`Listener`, a `process::Process`, an audio handle or a
        // `canvas::Image` got no entry and failed to build with an opaque
        // `truncated binary representation`. Reproduced on clean main
        // (fc5c8a6db) with `EXPORT FUNC useSock(s AS udp::Socket)`, so this
        // predates bug-464 -- the TLS work only made it unavoidable.
        //
        // `used` holds the spellings as they appear in signatures, which for a
        // plan-54 stateful handle is `fs.File STATE Cursor`, not `fs.File`. The
        // table is keyed by the BASE resource type, so each name is normalised
        // with `without_state()` before it is considered -- otherwise a stateful
        // handle adds a second, bogus entry under its full spelling, which is
        // exactly what drifted 45 committed `.mfp` files on the first attempt
        // here. (The three branches above compare bare names for the same
        // reason.)
        //
        // A BTreeSet both de-duplicates -- two params of `fs.File STATE A` and
        // `fs.File STATE B` normalise to one entry -- and fixes the order.
        // `used` is a HashSet, and an iteration-order-dependent table would make
        // `.mfp` bytes vary from run to run.
        let others: std::collections::BTreeSet<String> = used
            .iter()
            .map(|name| {
                crate::types::ParameterType::declared(name)
                    .without_state()
                    .name()
                    .to_string()
            })
            .filter(|base| {
                !matches!(base.as_str(), "fs.File" | "tcp.Socket" | "tcp.Listener")
                    && crate::codegen::resource::is_builtin_resource_type(
                        // `named`, not `declared`: this is already a normalised
                        // base identity, so it is a constructor call and not a
                        // second trip through the grammar (plan-111 class 1b).
                        &crate::types::ParameterType::named(base),
                    )
            })
            .collect();
        for name in &others {
            resources.add_standard_other(
                &mut types,
                &mut strings,
                &crate::types::ParameterType::named(name),
            );
        }
    }
    // Native LINK resources (plan-link-update.md §10): each becomes an opaque
    // type (exported when the declaration is `EXPORT`) plus a RESOURCE_TABLE
    // entry whose close op is referenced by name.
    for native in &ir.native_resources {
        // An opaque native resource has no fields; encode it as a zero-field
        // record so the type table round-trips. Its resource-ness (which blocks
        // construction and field access) comes from the RESOURCE_TABLE.
        let mut payload = Vec::new();
        put_u32(&mut payload, 0);
        let type_id = types.add_entry(&mut strings, &metadata.name, &native.name, 1, payload);
        if native.visibility == "export" {
            let index = (type_id - FIRST_TABLE_TYPE_ID) as usize;
            types.entries[index].abi_export_kind = Some(BinaryReprExportKind::Type);
        }
        resources.add_native(&mut strings, type_id, native);
    }
    let globals = ir
        .bindings
        .iter()
        .map(|binding| {
            let mut flags = 0;
            if binding.mutable {
                flags |= 1;
            }
            flags |= match binding.visibility.as_str() {
                "private" => 0 << 1,
                "public" => 1 << 1,
                "export" => 2 << 1,
                _ => 0,
            };
            GlobalEntry {
                name: strings.intern(&binding.name),
                type_id: types.type_id(&mut strings, &binding.type_),
                flags,
            }
        })
        .collect::<Vec<_>>();

    let mut constants = ConstPool::new();
    let mut function_ids = HashMap::new();
    for (index, function) in ir.functions.iter().enumerate() {
        function_ids.insert(function.name.clone(), index as u32);
        // The return type is interned even though the map that once held it was
        // never read (bug-100): the interning still fixes the string/type table
        // order, so the emitted bytes stay byte-identical.
        let _ = types.type_id(&mut strings, &function.returns);
    }
    for (name, id) in external_function_ids {
        function_ids.insert(name.clone(), *id);
    }
    for return_type_name in external_function_returns.values() {
        // Interning kept for table-order stability; the result is unused (bug-100).
        let _ = types.type_id(
            &mut strings,
            &crate::types::ParameterType::declared(return_type_name),
        );
    }

    let mut functions = Vec::new();
    let mut used_imported_functions = HashSet::new();
    for function in &ir.functions {
        functions.push(lower_function(
            function,
            &mut strings,
            &mut types,
            &mut constants,
            &function_ids,
            external_function_abi_hashes,
            &mut used_imported_functions,
        )?);
    }
    imports.record_used_imports(
        &mut strings,
        &used_imported_functions,
        external_function_abi_hashes,
    );
    // bug-390: surface (re-export) any imported type this package names in its own
    // exported API before the ABI index is built, so the index lists it under the
    // owning package's original identity.
    types.mark_reexported_foreign_types(&functions)?;
    let abi = AbiIndex::from_project(&strings, &types, &constants, &imports, &functions)?;

    let (entry_function, entry_flags) = if let Some(entry) = &ir.entry {
        let function_id = *function_ids.get(&entry.name).ok_or_else(|| {
            format!(
                "entry function `{}` was not lowered to binary representation",
                entry.name
            )
        })?;
        let mut flags = 1;
        if entry.accepts_args {
            flags |= 1 << 1;
        }
        if entry.returns == crate::types::ParameterType::Integer {
            flags |= 1 << 2;
        }
        (function_id, flags)
    } else {
        (u32::MAX, 0)
    };

    Ok(BinaryReprProject {
        strings,
        types,
        constants,
        resources,
        globals,
        manifest,
        imports,
        abi,
        entry_function,
        entry_flags,
        functions,
        binary_repr: crate::ir::encode_binary_repr(ir),
        docs: docs_from_ir(&ir.docs),
        // Assembled by the build path from the manifest's `libraries` section and
        // the IR's `LINK` names (plan-46-B §4.3); empty for a non-binding package.
        native_libraries: metadata.native_libraries.clone(),
        // plan-61-D: carried from the manifest into section 18. Empty here means
        // the section is not emitted at all.
        description: metadata.description.clone(),
    })
}

impl BinaryReprProject {
    pub(super) fn encode(&self) -> Vec<u8> {
        // Function bodies live in the structured Binary Representation payload, not a flat
        // code stream. The function table still records signatures/metadata, so
        // every per-function code region is zero-length.
        let code_offsets: Vec<(u64, u64)> = self.functions.iter().map(|_| (0, 0)).collect();

        // The native library table (plan-46-B §4.1) is encoded first because
        // interning its strings grows the pool, and the pool section below must be
        // encoded from the final pool. It is emitted only for a binding package
        // that declares a `LINK` block, so every non-binding package's `.mfp` —
        // and its string pool — stays byte-identical to a pre-plan-46 build.
        let mut strings = self.strings.clone();
        let native_libraries = if self.native_libraries.is_empty() {
            None
        } else {
            Some(encode_native_library_table(
                &mut strings,
                &self.native_libraries,
            ))
        };

        let mut sections = vec![
            Section::new(SECTION_MANIFEST, self.encode_manifest()),
            Section::new(SECTION_STRING_POOL, strings.encode()),
            Section::new(SECTION_TYPE_TABLE, self.types.encode()),
            Section::new(SECTION_CONST_POOL, self.constants.encode()),
            Section::new(SECTION_IMPORT_TABLE, self.imports.encode()),
            Section::new(SECTION_EXPORT_TABLE, self.encode_exports()),
            Section::new(SECTION_GLOBAL_TABLE, self.encode_globals()),
            Section::new(SECTION_FUNCTION_TABLE, self.encode_functions(&code_offsets)),
            Section::new(SECTION_BINARY_REPR, self.binary_repr.clone()),
            Section::new(SECTION_ABI_INDEX, self.abi.encode()),
        ];
        if !self.resources.entries.is_empty() {
            sections.push(Section::new(
                SECTION_RESOURCE_TABLE,
                self.resources.encode(),
            ));
        }
        // The doc section is emitted only when the package has documentation
        // (plan-09-doc.md §5); it does not affect execution or the ABI.
        if !self.docs.is_empty() {
            sections.push(Section::new(
                SECTION_DOC_TABLE,
                encode_doc_table(&self.docs),
            ));
        }
        if let Some(table) = native_libraries {
            sections.push(Section::new(SECTION_NATIVE_LIBRARY_TABLE, table));
        }
        // Section 18 is emitted **only** when there is something to put in it.
        // Emitting an empty section instead would change the bytes of every
        // package that has no description — and the whole point of plan-61-D is
        // that packages without one are byte-identical to before.
        if let Some(section) = encode_package_meta(&self.description) {
            sections.push(Section::new(SECTION_PACKAGE_META, section));
        }

        encode_sections(&sections)
    }

    pub(super) fn encode_manifest(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.manifest.package_name);
        put_u32(&mut bytes, self.manifest.package_ident);
        put_u32(&mut bytes, self.manifest.package_version);
        put_u32(&mut bytes, self.manifest.ident_key);
        put_u32(&mut bytes, self.manifest.ident_fingerprint);
        put_u32(&mut bytes, self.manifest.signing_fingerprint);
        put_u32(&mut bytes, self.manifest.author);
        put_u32(&mut bytes, self.manifest.url);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, self.imports.entries.len() as u32);
        put_u32(&mut bytes, 0);
        put_u32(&mut bytes, self.export_count());
        put_u32(&mut bytes, self.entry_function);
        put_u32(&mut bytes, self.entry_flags);
        bytes
    }

    pub(super) fn encode_exports(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.export_count());
        for (index, function) in self.functions.iter().enumerate() {
            if !is_exported_function(function) {
                continue;
            }
            put_u32(&mut bytes, function.name);
            put_u16(&mut bytes, encode_export_kind(BinaryReprExportKind::Func));
            put_u16(&mut bytes, 0);
            put_u32(&mut bytes, index as u32);
        }
        bytes
    }

    pub(super) fn encode_globals(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.globals.len() as u32);
        for global in &self.globals {
            put_u32(&mut bytes, global.name);
            put_u32(&mut bytes, global.type_id);
            put_u32(&mut bytes, global.flags);
        }
        bytes
    }

    pub(super) fn export_count(&self) -> u32 {
        self.functions
            .iter()
            .filter(|function| is_exported_function(function))
            .count() as u32
    }

    pub(super) fn encode_functions(&self, code_offsets: &[(u64, u64)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.functions.len() as u32);
        for (index, function) in self.functions.iter().enumerate() {
            let (code_offset, code_length) = code_offsets[index];
            put_u32(&mut bytes, function.name);
            put_u16(&mut bytes, function.kind);
            put_u16(&mut bytes, function.flags);
            put_u32(&mut bytes, function.params.len() as u32);
            put_u32(&mut bytes, function.return_type);
            put_u32(&mut bytes, function.registers.len() as u32);
            put_u64(&mut bytes, code_offset);
            put_u64(&mut bytes, code_length);
            put_u32(&mut bytes, u32::MAX);
            put_u32(&mut bytes, function.cleanups.len() as u32);
            let cleanup_offset =
                bytes.len() + 8 + function.params.len() * 16 + function.registers.len() * 8;
            put_u64(
                &mut bytes,
                if function.cleanups.is_empty() {
                    0
                } else {
                    cleanup_offset as u64
                },
            );

            for param in &function.params {
                put_u32(&mut bytes, param.name);
                put_u32(&mut bytes, param.type_id);
                put_u32(&mut bytes, param.flags);
                put_u32(&mut bytes, param.default_const);
            }

            for register in &function.registers {
                put_u32(&mut bytes, register.type_id);
                put_u32(&mut bytes, register.flags);
            }

            for cleanup in &function.cleanups {
                put_u32(&mut bytes, cleanup.id);
                put_u32(&mut bytes, cleanup.start_pc);
                put_u32(&mut bytes, cleanup.end_pc);
                put_u32(&mut bytes, cleanup.resource_register);
                put_u32(&mut bytes, cleanup.close_function_id);
                put_u32(&mut bytes, cleanup.flags);
            }
        }
        bytes
    }
}

// === Doc-section lowering (bug-335 B1, plan-126-D) =========================
// The IR→PackageDocs lowering. The section-17 codec itself -- `encode_doc_table`,
// formerly here, and `read_doc_table`/`doc_kind_name`, formerly in reader.rs --
// moved to `mfb_wire::docs` so the registry can decode published documentation
// without this crate. `docs_from_ir` stays because `crate::ir::ProjectDocs` is
// compiler IR; only the `PackageDocs` half of the boundary moved.

pub(super) fn docs_from_ir(docs: &crate::ir::ProjectDocs) -> PackageDocs {
    use crate::ir::IrDocKind;
    let package = docs.package.as_ref().map(|package| PackageDocEntry {
        name: package.name.clone(),
        desc: package.desc.clone(),
        deprecated: package.deprecated.clone(),
    });
    let decls = docs
        .decls
        .iter()
        .map(|decl| DeclDocEntry {
            kind: match decl.kind {
                IrDocKind::Func => "func",
                IrDocKind::Type => "type",
                IrDocKind::Union => "union",
                IrDocKind::Enum => "enum",
                IrDocKind::Resource => "resource",
            }
            .to_string(),
            name: decl.name.clone(),
            signature: decl.signature.clone(),
            group: decl.group.clone(),
            desc: decl.desc.clone(),
            args: decl.args.clone(),
            props: decl.props.clone(),
            ret: decl.ret.clone(),
            errors: decl.errors.clone(),
            example: decl.example.clone(),
            internal: decl.internal,
            deprecated: decl.deprecated.clone(),
        })
        .collect();
    PackageDocs { package, decls }
}
