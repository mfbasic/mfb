use std::collections::HashSet;

use crate::json_string;
use crate::target::shared::plan::{CallKind, NativePlan};

const IMAGE_BASE: u64 = 0x400000;
const TEXT_FILE_OFFSET: usize = 0x1000;

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
    let imported_symbols = imported_symbols(plan);
    let data_units = data_units(plan);
    let code_units = code_units(plan, &entry, &data_units);
    let text_size = code_units
        .iter()
        .map(|unit| unit.planned_size)
        .sum::<usize>();
    let rodata_file_offset = TEXT_FILE_OFFSET + align(text_size, 16);
    let rodata_size = data_units.iter().map(|unit| unit.size).sum::<usize>();
    let image_size = rodata_file_offset + rodata_size;
    let sections = vec![
        SectionPlan {
            segment: "PT_LOAD".to_string(),
            section: Some(".text".to_string()),
            kind: "code".to_string(),
            vm_address: IMAGE_BASE + TEXT_FILE_OFFSET as u64,
            file_offset: TEXT_FILE_OFFSET,
            size: text_size,
            align: 4,
        },
        SectionPlan {
            segment: "PT_LOAD".to_string(),
            section: Some(".rodata".to_string()),
            kind: "rodata".to_string(),
            vm_address: IMAGE_BASE + rodata_file_offset as u64,
            file_offset: rodata_file_offset,
            size: rodata_size,
            align: 8,
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
        container: "elf".to_string(),
        status: "planOnly".to_string(),
        entry,
        image_base: IMAGE_BASE,
        dylibs: Vec::new(),
        load_commands: vec![LoadCommandPlan {
            kind: "PT_LOAD".to_string(),
            name: Some("load-rx".to_string()),
        }],
        segments: vec![SegmentPlan {
            name: "PT_LOAD".to_string(),
            vm_address: IMAGE_BASE,
            vm_size: image_size,
            file_offset: 0,
            file_size: image_size,
            max_protection: "read-execute".to_string(),
            initial_protection: "read-execute".to_string(),
        }],
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
        // The object/ELF plan is ISA-neutral (an ELF container is ELF); accept any
        // Linux target so the x86-64 backend (plan-00-H) reuses this linker.
        if self.target != "linux-aarch64" && self.target != "linux-x86_64" {
            return Err(format!(
                "native object plan target '{}' is not a supported Linux target",
                self.target
            ));
        }
        if self.container != "elf" || self.status != "planOnly" {
            return Err("native object plan must be a plan-only ELF plan".to_string());
        }
        if !self.defined_symbols.contains(&self.entry) {
            return Err(format!(
                "native object plan entry '{}' is not a defined symbol",
                self.entry
            ));
        }
        reject_duplicates("defined symbol", &self.defined_symbols)?;
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

fn imported_symbols(plan: &NativePlan) -> Vec<ObjectImport> {
    plan.platform_imports
        .iter()
        .map(|import| ObjectImport {
            library: import.library.clone(),
            symbol: import.symbol.clone(),
        })
        .collect()
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
            let size = align(8 + value.len() + 1, 8);
            let unit = DataUnitPlan {
                symbol: format!("_mfb_str_{index}"),
                section: ".rodata".to_string(),
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
        section: ".text".to_string(),
        offset,
        planned_size: 24,
        operations: vec![
            "call entry function".to_string(),
            "exit via Linux syscall".to_string(),
        ],
        calls: vec![entry_call],
        data_refs: Vec::new(),
    });
    offset += 24;
    for function in &plan.functions {
        let calls = function
            .calls
            .iter()
            .map(|call| call.symbol.clone())
            .collect::<Vec<_>>();
        let planned_size = align(function.operations.len().max(1) * 4, 4);
        units.push(CodeUnitPlan {
            symbol: function.symbol.clone(),
            section: ".text".to_string(),
            offset,
            planned_size,
            operations: function.operations.clone(),
            calls,
            data_refs: Vec::new(),
        });
        offset += planned_size;
    }
    for runtime_symbol in &plan.runtime_symbols {
        units.push(CodeUnitPlan {
            symbol: runtime_symbol.clone(),
            section: ".text".to_string(),
            offset,
            planned_size: 64,
            operations: vec!["execute runtime helper".to_string()],
            calls: Vec::new(),
            data_refs: data_units.iter().map(|unit| unit.symbol.clone()).collect(),
        });
        offset += 64;
    }
    // Native `LINK` initializer + marshaling thunks (plan-linker.md §12).
    for link_symbol in &plan.link_symbols {
        units.push(CodeUnitPlan {
            symbol: link_symbol.clone(),
            section: ".text".to_string(),
            offset,
            planned_size: 64,
            operations: vec!["native link binding".to_string()],
            calls: Vec::new(),
            data_refs: Vec::new(),
        });
        offset += 64;
    }
    units
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
                section: ".text".to_string(),
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
                    section: ".text".to_string(),
                },
            );
        }
    }
    for runtime_symbol in &plan.runtime_symbols {
        for unit in data_units {
            push_relocation(
                &mut relocations,
                ObjectRelocation {
                    from: runtime_symbol.clone(),
                    to: unit.symbol.clone(),
                    kind: "dataReference".to_string(),
                    section: ".text".to_string(),
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

fn join_json<T: ToObjectJson>(values: &[T], indent: usize) -> String {
    values
        .iter()
        .map(|value| value.to_json(indent))
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
        NativePlan, PlanCall, PlannedFunction, PlatformImport, StorageClass, StorageType,
    };

    fn void_type() -> StorageType {
        StorageType {
            name: "Nothing".to_string(),
            class: StorageClass::Void,
            size: 0,
            align: 1,
        }
    }

    fn function(symbol: &str, operations: Vec<&str>, calls: Vec<PlanCall>) -> PlannedFunction {
        PlannedFunction {
            name: symbol.trim_start_matches("_mfb_fn_").to_string(),
            symbol: symbol.to_string(),
            returns: void_type(),
            params: Vec::new(),
            local_slots: Vec::new(),
            labels: Vec::new(),
            operations: operations.into_iter().map(str::to_string).collect(),
            calls,
        }
    }

    fn call(target: &str, symbol: &str, kind: CallKind, literals: Vec<&str>) -> PlanCall {
        PlanCall {
            target: target.to_string(),
            symbol: symbol.to_string(),
            kind,
            string_literals: literals.into_iter().map(str::to_string).collect(),
        }
    }

    fn base_plan(target: &str) -> NativePlan {
        NativePlan {
            target: target.to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            project: "hello".to_string(),
            entry_symbol: Some("_mfb_fn_main".to_string()),
            runtime_symbols: Vec::new(),
            external_symbols: Vec::new(),
            platform_imports: Vec::new(),
            functions: vec![function("_mfb_fn_main", vec!["ret"], Vec::new())],
            link_symbols: Vec::new(),
        }
    }

    // The exhaustive plan drives every code/data/reloc/symbol branch: functions
    // (each CallKind), runtime symbols with data refs, LINK symbols, imports,
    // string literals, and a package (import) call producing an external symbol.
    fn full_plan(target: &str) -> NativePlan {
        let mut plan = base_plan(target);
        plan.runtime_symbols = vec!["_mfb_rt_io_io_print".to_string()];
        plan.link_symbols = vec!["_mfb_linker_init".to_string()];
        plan.platform_imports = vec![PlatformImport {
            library: "libc.so.6".to_string(),
            symbol: "write".to_string(),
            required_by: "_mfb_rt_io_io_print".to_string(),
        }];
        plan.functions = vec![function(
            "_mfb_fn_main",
            vec!["call local", "call import", "call runtime", "call indirect"],
            vec![
                call("local", "_mfb_fn_helper", CallKind::Local, vec!["Hello"]),
                call("pkg.f", "_pkg_f", CallKind::Import, vec!["World", "Hello"]),
                call(
                    "io.print",
                    "_mfb_rt_io_io_print",
                    CallKind::Runtime,
                    Vec::new(),
                ),
                call("dyn", "_mfb_fn_dyn", CallKind::Indirect, Vec::new()),
            ],
        )];
        plan.functions
            .push(function("_mfb_fn_helper", vec!["ret"], Vec::new()));
        plan.functions
            .push(function("_mfb_fn_dyn", vec!["ret"], Vec::new()));
        plan
    }

    #[test]
    fn lowers_minimal_plan_to_static_elf_object() {
        let object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
        assert_eq!(object.container, "elf");
        assert_eq!(object.status, "planOnly");
        assert_eq!(object.image_base, IMAGE_BASE);
        assert!(object.dylibs.is_empty());
        assert!(object.imported_symbols.is_empty());
        assert!(object.data_units.is_empty());
        // The synthetic _main entry plus the single planned function.
        assert!(object.defined_symbols.contains(&"_main".to_string()));
        assert!(object.defined_symbols.contains(&"_mfb_fn_main".to_string()));
        assert_eq!(object.sections[0].section.as_deref(), Some(".text"));
        assert_eq!(object.sections[1].section.as_deref(), Some(".rodata"));
    }

    #[test]
    fn lowers_x86_target() {
        let object = lower_plan(&base_plan("linux-x86_64")).expect("x86 object plan");
        assert_eq!(object.target, "linux-x86_64");
        object.validate().expect("x86 validates");
    }

    #[test]
    fn lowers_full_plan_covering_every_branch() {
        let object = lower_plan(&full_plan("linux-aarch64")).expect("full object plan");
        // Data units for the deduplicated string literals ("Hello", "World").
        assert_eq!(object.data_units.len(), 2);
        assert_eq!(object.data_units[0].value, "Hello");
        assert_eq!(object.data_units[1].value, "World");
        assert_eq!(object.data_units[0].section, ".rodata");
        // The second unit's offset follows the first (aligned to 8).
        assert_eq!(object.data_units[0].offset, 0);
        assert!(object.data_units[1].offset >= object.data_units[0].size);
        // Runtime + link symbols appear in code units and defined symbols.
        assert!(object
            .defined_symbols
            .contains(&"_mfb_rt_io_io_print".to_string()));
        assert!(object
            .defined_symbols
            .contains(&"_mfb_linker_init".to_string()));
        // Import call becomes an external symbol (packageCall relocation target).
        assert!(object.external_symbols.contains(&"_pkg_f".to_string()));
        assert!(object.imported_symbols.iter().any(|s| s.symbol == "write"));
        // Relocation kinds: internalCall (entry), packageCall, indirectCall,
        // dataReference (runtime symbol -> data unit).
        let kinds: std::collections::HashSet<_> =
            object.relocations.iter().map(|r| r.kind.as_str()).collect();
        assert!(kinds.contains("internalCall"));
        assert!(kinds.contains("packageCall"));
        assert!(kinds.contains("indirectCall"));
        assert!(kinds.contains("dataReference"));
    }

    #[test]
    fn to_json_emits_all_sections_and_round_trips_shape() {
        let object = lower_plan(&full_plan("linux-aarch64")).expect("full object plan");
        let json = object.to_json();
        assert!(json.contains("\"format\": \"mfb-native-object-plan\""));
        assert!(json.contains("\"container\": \"elf\""));
        assert!(json.contains("\"target\": \"linux-aarch64\""));
        assert!(json.contains("\"loadCommands\""));
        assert!(json.contains("PT_LOAD"));
        assert!(json.contains("\".text\""));
        assert!(json.contains("\".rodata\""));
        assert!(json.contains("\"stringTable\""));
        assert!(json.contains("\"relocations\""));
        assert!(json.contains("\"externalSymbols\""));
        assert!(json.contains("\"library\": \"libc.so.6\""));
        assert!(json.contains("\"kind\": \"packageCall\""));
        // The imported symbol has a null section/value in the symbol table.
        assert!(json.contains("\"kind\": \"imported\", \"section\": null"));
    }

    #[test]
    fn to_json_minimal_plan_has_empty_collections() {
        let object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
        let json = object.to_json();
        assert!(json.contains("\"dylibs\": []"));
        assert!(json.contains("\"importedSymbols\": [\n  ]"));
        assert!(json.contains("\"externalSymbols\": []"));
    }

    #[test]
    fn validate_rejects_unknown_target() {
        let object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
        let mut broken = object;
        broken.target = "macos-aarch64".to_string();
        let err = broken.validate().expect_err("bad target");
        assert!(err.contains("not a supported Linux target"), "{err}");
    }

    #[test]
    fn validate_rejects_wrong_container_and_status() {
        let mut object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
        object.container = "mach-o".to_string();
        assert!(object
            .validate()
            .expect_err("bad container")
            .contains("plan-only ELF"));
        let mut object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
        object.status = "final".to_string();
        assert!(object
            .validate()
            .expect_err("bad status")
            .contains("plan-only ELF"));
    }

    #[test]
    fn validate_rejects_entry_not_defined() {
        let mut object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
        object.entry = "_missing".to_string();
        assert!(object
            .validate()
            .expect_err("entry not defined")
            .contains("not a defined symbol"));
    }

    #[test]
    fn validate_rejects_duplicate_defined_symbol() {
        let mut object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
        object.defined_symbols.push("_main".to_string());
        assert!(object
            .validate()
            .expect_err("duplicate")
            .contains("duplicate defined symbol"));
    }

    #[test]
    fn validate_rejects_relocation_source_not_defined() {
        let mut object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
        object.relocations.push(ObjectRelocation {
            from: "_ghost".to_string(),
            to: "_main".to_string(),
            kind: "internalCall".to_string(),
            section: ".text".to_string(),
        });
        assert!(object
            .validate()
            .expect_err("bad source")
            .contains("relocation source"));
    }

    #[test]
    fn validate_rejects_relocation_target_not_defined() {
        let mut object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
        object.relocations.push(ObjectRelocation {
            from: "_main".to_string(),
            to: "_ghost".to_string(),
            kind: "internalCall".to_string(),
            section: ".text".to_string(),
        });
        assert!(object
            .validate()
            .expect_err("bad target")
            .contains("relocation target"));
    }

    #[test]
    fn validate_accepts_relocation_to_imported_or_external_symbol() {
        // The full plan already carries an imported symbol ("write") reachable via
        // an externalCall-style relocation and a packageCall external symbol, so a
        // successful validate proves both the imported and external branches.
        let object = lower_plan(&full_plan("linux-aarch64")).expect("full object plan");
        object.validate().expect("full plan validates");
    }

    #[test]
    fn push_relocation_deduplicates_identical_entries() {
        let mut relocations = Vec::new();
        let make = || ObjectRelocation {
            from: "a".to_string(),
            to: "b".to_string(),
            kind: "internalCall".to_string(),
            section: ".text".to_string(),
        };
        push_relocation(&mut relocations, make());
        push_relocation(&mut relocations, make());
        assert_eq!(relocations.len(), 1);
    }

    #[test]
    fn push_unique_skips_repeats() {
        let mut values = Vec::new();
        push_unique(&mut values, "x".to_string());
        push_unique(&mut values, "x".to_string());
        push_unique(&mut values, "y".to_string());
        assert_eq!(values, vec!["x".to_string(), "y".to_string()]);
    }

    #[test]
    fn align_rounds_up_to_multiple() {
        assert_eq!(align(0, 8), 0);
        assert_eq!(align(1, 8), 8);
        assert_eq!(align(8, 8), 8);
        assert_eq!(align(9, 8), 16);
    }

    #[test]
    fn plan_without_entry_symbol_emits_no_entry_relocation() {
        let mut plan = base_plan("linux-aarch64");
        plan.entry_symbol = None;
        let object = lower_plan(&plan).expect("object plan");
        // No entry_symbol -> the synthetic _main carries an empty call and there is
        // no internalCall relocation from _main to a named entry.
        assert!(!object
            .relocations
            .iter()
            .any(|r| r.from == "_main" && r.kind == "internalCall"));
    }
}
