use std::collections::HashSet;

use crate::json::json_string;
use crate::os::object_plan::{
    align, join_json, json_string_list, push_unique, reject_duplicates, CodeUnitPlan, DataUnitPlan,
    LoadEntryPlan, ObjectImport, ObjectRelocation, SectionPlan, SegmentPlan, StringTableEntry,
    StringTablePlan, SymbolPlan,
};
use crate::target::shared::plan::{CallKind, NativePlan};

const VM_BASE: u64 = 0x1_0000_0000;
const TEXT_FILE_OFFSET: usize = 0x4000;
const LINKEDIT_FILE_OFFSET: usize = 0x8000;
const MACHO_PAGE_SIZE: usize = 0x4000;

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
    // `__LINKEDIT` follows `__text` + `__cstring`, page-aligned, mirroring the
    // real linker layout in `link.rs` (`macho_layout`). The historical fixed
    // `LINKEDIT_FILE_OFFSET` (0x8000) is kept as a floor so small programs keep
    // their existing object-plan goldens; larger code grows the offset instead
    // of overflowing into `__LINKEDIT` (a large source package like `regex`
    // alone exceeds the old 16 KiB text budget).
    let linkedit_file_offset = std::cmp::max(
        LINKEDIT_FILE_OFFSET,
        align(cstring_file_offset + cstring_size, MACHO_PAGE_SIZE),
    );

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
            vm_address: VM_BASE + linkedit_file_offset as u64,
            file_offset: linkedit_file_offset,
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
                vm_size: linkedit_file_offset,
                file_offset: 0,
                file_size: linkedit_file_offset,
                max_protection: "read-execute".to_string(),
                initial_protection: "read-execute".to_string(),
            },
            SegmentPlan {
                name: "__LINKEDIT".to_string(),
                vm_address: VM_BASE + linkedit_file_offset as u64,
                vm_size: linkedit_size,
                file_offset: linkedit_file_offset,
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

fn load_commands(plan: &NativePlan) -> Vec<LoadEntryPlan> {
    let mut commands = vec![
        LoadEntryPlan {
            kind: "LC_SEGMENT_64".to_string(),
            name: Some("__TEXT".to_string()),
        },
        LoadEntryPlan {
            kind: "LC_SEGMENT_64".to_string(),
            name: Some("__LINKEDIT".to_string()),
        },
        LoadEntryPlan {
            kind: "LC_DYLD_INFO_ONLY".to_string(),
            name: None,
        },
        LoadEntryPlan {
            kind: "LC_SYMTAB".to_string(),
            name: None,
        },
        LoadEntryPlan {
            kind: "LC_DYSYMTAB".to_string(),
            name: None,
        },
        LoadEntryPlan {
            kind: "LC_LOAD_DYLINKER".to_string(),
            name: Some("/usr/lib/dyld".to_string()),
        },
        LoadEntryPlan {
            kind: "LC_BUILD_VERSION".to_string(),
            name: Some("macos".to_string()),
        },
        LoadEntryPlan {
            kind: "LC_MAIN".to_string(),
            name: None,
        },
        LoadEntryPlan {
            kind: "LC_CODE_SIGNATURE".to_string(),
            name: None,
        },
    ];
    for dylib in dylibs(plan).unwrap_or_default() {
        commands.push(LoadEntryPlan {
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
        // Audio (plan-33-B §5): the AudioQueue backend statically imports these
        // three frameworks (no dlopen on macOS).
        "AudioToolbox" => {
            Ok("/System/Library/Frameworks/AudioToolbox.framework/AudioToolbox".to_string())
        }
        "CoreAudio" => Ok("/System/Library/Frameworks/CoreAudio.framework/CoreAudio".to_string()),
        "CoreFoundation" => {
            Ok("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation".to_string())
        }
        "AppKit" => Ok("/System/Library/Frameworks/AppKit.framework/AppKit".to_string()),
        // plan-98-C Phase 3: the canvas frame blit builds a `CGImage`. The install
        // name is versioned (`Versions/A/`), unlike AppKit's and Foundation's above
        // — taken from `dyld_info -dependents` on AppKit, which links it, rather
        // than assumed to follow the same shape as its neighbours.
        // plan-98-E: the Metal renderer. Both install names are versioned
        // (`Versions/A/`), taken from `dyld_info -dependents` on AppKit rather than
        // assumed to match AppKit's own unversioned shape.
        "Metal" => Ok("/System/Library/Frameworks/Metal.framework/Versions/A/Metal".to_string()),
        // plan-148-B: `canvas::systemFontTable`'s CoreText enumeration. Versioned, as
        // `otool -L` reports it for a binary linking CoreText.
        "CoreText" => {
            Ok("/System/Library/Frameworks/CoreText.framework/Versions/A/CoreText".to_string())
        }
        "QuartzCore" => {
            Ok("/System/Library/Frameworks/QuartzCore.framework/Versions/A/QuartzCore".to_string())
        }
        "CoreGraphics" => Ok(
            "/System/Library/Frameworks/CoreGraphics.framework/Versions/A/CoreGraphics".to_string(),
        ),
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

#[cfg(test)]
mod tests;
