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
mod operators;
mod optimizer;
mod os;
mod resolver;
mod rules;
mod target;
mod terminal_safe;
mod testing;
#[cfg(test)]
mod testutil;
mod trace;
mod types;
mod unicode;

/// Native stack for the compiler's work thread.
///
/// Every depth guard in the front end admits a tree 256 levels deep
/// (`ast::expr::MAX_EXPR_DEPTH`, `ast::stmt`'s block cap, `parse_type_name`'s
/// type cap — all matched to `ir::verify::check_value_depth`), and every pass
/// after the parser walks that tree recursively. Those caps were calibrated
/// against an 8 MiB main-thread stack, which is what Linux and macOS give
/// `main`; Windows reserves 1 MiB, and there the deepest ADMITTED program did
/// not fit — `mfb build` of 250 nested groups died with a native stack overflow
/// (`0xC00000FD`, "thread 'main' has overflowed its stack") instead of
/// compiling, and the hostile shapes bug-501 taught the parser to reject died
/// the same way before their diagnostic could be printed.
///
/// The guards define the language surface, so the stack is what moves: the
/// compiler runs on a thread whose size it chooses rather than on whatever
/// `main` was handed. 64 MiB is the size the parser's own deep-nesting tests
/// have always used (`ast::expr::tests::on_big_stack`), ~8x the headroom the
/// two platforms that already pass have. The stack is reserved address space,
/// not committed memory, on every supported host.
const COMPILER_STACK_BYTES: usize = 64 * 1024 * 1024;

fn main() {
    // A panic on this thread has already printed through the panic hook by the
    // time `join` reports it; exit 101 so a panicking `mfb` still looks exactly
    // like one that panicked on `main`.
    let compiler = std::thread::Builder::new()
        .name("mfb".to_string())
        .stack_size(COMPILER_STACK_BYTES)
        .spawn(cli::dispatch::run)
        .expect("spawn the compiler thread");
    if compiler.join().is_err() {
        std::process::exit(101);
    }
}
