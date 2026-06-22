use std::collections::HashSet;

use crate::json_string;
use crate::target::shared::plan::{CallKind, NativePlan};

const VM_BASE: u64 = 0x1_0000_0000;
const TEXT_FILE_OFFSET: usize = 0x4000;
const LINKEDIT_FILE_OFFSET: usize = 0x8000;

pub(crate) struct NativeObjectPlan {
    target: String,
    container: String,
    status: String,
    entry: String,
    image_base: u64,
    dylibs: Vec<String>,
    load_commands: Vec<LoadCommandPlan>,
    segments: Vec<SegmentPlan>,
    sections: Vec<SectionPlan>,
    code_units: Vec<CodeUnitPlan>,
    data_units: Vec<DataUnitPlan>,
    defined_symbols: Vec<String>,
    imported_symbols: Vec<ObjectImport>,
    external_symbols: Vec<String>,
    symbol_table: Vec<SymbolPlan>,
    string_table: StringTablePlan,
    relocations: Vec<ObjectRelocation>,
}

struct LoadCommandPlan {
    kind: String,
    name: Option<String>,
}

struct SegmentPlan {
    name: String,
    vm_address: u64,
    vm_size: usize,
    file_offset: usize,
    file_size: usize,
    max_protection: String,
    initial_protection: String,
}

struct SectionPlan {
    segment: String,
    section: Option<String>,
    kind: String,
    vm_address: u64,
    file_offset: usize,
    size: usize,
    align: usize,
}

struct CodeUnitPlan {
    symbol: String,
    section: String,
    offset: usize,
    planned_size: usize,
    operations: Vec<String>,
    calls: Vec<String>,
    data_refs: Vec<String>,
}

struct DataUnitPlan {
    symbol: String,
    section: String,
    offset: usize,
    size: usize,
    value: String,
}

struct ObjectImport {
    library: String,
    symbol: String,
}

struct SymbolPlan {
    name: String,
    kind: String,
    section: Option<String>,
    value: Option<u64>,
    string_table_offset: usize,
}

struct StringTablePlan {
    size: usize,
    entries: Vec<StringTableEntry>,
}

struct StringTableEntry {
    value: String,
    offset: usize,
}

struct ObjectRelocation {
    from: String,
    to: String,
    kind: String,
    section: String,
}

pub(crate) fn lower_plan(plan: &NativePlan) -> Result<NativeObjectPlan, String> {
    let entry = "_main".to_string();
    let dylibs = dylibs(plan)?;
    let imported_symbols = imported_symbols(plan);
    let data_units = data_units(plan);
    let code_units = code_units(plan, &entry, &data_units);
    let text_size = code_units
        .iter()
        .map(|unit| unit.planned_size)
        .sum::<usize>();
    let cstring_size = data_units.iter().map(|unit| unit.size).sum::<usize>();
    let text_vm_address = VM_BASE + TEXT_FILE_OFFSET as u64;
    let cstring_file_offset = TEXT_FILE_OFFSET + align(text_size, 16);
    let cstring_vm_address = VM_BASE + cstring_file_offset as u64;
    let linkedit_size = 0x1000;

    let sections = vec![
        SectionPlan {
            segment: "__TEXT".to_string(),
            section: Some("__text".to_string()),
            kind: "code".to_string(),
            vm_address: text_vm_address,
            file_offset: TEXT_FILE_OFFSET,
            size: text_size,
            align: 4,
        },
        SectionPlan {
            segment: "__TEXT".to_string(),
            section: Some("__cstring".to_string()),
            kind: "cstring".to_string(),
            vm_address: cstring_vm_address,
            file_offset: cstring_file_offset,
            size: cstring_size,
            align: 1,
        },
        SectionPlan {
            segment: "__LINKEDIT".to_string(),
            section: None,
            kind: "linkedit".to_string(),
            vm_address: VM_BASE + LINKEDIT_FILE_OFFSET as u64,
            file_offset: LINKEDIT_FILE_OFFSET,
            size: linkedit_size,
            align: 1,
        },
    ];
    let defined_symbols = defined_symbols(&entry, plan, &data_units);
    let symbol_table = symbol_table(
        &defined_symbols,
        &imported_symbols,
        &code_units,
        &data_units,
    );
    let string_table = string_table(&symbol_table);
    let relocations = relocations(plan, &entry, &data_units);
    let external_symbols = external_symbols(&relocations);

    let object = NativeObjectPlan {
        target: plan.target.clone(),
        container: "mach-o".to_string(),
        status: "planOnly".to_string(),
        entry,
        image_base: VM_BASE,
        dylibs,
        load_commands: load_commands(plan),
        segments: vec![
            SegmentPlan {
                name: "__TEXT".to_string(),
                vm_address: VM_BASE,
                vm_size: LINKEDIT_FILE_OFFSET,
                file_offset: 0,
                file_size: LINKEDIT_FILE_OFFSET,
                max_protection: "read-execute".to_string(),
                initial_protection: "read-execute".to_string(),
            },
            SegmentPlan {
                name: "__LINKEDIT".to_string(),
                vm_address: VM_BASE + LINKEDIT_FILE_OFFSET as u64,
                vm_size: linkedit_size,
                file_offset: LINKEDIT_FILE_OFFSET,
                file_size: linkedit_size,
                max_protection: "read".to_string(),
                initial_protection: "read".to_string(),
            },
        ],
        sections,
        code_units,
        data_units,
        defined_symbols,
        imported_symbols,
        external_symbols,
        symbol_table,
        string_table,
        relocations,
    };
    object.validate()?;
    Ok(object)
}

