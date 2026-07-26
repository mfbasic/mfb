mod arch;
mod ast;
mod audit;
mod binary_repr;
mod builtins;
mod cli;
mod doc;
mod docs;
mod fmt;
mod html;
mod internal_name;
mod ir;
mod lexer;
mod manifest;
mod monomorph;
mod numeric;
mod os;
mod resolver;
mod rules;
mod scope_privates;
mod syntaxcheck;
mod target;
mod terminal_safe;
mod testing;
#[cfg(test)]
mod testutil;
mod unicode;

use tinyjson::JsonValue;

fn main() {
    cli::dispatch::run();
}

pub(crate) fn json_string(value: &str) -> String {
    JsonValue::String(value.to_string())
        .stringify()
        .unwrap_or_else(|_| "\"mfb_project\"".to_string())
}
