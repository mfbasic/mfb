//! The ISA-neutral linkable-image container types shared by every backend
//! encoder and both linkers.
//!
//! These describe a *linkable image* — its text/data bytes, symbols,
//! relocations, and imports — not any instruction set, so they are independent of
//! which `arch::<isa>::encode` produced them. They lived inside
//! `arch::aarch64::encode` historically (the AArch64 backend was written first)
//! and were re-exported verbatim by the x86_64/riscv64 encoders; bug-341-B2
//! relocated them here so no sibling backend or linker reaches through the
//! AArch64 module for an ISA-independent type.
//!
//! It also owns the ISA-neutral relocation-binding classification ([`Binding`],
//! [`resolve_call_binding`], [`resolve_data_binding`]) every backend's
//! symbol-reference emitter applied identically (bug-341-B5): the
//! internal/external/data split is the same on every target; only the per-ISA
//! `kind` string and the choice of instruction stay in the emitter.

use std::collections::HashMap;

/// A fully encoded, linkable image: the raw text/data bytes plus the symbol,
/// relocation, and import tables a linker needs to lay it out.
pub(crate) struct EncodedImage {
    pub(crate) text: Vec<u8>,
    pub(crate) data: Vec<u8>,
    /// Page-aligned length of the read-only constant prefix of `data` (bug-187).
    /// The linker maps `data[..rodata_size]` read-only and `data[rodata_size..]`
    /// R+W (the arena global and other mutable runtime globals). 0 = no read-only
    /// partition (the whole data segment stays writable).
    pub(crate) rodata_size: usize,
    pub(crate) symbols: Vec<EncodedSymbol>,
    pub(crate) relocations: Vec<EncodedRelocation>,
    pub(crate) imports: Vec<EncodedImport>,
    pub(crate) entry: String,
    /// Internal text symbols run, in order, after dynamic relocations and before
    /// the program entry (plan-linker.md §5.3). Materialized as ELF
    /// `DT_INIT_ARRAY` / Mach-O `S_MOD_INIT_FUNC_POINTERS`.
    pub(crate) initializers: Vec<String>,
    pub(crate) signing_metadata: Option<Vec<u8>>,
    /// Loader search paths for `dlopen`ing vendored native libraries
    /// (plan-46-D §4.2/§4.3), materialized as ELF `DT_RUNPATH` / Mach-O
    /// `LC_RPATH`.
    ///
    /// **Empty for every build that vendors nothing** — which is every build with
    /// only `system` locators — so no tag or load command is emitted and the
    /// binary stays byte-identical to a pre-plan-46 one.
    ///
    /// The strings are loader-relative and per output shape: `$ORIGIN/vendor`
    /// (ELF), `@loader_path/vendor` (macOS console), or
    /// `@executable_path/../Frameworks` (macOS `.app`). Chosen by the caller, not
    /// the encoder — which is what keeps the vendor directory's *location* out of
    /// the codegen and the `dlopen` call a bare-filename one.
    pub(crate) rpaths: Vec<String>,
}

/// Whether an imported symbol names a function (called through a stub) or a data
/// global (addressed through the GOT). Makes linker layout deterministic without
/// scanning relocations (plan-linker.md §5.1). `Data` is produced by a
/// `tls`/app-mode consumer (and the linker tests) once one exists; the built-in
/// surface is function-only, so allow it to be otherwise-unconstructed for now.
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ImportKind {
    Function,
    Data,
}

pub(crate) struct EncodedSymbol {
    pub(crate) name: String,
    pub(crate) section: EncodedSection,
    pub(crate) offset: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EncodedSection {
    Text,
    Data,
}

pub(crate) struct EncodedRelocation {
    pub(crate) offset: usize,
    pub(crate) target: String,
    pub(crate) kind: String,
    pub(crate) binding: String,
    pub(crate) library: Option<String>,
}

pub(crate) struct EncodedImport {
    pub(crate) library: String,
    pub(crate) symbol: String,
    /// Function (stub) vs data global (GOT-only) (plan-linker.md §5.1).
    pub(crate) kind: ImportKind,
    /// glibc symbol version this reference requires, e.g. `Some("GLIBC_2.17")`
    /// (plan-linker.md §5.2). `None` emits an unversioned reference. Ignored on
    /// Mach-O, which selects by dylib ordinal.
    pub(crate) version: Option<String>,
}

/// The linker binding a relocation target resolves to. Every backend encoder
/// classifies a symbol reference the same way (bug-341-B5); the per-ISA `kind`
/// string and the instruction bytes stay in the emitter, only this
/// internal/external/data split is shared.
pub(crate) enum Binding {
    /// A symbol defined in this image (a direct, module-internal reference).
    Internal,
    /// An imported symbol resolved from `library` (through a stub / the GOT).
    External(String),
    /// A plain data address, neither defined here nor imported.
    Data,
}

impl Binding {
    /// The `(binding, library)` pair an [`EncodedRelocation`] records.
    pub(crate) fn into_relocation_fields(self) -> (String, Option<String>) {
        match self {
            Binding::Internal => ("internal".to_string(), None),
            Binding::External(library) => ("external".to_string(), Some(library)),
            Binding::Data => ("data".to_string(), None),
        }
    }
}

/// Classify a **call** target: `Internal` if defined in this image, `External`
/// if imported, else an error labelled with `arch` (e.g. `"AArch64"`,
/// `"x86-64"`, `"rv64"`). The message always contains "does not resolve".
pub(crate) fn resolve_call_binding(
    symbols: &[EncodedSymbol],
    imports: &HashMap<String, String>,
    target: &str,
    arch: &str,
) -> Result<Binding, String> {
    if symbols.iter().any(|symbol| symbol.name == target) {
        Ok(Binding::Internal)
    } else if let Some(library) = imports.get(target) {
        Ok(Binding::External(library.clone()))
    } else {
        Err(format!(
            "{arch} call target symbol '{target}' does not resolve"
        ))
    }
}

/// Classify a **data** address: `External` (resolved through the GOT) if the
/// symbol is imported, else a plain `Data` address. Never fails — an undefined
/// data symbol is a `data`-bound relocation the linker resolves.
pub(crate) fn resolve_data_binding(imports: &HashMap<String, String>, target: &str) -> Binding {
    match imports.get(target) {
        Some(library) => Binding::External(library.clone()),
        None => Binding::Data,
    }
}
