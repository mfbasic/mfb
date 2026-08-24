mod arch;
mod ast;
mod audit;
mod binary_repr;
mod cli;
mod codegen;
mod doc;
mod docs;
mod fmt;
// Not yet consumed by the build path; `ir::lower` is repointed onto the HIR in
// plan-102-C3, so until then its types/fields read as dead code.
#[allow(dead_code)] // wired into ir::lower in plan-102-C3
mod hir;
mod html;
mod intern;
mod internal_name;
mod ir;
mod json;
mod lexer;
mod manifest;
mod monomorph;
mod numeric;
mod optimizer;
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
