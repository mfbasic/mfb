use super::commands::bind_info;
use super::*;
use crate::arch::image::{EncodedImport, EncodedRelocation, EncodedSymbol, ImportKind};
// `read_u32` and `symbol_vmaddr` moved to the shared linker-encode module
// (bug-335 A3/A4); the tests still exercise them directly.
use crate::os::link_encode::{read_u32, symbol_vmaddr};

mod bundles_and_debug;
mod dylib_paths;
mod mach_o;

fn import(library: &str, symbol: &str) -> EncodedImport {
    EncodedImport {
        library: library.to_string(),
        symbol: symbol.to_string(),
        kind: ImportKind::Function,
        version: None,
    }
}

/// An empty image with `_main` as the entry: the base every test spreads over
/// with `..none()`, spelling only the fields it actually exercises (bug-335 C2).
fn none() -> EncodedImage {
    EncodedImage {
        text: Vec::new(),
        data: Vec::new(),
        rodata_size: 0,
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
        debug_symbols: false,
    }
}

fn text_symbol(name: &str, offset: usize) -> EncodedSymbol {
    EncodedSymbol {
        name: name.to_string(),
        section: EncodedSection::Text,
        offset,
    }
}

fn data_symbol(name: &str, offset: usize) -> EncodedSymbol {
    EncodedSymbol {
        name: name.to_string(),
        section: EncodedSection::Data,
        offset,
    }
}

// Build a real AArch64 image through the arch encoder so text and relocations are
// self-consistent, then drive the *full* Mach-O byte writer end to end without
// running the binary. This is host-neutral (pure byte generation + a tempfile
// write) so it covers the imports + data + initializer path — data_const_segment,
// data_segment, dyld_info, bind_info, symbol_table, string_table, load_dylib,
// dysymtab, rebase_info, linkedit_layout — on Linux CI too, where the
// `#[cfg(target_os = "macos")]` launch tests do not run.
fn encode_aarch64(
    imports: Vec<crate::codegen::engine::types::CodeImport>,
    data_objects: Vec<crate::codegen::engine::types::CodeDataObject>,
    functions: Vec<crate::codegen::engine::types::CodeFunction>,
    entry: &str,
) -> EncodedImage {
    let plan = crate::codegen::engine::types::NativeCodePlan {
        target: "macos-aarch64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        arch: "aarch64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some(entry.to_string()),
        imports,
        data_objects,
        functions,
    };
    crate::arch::aarch64::encode::encode(&plan).expect("aarch64 encode")
}

fn code_fn(
    name: &str,
    instructions: Vec<crate::codegen::engine::types::CodeInstruction>,
) -> crate::codegen::engine::types::CodeFunction {
    crate::codegen::engine::types::CodeFunction {
        name: name.to_string(),
        symbol: name.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: crate::codegen::engine::types::CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        instructions,
        relocations: Vec::new(),
        stack_slots: Vec::new(),
    }
}

fn inst(
    op: &str,
    fields: &[(&'static str, &str)],
) -> crate::codegen::engine::types::CodeInstruction {
    let mut instruction = crate::codegen::engine::types::CodeInstruction::new(op);
    // Encoder fixtures spell physical registers (bug-721: backend entry point).
    for (key, value) in fields {
        instruction = instruction.physical_field(key, value);
    }
    instruction
}