impl NativeObjectPlan {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.target != "macos-aarch64" {
            return Err(format!(
                "native object plan target '{}' does not match macos-aarch64",
                self.target
            ));
        }
        if self.container != "mach-o" || self.status != "planOnly" {
            return Err("native object plan must be a plan-only Mach-O plan".to_string());
        }
        if !self.defined_symbols.contains(&self.entry) {
            return Err(format!(
                "native object plan entry '{}' is not a defined symbol",
                self.entry
            ));
        }
        reject_duplicates("defined symbol", &self.defined_symbols)?;
        reject_duplicates("dylib", &self.dylibs)?;
        validate_sections(&self.sections)?;
        for unit in &self.code_units {
            if unit.operations.is_empty() {
                return Err(format!(
                    "native object plan code unit '{}' has no operations",
                    unit.symbol
                ));
            }
        }
        validate_relocations(self)?;
        Ok(())
    }

    pub(crate) fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-native-object-plan\",\n",
                "  \"version\": 2,\n",
                "  \"target\": {},\n",
                "  \"container\": {},\n",
                "  \"status\": {},\n",
                "  \"entry\": {},\n",
                "  \"imageBase\": {},\n",
                "  \"dylibs\": [{}],\n",
                "  \"loadCommands\": [{}\n  ],\n",
                "  \"segments\": [{}\n  ],\n",
                "  \"sections\": [{}\n  ],\n",
                "  \"codeUnits\": [{}\n  ],\n",
                "  \"dataUnits\": [{}\n  ],\n",
                "  \"definedSymbols\": [{}],\n",
                "  \"importedSymbols\": [{}\n  ],\n",
                "  \"externalSymbols\": [{}],\n",
                "  \"symbolTable\": [{}\n  ],\n",
                "  \"stringTable\": {},\n",
                "  \"relocations\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.target),
            json_string(&self.container),
            json_string(&self.status),
            json_string(&self.entry),
            self.image_base,
            json_string_list(&self.dylibs),
            join_json(&self.load_commands, 2),
            join_json(&self.segments, 2),
            join_json(&self.sections, 2),
            join_json(&self.code_units, 2),
            join_json(&self.data_units, 2),
            json_string_list(&self.defined_symbols),
            join_json(&self.imported_symbols, 2),
            json_string_list(&self.external_symbols),
            join_json(&self.symbol_table, 2),
            self.string_table.to_json(2),
            join_json(&self.relocations, 2)
        )
    }
}

fn load_commands(plan: &NativePlan) -> Vec<LoadCommandPlan> {
    let mut commands = vec![
        LoadCommandPlan {
            kind: "LC_SEGMENT_64".to_string(),
            name: Some("__TEXT".to_string()),
        },
        LoadCommandPlan {
            kind: "LC_SEGMENT_64".to_string(),
            name: Some("__LINKEDIT".to_string()),
        },
        LoadCommandPlan {
            kind: "LC_DYLD_INFO_ONLY".to_string(),
            name: None,
        },
        LoadCommandPlan {
            kind: "LC_SYMTAB".to_string(),
            name: None,
        },
        LoadCommandPlan {
            kind: "LC_DYSYMTAB".to_string(),
            name: None,
        },
        LoadCommandPlan {
            kind: "LC_LOAD_DYLINKER".to_string(),
            name: Some("/usr/lib/dyld".to_string()),
        },
        LoadCommandPlan {
            kind: "LC_BUILD_VERSION".to_string(),
            name: Some("macos".to_string()),
        },
        LoadCommandPlan {
            kind: "LC_MAIN".to_string(),
            name: None,
        },
        LoadCommandPlan {
            kind: "LC_CODE_SIGNATURE".to_string(),
            name: None,
        },
    ];
    for dylib in dylibs(plan).unwrap_or_default() {
        commands.push(LoadCommandPlan {
            kind: "LC_LOAD_DYLIB".to_string(),
            name: Some(dylib),
        });
    }
    commands
}

