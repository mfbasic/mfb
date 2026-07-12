use super::*;

use crate::json_string;

pub(super) trait ToPlanJson {
    fn to_json(&self, indent: usize) -> String;
}

impl ToPlanJson for PlatformImport {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"library\": {}, \"symbol\": {}, \"requiredBy\": {} }}",
            pad,
            json_string(&self.library),
            json_string(&self.symbol),
            json_string(&self.required_by)
        )
    }
}

impl ToPlanJson for PlannedFunction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"symbol\": {},\n",
                "{}  \"returns\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"localSlots\": [{}\n{}  ],\n",
                "{}  \"labels\": [{}\n{}  ],\n",
                "{}  \"operations\": [{}],\n",
                "{}  \"calls\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.symbol),
            pad,
            self.returns.to_json(0),
            pad,
            join_json(&self.params, indent + 2),
            pad,
            pad,
            join_json(&self.local_slots, indent + 2),
            pad,
            pad,
            join_json(&self.labels, indent + 2),
            pad,
            pad,
            json_string_list(&self.operations),
            pad,
            join_json(&self.calls, indent + 2),
            pad,
            pad
        )
    }
}

impl ToPlanJson for PlannedParam {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"storage\": {} }}",
            pad,
            json_string(&self.name),
            self.storage.to_json(0)
        )
    }
}

impl ToPlanJson for StackSlot {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"storage\": {}, \"offset\": {}, \"mutable\": {} }}",
            pad,
            json_string(&self.name),
            self.storage.to_json(0),
            self.offset,
            self.mutable
        )
    }
}

impl ToPlanJson for PlanLabel {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"kind\": {} }}",
            pad,
            json_string(&self.name),
            json_string(self.kind.name())
        )
    }
}

impl ToPlanJson for PlanCall {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"target\": {}, \"symbol\": {}, \"kind\": {}, \"stringLiterals\": [{}] }}",
            pad,
            json_string(&self.target),
            json_string(&self.symbol),
            json_string(self.kind.name()),
            json_string_list(&self.string_literals)
        )
    }
}

impl ToPlanJson for StorageType {
    fn to_json(&self, _indent: usize) -> String {
        format!(
            "{{ \"name\": {}, \"class\": {}, \"size\": {}, \"align\": {} }}",
            json_string(&self.name),
            json_string(self.class.name()),
            self.size,
            self.align
        )
    }
}

impl LabelKind {
    fn name(&self) -> &'static str {
        match self {
            LabelKind::IfElse => "ifElse",
            LabelKind::IfEnd => "ifEnd",
            LabelKind::MatchCase => "matchCase",
            LabelKind::MatchEnd => "matchEnd",
            LabelKind::WhileLoop => "whileLoop",
            LabelKind::WhileEnd => "whileEnd",
        }
    }
}

impl CallKind {
    fn name(&self) -> &'static str {
        match self {
            CallKind::Local => "local",
            CallKind::Import => "import",
            CallKind::Runtime => "runtime",
            CallKind::Indirect => "indirect",
        }
    }
}

impl StorageClass {
    fn name(&self) -> &'static str {
        match self {
            StorageClass::Void => "void",
            StorageClass::Byte => "byte",
            StorageClass::Integer => "integer",
            StorageClass::Float => "float",
            StorageClass::Fixed => "fixed",
            StorageClass::Money => "money",
            StorageClass::Boolean => "boolean",
            StorageClass::Reference => "reference",
        }
    }
}

pub(super) fn join_json<T: ToPlanJson>(items: &[T], indent: usize) -> String {
    items
        .iter()
        .map(|item| item.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn json_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| json_string(value))
        .collect::<Vec<_>>()
        .join(", ")
}
