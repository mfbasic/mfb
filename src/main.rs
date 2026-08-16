mod arch;
mod ast;
mod audit;
mod binary_repr;
mod builtins;
mod cli;
mod codegen;
mod doc;
mod docs;
mod fmt;
mod html;
mod internal_name;
mod ir;
mod json;
mod lexer;
mod manifest;
mod monomorph;
mod numeric;
mod os;
mod resolver;
mod rules;
mod syntaxcheck;
mod target;
mod terminal_safe;
mod testing;
#[cfg(test)]
mod testutil;
mod types;
mod unicode;

fn main() {
    cli::dispatch::run();
}