fn dylibs(plan: &NativePlan) -> Result<Vec<String>, String> {
    let mut dylibs = Vec::new();
    for import in &plan.platform_imports {
        push_unique(&mut dylibs, dylib_for_library(&import.library)?);
    }
    Ok(dylibs)
}

fn imported_symbols(plan: &NativePlan) -> Vec<ObjectImport> {
    let mut symbols = Vec::new();
    for import in &plan.platform_imports {
        push_import(
            &mut symbols,
            ObjectImport {
                library: import.library.clone(),
                symbol: import.symbol.clone(),
            },
        );
    }
    symbols
}

fn data_units(plan: &NativePlan) -> Vec<DataUnitPlan> {
    let mut values = Vec::new();
    for function in &plan.functions {
        for call in &function.calls {
            for literal in &call.string_literals {
                push_unique(&mut values, literal.clone());
            }
        }
    }
    let mut offset = 0;
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let size = value.len() + 1;
            let unit = DataUnitPlan {
                symbol: format!("_mfb_cstr_{index}"),
                section: "__TEXT,__cstring".to_string(),
                offset,
                size,
                value,
            };
            offset += size;
            unit
        })
        .collect()
}

fn code_units(plan: &NativePlan, entry: &str, data_units: &[DataUnitPlan]) -> Vec<CodeUnitPlan> {
    let mut units = Vec::new();
    let mut offset = 0;
    let entry_call = plan.entry_symbol.clone().unwrap_or_default();
    units.push(CodeUnitPlan {
        symbol: entry.to_string(),
        section: "__TEXT,__text".to_string(),
        offset,
        planned_size: 16,
        operations: vec!["call entry function".to_string(), "return 0".to_string()],
        calls: vec![entry_call],
        data_refs: Vec::new(),
    });
    offset += 16;

    for function in &plan.functions {
        let calls = function
            .calls
            .iter()
            .map(|call| call.symbol.clone())
            .collect::<Vec<_>>();
        let planned_size = planned_code_size(function.operations.len());
        units.push(CodeUnitPlan {
            symbol: function.symbol.clone(),
            section: "__TEXT,__text".to_string(),
            offset,
            planned_size,
            operations: function.operations.clone(),
            calls,
            data_refs: Vec::new(),
        });
        offset += planned_size;
    }

    for runtime_symbol in &plan.runtime_symbols {
        let mut calls = Vec::new();
        for import in &plan.platform_imports {
            if &import.required_by == runtime_symbol {
                push_unique(&mut calls, import.symbol.clone());
            }
        }
        units.push(CodeUnitPlan {
            symbol: runtime_symbol.clone(),
            section: "__TEXT,__text".to_string(),
            offset,
            planned_size: 32,
            operations: vec!["execute runtime helper".to_string()],
            calls,
            data_refs: data_units.iter().map(|unit| unit.symbol.clone()).collect(),
        });
        offset += 32;
    }
    // Native `LINK` initializer + marshaling thunks (plan-linker.md §12): defined
    // internal code, each carrying its dlopen/dlsym (or no) platform-import calls.
    for link_symbol in &plan.link_symbols {
        let mut calls = Vec::new();
        for import in &plan.platform_imports {
            if &import.required_by == link_symbol {
                push_unique(&mut calls, import.symbol.clone());
            }
        }
        units.push(CodeUnitPlan {
            symbol: link_symbol.clone(),
            section: "__TEXT,__text".to_string(),
            offset,
            planned_size: 32,
            operations: vec!["native link binding".to_string()],
            calls,
            data_refs: Vec::new(),
        });
        offset += 32;
    }
    units
}

fn planned_code_size(operation_count: usize) -> usize {
    align(operation_count.max(1) * 4, 4)
}

