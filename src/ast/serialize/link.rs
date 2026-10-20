//! plan-176-M: AST JSON serialization of the native-binding nodes — `RESOURCE`, `FUNC`
//! alias, `LINK`, C structs, bind-in and ABI specs. The project/file entry points and the
//! shared helpers live in `ast/serialize/mod.rs`.

use super::*;

impl ToJson for ResourceDecl {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"kind\": \"resource\", \"visibility\": {}, \"name\": {}, \"closeFn\": {}, \"threadSendable\": {}, \"line\": {} }}",
            pad,
            json_string(visibility_name(self.visibility)),
            json_string(&self.name),
            json_string(&self.close_fn),
            self.thread_sendable,
            self.line
        )
    }
}

impl ToJson for FuncAlias {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"kind\": \"funcAlias\", \"visibility\": {}, \"name\": {}, \"target\": {}, \"line\": {} }}",
            pad,
            json_string(visibility_name(self.visibility)),
            json_string(&self.name),
            json_string(&self.target),
            self.line
        )
    }
}

impl ToJson for LinkBlock {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": \"link\",\n",
                "{}  \"library\": {},\n",
                "{}  \"alias\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"cstructs\": [{}\n{}  ],\n",
                "{}  \"functions\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            pad,
            json_string(&self.library),
            pad,
            json_string(&self.alias),
            pad,
            self.line,
            pad,
            join_json(&self.cstructs, indent + 2),
            pad,
            pad,
            join_json(&self.functions, indent + 2),
            pad,
            pad
        )
    }
}

impl ToJson for CStructDecl {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": \"cstruct\",\n",
                "{}  \"name\": {},\n",
                "{}  \"mapsTo\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"fields\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.maps_to),
            pad,
            self.line,
            pad,
            join_json(&self.fields, indent + 2),
            pad,
            pad
        )
    }
}

impl ToJson for CStructField {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"ctype\": {},\n",
                "{}  \"line\": {}\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.ctype),
            pad,
            self.line,
            pad
        )
    }
}

/// bug-300 E3: `BIND IN` blocks and their fields were absent from the `-ast` dump
/// entirely, so a native FUNC carrying one dumped identically to one without.
impl ToJson for BindIn {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": \"bindIn\",\n",
                "{}  \"slot\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"fields\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            pad,
            json_string(&self.slot),
            pad,
            self.line,
            pad,
            join_json(&self.fields, indent + 2),
            pad,
            pad
        )
    }
}

impl ToJson for BindInField {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": \"bindInField\",\n",
                "{}  \"name\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"value\": {}\n",
                "{}}}"
            ),
            pad,
            pad,
            pad,
            json_string(&self.name),
            pad,
            self.line,
            pad,
            self.value.to_json(indent + 2),
            pad
        )
    }
}

impl BindState {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "{{\n",
                "{}  \"kind\": \"bindState\",\n",
                "{}  \"resourceSlot\": {},\n",
                "{}  \"structSlot\": {}\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.resource_slot),
            pad,
            json_string(&self.struct_slot),
            pad
        )
    }
}

impl ToJson for LinkFunction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let return_type = self
            .return_type
            .as_ref()
            .map(|value| json_string(value))
            .unwrap_or_else(|| "null".to_string());
        let success_on = self
            .success_on
            .as_ref()
            .map(|value| value.to_json(indent + 2))
            .unwrap_or_else(|| "null".to_string());
        let result = self
            .result
            .as_ref()
            .map(|value| value.to_json(indent + 2))
            .unwrap_or_else(|| "null".to_string());
        let free = self
            .free
            .as_ref()
            .map(|value| value.to_json(indent + 2))
            .unwrap_or_else(|| "null".to_string());
        // bug-300 E3: these three were omitted, so a native
        // `FUNC … AS RES SoundFile STATE FileInfo` with `BIND IN`/`BIND STATE`
        // dumped identically to one without them. `Function::to_json` already
        // emits `returnState`, so the LINK side was the asymmetric one.
        let return_state = self
            .return_state_type
            .as_ref()
            .map(|value| json_string(value))
            .unwrap_or_else(|| "null".to_string());
        let bind_state = self
            .bind_state
            .as_ref()
            .map(|value| value.to_json(indent + 2))
            .unwrap_or_else(|| "null".to_string());
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": \"linkFunc\",\n",
                "{}  \"name\": {},\n",
                "{}  \"symbol\": {},\n",
                "{}  \"returnResource\": {},\n",
                "{}  \"returnType\": {},\n",
                "{}  \"returnState\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"abi\": {},\n",
                "{}  \"consts\": [{}\n{}  ],\n",
                "{}  \"bindIn\": [{}\n{}  ],\n",
                "{}  \"bindState\": {},\n",
                "{}  \"successOn\": {},\n",
                "{}  \"result\": {},\n",
                "{}  \"free\": {}\n",
                "{}}}"
            ),
            pad,
            pad,
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.symbol),
            pad,
            self.return_resource,
            pad,
            return_type,
            pad,
            return_state,
            pad,
            self.line,
            pad,
            join_json(&self.params, indent + 2),
            pad,
            pad,
            self.abi.to_json(indent + 2),
            pad,
            join_json(&self.consts, indent + 2),
            pad,
            pad,
            join_json(&self.bind_in, indent + 2),
            pad,
            pad,
            bind_state,
            pad,
            success_on,
            pad,
            result,
            pad,
            free,
            pad
        )
    }
}

impl FreeSpec {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "{{\n",
                "{}  \"slot\": {},\n",
                "{}  \"symbol\": {},\n",
                "{}  \"paramName\": {},\n",
                "{}  \"paramCType\": {},\n",
                "{}  \"returnCType\": {}\n",
                "{}}}"
            ),
            pad,
            json_string(&self.slot),
            pad,
            json_string(&self.symbol),
            pad,
            json_string(&self.param_name),
            pad,
            json_string(&self.param_ctype),
            pad,
            json_string(&self.return_ctype),
            pad
        )
    }
}

impl AbiSpec {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "{{\n",
                "{}  \"slots\": [{}\n{}  ],\n",
                "{}  \"returnName\": {},\n",
                "{}  \"returnCType\": {}\n",
                "{}}}"
            ),
            pad,
            join_json(&self.slots, indent + 2),
            pad,
            pad,
            json_string(&self.return_name),
            pad,
            json_string(&self.return_ctype),
            pad
        )
    }
}

impl ToJson for AbiSlot {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"ctype\": {}, \"out\": {}, \"line\": {} }}",
            pad,
            json_string(&self.name),
            json_string(&self.ctype),
            self.direction.writes_back(),
            self.line
        )
    }
}

impl ToJson for ConstPin {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"slot\": {}, \"value\": {}, \"line\": {} }}",
            pad,
            json_string(&self.slot),
            self.value.to_json(indent),
            self.line
        )
    }
}
