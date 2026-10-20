use std::collections::HashSet;

use crate::json::json_string;
use crate::os::object_plan::{
    align, join_json, json_string_list, push_unique, reject_duplicates, CodeUnitPlan, DataUnitPlan,
    LoadEntryPlan, ObjectImport, ObjectRelocation, SectionPlan, SegmentPlan, StringTableEntry,
    StringTablePlan, SymbolPlan,
};
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
    load_commands: Vec<LoadEntryPlan>,
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
        load_commands: vec![LoadEntryPlan {
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
        if self.target != "linux-aarch64"
            && self.target != "linux-x86_64"
            && self.target != "linux-riscv64"
        {
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
                // An indirect call dispatches through a `FUNC`-typed runtime value
                // and has no symbol; emit no relocation (bug-72).
                CallKind::Indirect => continue,
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

#[cfg(test)]
mod tests;