fn defined_symbols(entry: &str, plan: &NativePlan, data_units: &[DataUnitPlan]) -> Vec<String> {
    let mut defined = vec![entry.to_string()];
    for function in &plan.functions {
        push_unique(&mut defined, function.symbol.clone());
    }
    for symbol in &plan.runtime_symbols {
        push_unique(&mut defined, symbol.clone());
    }
    for symbol in &plan.link_symbols {
        push_unique(&mut defined, symbol.clone());
    }
    for unit in data_units {
        push_unique(&mut defined, unit.symbol.clone());
    }
    defined
}

fn symbol_table(
    defined_symbols: &[String],
    imported_symbols: &[ObjectImport],
    code_units: &[CodeUnitPlan],
    data_units: &[DataUnitPlan],
) -> Vec<SymbolPlan> {
    let mut table = Vec::new();
    for symbol in defined_symbols {
        let (section, value) = code_units
            .iter()
            .find(|unit| &unit.symbol == symbol)
            .map(|unit| (Some(unit.section.clone()), Some(unit.offset as u64)))
            .or_else(|| {
                data_units
                    .iter()
                    .find(|unit| &unit.symbol == symbol)
                    .map(|unit| (Some(unit.section.clone()), Some(unit.offset as u64)))
            })
            .unwrap_or((None, None));
        table.push(SymbolPlan {
            name: symbol.clone(),
            kind: "defined".to_string(),
            section,
            value,
            string_table_offset: 0,
        });
    }
    for import in imported_symbols {
        table.push(SymbolPlan {
            name: import.symbol.clone(),
            kind: "imported".to_string(),
            section: None,
            value: None,
            string_table_offset: 0,
        });
    }
    let mut offset = 1;
    for symbol in &mut table {
        symbol.string_table_offset = offset;
        offset += symbol.name.len() + 1;
    }
    table
}

fn string_table(symbol_table: &[SymbolPlan]) -> StringTablePlan {
    let mut entries = Vec::new();
    let mut offset = 1;
    for symbol in symbol_table {
        entries.push(StringTableEntry {
            value: symbol.name.clone(),
            offset,
        });
        offset += symbol.name.len() + 1;
    }
    StringTablePlan {
        size: offset,
        entries,
    }
}

fn relocations(
    plan: &NativePlan,
    entry: &str,
    data_units: &[DataUnitPlan],
) -> Vec<ObjectRelocation> {
    let mut relocations = Vec::new();
    if let Some(entry_symbol) = &plan.entry_symbol {
        push_relocation(
            &mut relocations,
            ObjectRelocation {
                from: entry.to_string(),
                to: entry_symbol.clone(),
                kind: "internalCall".to_string(),
                section: "__TEXT,__text".to_string(),
            },
        );
    }
    for function in &plan.functions {
        for call in &function.calls {
            let kind = match call.kind {
                CallKind::Local | CallKind::Runtime => "internalCall",
                CallKind::Import => "packageCall",
                CallKind::Indirect => "indirectCall",
            };
            push_relocation(
                &mut relocations,
                ObjectRelocation {
                    from: function.symbol.clone(),
                    to: call.symbol.clone(),
                    kind: kind.to_string(),
                    section: "__TEXT,__text".to_string(),
                },
            );
        }
    }
    for import in &plan.platform_imports {
        push_relocation(
            &mut relocations,
            ObjectRelocation {
                from: import.required_by.clone(),
                to: import.symbol.clone(),
                kind: "externalCall".to_string(),
                section: "__TEXT,__text".to_string(),
            },
        );
    }
    for runtime_symbol in &plan.runtime_symbols {
        for unit in data_units {
            push_relocation(
                &mut relocations,
                ObjectRelocation {
                    from: runtime_symbol.clone(),
                    to: unit.symbol.clone(),
                    kind: "dataReference".to_string(),
                    section: "__TEXT,__text".to_string(),
                },
            );
        }
    }
    relocations
}

fn external_symbols(relocations: &[ObjectRelocation]) -> Vec<String> {
    let mut symbols = Vec::new();
    for relocation in relocations {
        if relocation.kind == "packageCall" {
            push_unique(&mut symbols, relocation.to.clone());
        }
    }
    symbols
}

fn dylib_for_library(library: &str) -> Result<String, String> {
    // Mirror of the linker's library->path table (plan-linker.md §7.3); kept in
    // sync so the object plan validates the same multi-library import sets.
    match library {
        "libSystem" => Ok("/usr/lib/libSystem.B.dylib".to_string()),
        "Network" => Ok("/System/Library/Frameworks/Network.framework/Network".to_string()),
        "AppKit" => Ok("/System/Library/Frameworks/AppKit.framework/AppKit".to_string()),
        "Foundation" => {
            Ok("/System/Library/Frameworks/Foundation.framework/Foundation".to_string())
        }
        "libobjc" => Ok("/usr/lib/libobjc.A.dylib".to_string()),
        "libz" => Ok("/usr/lib/libz.1.dylib".to_string()),
        other => Err(format!(
            "macos native object plan does not know dylib for platform library '{other}'"
        )),
    }
}

