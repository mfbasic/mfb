//! ISA-neutral native object-plan model, shared by the Mach-O
//! (`macos/object.rs`) and ELF (`linux/object.rs`) writers. Holds the plan
//! structs, their `ToObjectJson` rendering, and the small dedup/align helpers.
//! The format-specific lowering (`lower_plan`) and validation stay in each
//! platform module; only the neutral half lives here (bug-335 A1/A2/A5).

use std::collections::HashSet;

use crate::json::json_string;

/// One container directive: a Mach-O load command or an ELF program header. The
/// two writers spell the concept differently but emit the same
/// `{ "kind", "name" }` shape, so the neutral name keeps one struct. The frozen
/// `.nobj` JSON key stays `loadCommands` regardless — see each writer's
/// `to_json`, where the key is a literal.
pub(in crate::os) struct LoadEntryPlan {
    pub(in crate::os) kind: String,
    pub(in crate::os) name: Option<String>,
}

pub(in crate::os) struct SegmentPlan {
    pub(in crate::os) name: String,
    pub(in crate::os) vm_address: u64,
    pub(in crate::os) vm_size: usize,
    pub(in crate::os) file_offset: usize,
    pub(in crate::os) file_size: usize,
    pub(in crate::os) max_protection: String,
    pub(in crate::os) initial_protection: String,
}

pub(in crate::os) struct SectionPlan {
    pub(in crate::os) segment: String,
    pub(in crate::os) section: Option<String>,
    pub(in crate::os) kind: String,
    pub(in crate::os) vm_address: u64,
    pub(in crate::os) file_offset: usize,
    pub(in crate::os) size: usize,
    pub(in crate::os) align: usize,
}

pub(in crate::os) struct CodeUnitPlan {
    pub(in crate::os) symbol: String,
    pub(in crate::os) section: String,
    pub(in crate::os) offset: usize,
    pub(in crate::os) planned_size: usize,
    pub(in crate::os) operations: Vec<String>,
    pub(in crate::os) calls: Vec<String>,
    pub(in crate::os) data_refs: Vec<String>,
}

pub(in crate::os) struct DataUnitPlan {
    pub(in crate::os) symbol: String,
    pub(in crate::os) section: String,
    pub(in crate::os) offset: usize,
    pub(in crate::os) size: usize,
    pub(in crate::os) value: String,
}

pub(in crate::os) struct ObjectImport {
    pub(in crate::os) library: String,
    pub(in crate::os) symbol: String,
}

pub(in crate::os) struct SymbolPlan {
    pub(in crate::os) name: String,
    pub(in crate::os) kind: String,
    pub(in crate::os) section: Option<String>,
    pub(in crate::os) value: Option<u64>,
    pub(in crate::os) string_table_offset: usize,
}

pub(in crate::os) struct StringTablePlan {
    pub(in crate::os) size: usize,
    pub(in crate::os) entries: Vec<StringTableEntry>,
}

pub(in crate::os) struct StringTableEntry {
    pub(in crate::os) value: String,
    pub(in crate::os) offset: usize,
}

pub(in crate::os) struct ObjectRelocation {
    pub(in crate::os) from: String,
    pub(in crate::os) to: String,
    pub(in crate::os) kind: String,
    pub(in crate::os) section: String,
}

pub(in crate::os) trait ToObjectJson {
    fn to_json(&self, indent: usize) -> String;
}

impl ToObjectJson for LoadEntryPlan {
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

impl StringTablePlan {
    pub(in crate::os) fn to_json(&self, indent: usize) -> String {
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

pub(in crate::os) fn join_json<T: ToObjectJson>(items: &[T], indent: usize) -> String {
    items
        .iter()
        .map(|item| item.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}

pub(in crate::os) fn json_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| json_string(value))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(in crate::os) fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

pub(in crate::os) fn reject_duplicates(label: &str, values: &[String]) -> Result<(), String> {
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

/// Round `value` up to the next multiple of `alignment` (the general `div_ceil`
/// form). `alignment == 0` returns `value` unchanged rather than dividing by
/// zero — the most defensive of the former five copies (bug-335 A5).
pub(in crate::os) fn align(value: usize, alignment: usize) -> usize {
    if alignment == 0 {
        return value;
    }
    value.div_ceil(alignment) * alignment
}