fn validate_sections(sections: &[SectionPlan]) -> Result<(), String> {
    if !sections
        .iter()
        .any(|section| section.segment == "__TEXT" && section.section.as_deref() == Some("__text"))
    {
        return Err("native object plan requires __TEXT,__text".to_string());
    }
    for (index, section) in sections.iter().enumerate() {
        if section.size == 0 && section.kind != "linkedit" {
            continue;
        }
        let end = section.file_offset + section.size;
        for other in sections.iter().skip(index + 1) {
            let other_end = other.file_offset + other.size;
            if section.size > 0
                && other.size > 0
                && section.file_offset < other_end
                && other.file_offset < end
            {
                return Err(format!(
                    "native object plan sections '{}' and '{}' overlap",
                    section_name(section),
                    section_name(other)
                ));
            }
        }
    }
    Ok(())
}

fn validate_relocations(plan: &NativeObjectPlan) -> Result<(), String> {
    let defined = plan.defined_symbols.iter().collect::<HashSet<_>>();
    let imported = plan
        .imported_symbols
        .iter()
        .map(|symbol| &symbol.symbol)
        .collect::<HashSet<_>>();
    let external = plan.external_symbols.iter().collect::<HashSet<_>>();
    for relocation in &plan.relocations {
        if !defined.contains(&relocation.from) {
            return Err(format!(
                "native object plan relocation source '{}' is not defined",
                relocation.from
            ));
        }
        if !defined.contains(&relocation.to)
            && !imported.contains(&relocation.to)
            && !external.contains(&relocation.to)
        {
            return Err(format!(
                "native object plan relocation target '{}' is neither defined nor imported",
                relocation.to
            ));
        }
    }
    Ok(())
}

fn section_name(section: &SectionPlan) -> String {
    section
        .section
        .as_ref()
        .map(|name| format!("{},{}", section.segment, name))
        .unwrap_or_else(|| section.segment.clone())
}

fn push_import(imports: &mut Vec<ObjectImport>, import: ObjectImport) {
    if imports
        .iter()
        .any(|existing| existing.library == import.library && existing.symbol == import.symbol)
    {
        return;
    }
    imports.push(import);
}

fn push_relocation(relocations: &mut Vec<ObjectRelocation>, relocation: ObjectRelocation) {
    if relocations.iter().any(|existing| {
        existing.from == relocation.from
            && existing.to == relocation.to
            && existing.kind == relocation.kind
            && existing.section == relocation.section
    }) {
        return;
    }
    relocations.push(relocation);
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn reject_duplicates(label: &str, values: &[String]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(format!(
                "native object plan has duplicate {label} '{value}'"
            ));
        }
    }
    Ok(())
}

fn align(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

trait ToObjectJson {
    fn to_json(&self, indent: usize) -> String;
}

impl ToObjectJson for LoadCommandPlan {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let name = self
            .name
            .as_ref()
            .map(|name| json_string(name))
            .unwrap_or_else(|| "null".to_string());
        format!(
            "\n{}{{ \"kind\": {}, \"name\": {} }}",
            pad,
            json_string(&self.kind),
            name
        )
    }
}

impl ToObjectJson for SegmentPlan {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{ \"name\": {}, \"vmAddress\": {}, \"vmSize\": {}, ",
                "\"fileOffset\": {}, \"fileSize\": {}, \"maxProtection\": {}, ",
                "\"initialProtection\": {} }}"
            ),
            pad,
            json_string(&self.name),
            self.vm_address,
            self.vm_size,
            self.file_offset,
            self.file_size,
            json_string(&self.max_protection),
            json_string(&self.initial_protection)
        )
    }
}

impl ToObjectJson for SectionPlan {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let section = self
            .section
            .as_ref()
            .map(|section| json_string(section))
            .unwrap_or_else(|| "null".to_string());
        format!(
            concat!(
                "\n{}{{ \"segment\": {}, \"section\": {}, \"kind\": {}, ",
                "\"vmAddress\": {}, \"fileOffset\": {}, \"size\": {}, \"align\": {} }}"
            ),
            pad,
            json_string(&self.segment),
            section,
            json_string(&self.kind),
            self.vm_address,
            self.file_offset,
            self.size,
            self.align
        )
    }
}

impl ToObjectJson for CodeUnitPlan {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{ \"symbol\": {}, \"section\": {}, \"offset\": {}, ",
                "\"plannedSize\": {}, \"operations\": [{}], \"calls\": [{}], \"dataRefs\": [{}] }}"
            ),
            pad,
            json_string(&self.symbol),
            json_string(&self.section),
            self.offset,
            self.planned_size,
            json_string_list(&self.operations),
            json_string_list(&self.calls),
            json_string_list(&self.data_refs)
        )
    }
}

impl ToObjectJson for DataUnitPlan {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"symbol\": {}, \"section\": {}, \"offset\": {}, \"size\": {}, \"value\": {} }}",
            pad,
            json_string(&self.symbol),
            json_string(&self.section),
            self.offset,
            self.size,
            json_string(&self.value)
        )
    }
}

impl ToObjectJson for ObjectImport {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"library\": {}, \"symbol\": {} }}",
            pad,
            json_string(&self.library),
            json_string(&self.symbol)
        )
    }
}

impl ToObjectJson for SymbolPlan {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let section = self
            .section
            .as_ref()
            .map(|section| json_string(section))
            .unwrap_or_else(|| "null".to_string());
        let value = self
            .value
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string());
        format!(
            concat!(
                "\n{}{{ \"name\": {}, \"kind\": {}, \"section\": {}, ",
                "\"value\": {}, \"stringTableOffset\": {} }}"
            ),
            pad,
            json_string(&self.name),
            json_string(&self.kind),
            section,
            value,
            self.string_table_offset
        )
    }
}

impl ToObjectJson for StringTableEntry {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"value\": {}, \"offset\": {} }}",
            pad,
            json_string(&self.value),
            self.offset
        )
    }
}

impl StringTablePlan {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "{{\n{}  \"size\": {},\n{}  \"entries\": [{}\n{}  ]\n{}}}",
            pad,
            self.size,
            pad,
            join_json(&self.entries, indent + 2),
            pad,
            pad
        )
    }
}

impl ToObjectJson for ObjectRelocation {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"from\": {}, \"to\": {}, \"kind\": {}, \"section\": {} }}",
            pad,
            json_string(&self.from),
            json_string(&self.to),
            json_string(&self.kind),
            json_string(&self.section)
        )
    }
}

fn join_json<T: ToObjectJson>(items: &[T], indent: usize) -> String {
    items
        .iter()
        .map(|item| item.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}

fn json_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| json_string(value))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::plan::{
        CallKind, NativePlan, PlanCall, PlannedFunction, PlatformImport, StorageClass, StorageType,
    };

    #[test]
    fn lowers_libsystem_import_to_mach_o_object_plan() {
        let plan = NativePlan {
            target: "macos-aarch64".to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            project: "hello".to_string(),
            entry_symbol: Some("_mfb_fn_main".to_string()),
            runtime_symbols: vec!["_mfb_rt_io_io_print".to_string()],
            external_symbols: Vec::new(),
            platform_imports: vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_write".to_string(),
                required_by: "_mfb_rt_io_io_print".to_string(),
            }],
            functions: vec![PlannedFunction {
                name: "main".to_string(),
                symbol: "_mfb_fn_main".to_string(),
                returns: StorageType {
                    name: "Nothing".to_string(),
                    class: StorageClass::Void,
                    size: 0,
                    align: 1,
                },
                params: Vec::new(),
                local_slots: Vec::new(),
                labels: Vec::new(),
                operations: vec![
                    "eval runtimeCall io io.print(String(\"Hello World\"))".to_string()
                ],
                calls: vec![PlanCall {
                    target: "io.print".to_string(),
                    symbol: "_mfb_rt_io_io_print".to_string(),
                    kind: CallKind::Runtime,
                    string_literals: vec!["Hello World".to_string()],
                }],
            }],
            link_symbols: Vec::new(),
        };

        let object = lower_plan(&plan).expect("object plan");
        assert_eq!(object.container, "mach-o");
        assert_eq!(object.status, "planOnly");
        assert_eq!(object.dylibs, vec!["/usr/lib/libSystem.B.dylib"]);
        assert_eq!(object.imported_symbols[0].symbol, "_write");
        assert_eq!(object.data_units[0].value, "Hello World");
        assert!(object
            .relocations
            .iter()
            .any(|relocation| relocation.kind == "externalCall"));
    }
}
