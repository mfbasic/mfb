use super::*;
use crate::types::ParameterType;

#[derive(Clone)]
pub(crate) struct IrType {
    pub(crate) kind: String,
    pub(crate) visibility: String,
    pub(crate) name: String,
    pub(crate) fields: Vec<IrField>,
    pub(crate) includes: Vec<String>,
    pub(crate) variants: Vec<IrVariant>,
    pub(crate) members: Vec<IrEnumMember>,
    // Source location of the type declaration.
    pub(crate) loc: IrSourceLoc,
    // Project-relative source file this type was declared in, for diagnostics
    // (plan-20-Z relocated type-declaration rules report against it).
    pub(crate) file: String,
}

#[derive(Clone)]
pub(crate) struct IrBinding {
    pub(crate) name: String,
    pub(crate) visibility: String,
    pub(crate) mutable: bool,
    pub(crate) type_: ParameterType,
    pub(crate) value: Option<IrValue>,
    // Source location of the binding declaration.
    pub(crate) loc: IrSourceLoc,
    // Project-relative source file this binding was declared in, for
    // diagnostics (plan-20-Z relocated binding rules report against it).
    pub(crate) file: String,
    // Whether `type_` came from an explicit `AS T` annotation; only then is the
    // binding subject to `TYPE_BINDING_MISMATCH` (plan-20-Z).
    pub(crate) explicit_type: bool,
}

#[derive(Clone)]
pub(crate) struct IrField {
    pub(crate) visibility: Option<String>,
    pub(crate) name: String,
    pub(crate) type_: ParameterType,
    // Source location of the field declaration.
    pub(crate) loc: IrSourceLoc,
}

#[derive(Clone)]
pub(crate) struct IrVariant {
    pub(crate) name: String,
    pub(crate) fields: Vec<IrField>,
    // Source location of the variant declaration.
    pub(crate) loc: IrSourceLoc,
}
#[derive(Clone)]
pub(crate) struct IrEnumMember {
    pub(crate) name: String,
}

#[derive(Clone)]
pub(crate) struct IrParam {
    pub(crate) name: String,
    pub(crate) type_: ParameterType,
    pub(crate) default: Option<IrValue>,
    // Source location of the parameter declaration.
    pub(crate) loc: IrSourceLoc,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct IrSourceLoc {
    pub(crate) line: u32,
    pub(crate) column: u32,
}

#[derive(Clone)]
pub(crate) struct IrRecordUpdate {
    pub(crate) field: String,
    pub(crate) value: IrValue,
}

#[derive(Clone)]
pub struct ExternalFunctionParam {
    pub name: String,
    pub type_: ParameterType,
}

/// One imported package function's signature, carried as **typed data** from the
/// `.mfp` decode boundary to [`crate::ir::lower_augmented_project`].
///
/// The `.mfp` stores parameter and return types as strings (the wire format is
/// unchanged — plan-105-A). `manifest::package` parses each ONE time here, at the
/// decode boundary, and every consumer reads the structure. Before plan-105-A the
/// driver instead *formatted* an export into a `FUNC(p1, p2) AS R` string and then
/// re-parsed the return type back out of it with `rsplit_once(" AS ")`, with
/// `ir::lower` re-parsing the same string a third time — a structured→string→
/// structured round-trip whose format was a silent coupling between three helpers
/// (`planning/Compiler Pipeline.md:58`, Recommendation #1).
///
/// [`signature_type`](Self::signature_type) renders the equivalent
/// [`ParameterType::Func`] when a consumer still needs the string spelling (the
/// lowering context's `function_types` map is string-keyed until plan-106); that is
/// a render-*out*, not a round-trip — nothing parses it back.
#[derive(Clone)]
pub struct ExternalSignature {
    pub params: Vec<ExternalFunctionParam>,
    pub returns: ParameterType,
    pub isolated: bool,
}

impl ExternalSignature {
    /// The signature as a [`ParameterType::Func`] — the typed form of the
    /// `{ISOLATED }FUNC(p1, p2) AS R` spelling the `.mfp` decode used to build by
    /// hand. `.name()` on the result reproduces that spelling byte-exactly.
    pub(crate) fn signature_type(&self) -> ParameterType {
        let params = self
            .params
            .iter()
            .map(|param| param.type_.clone())
            .collect::<Vec<_>>();
        if self.isolated {
            ParameterType::func_isolated(params, self.returns.clone())
        } else {
            ParameterType::func(params, self.returns.clone())
        }
    }
}

/// The whole compiled IR for one project: its declared entities, functions, the
/// native-`LINK` model, and the metadata carried to the backend or into a `.mfp`.
#[derive(Clone)]
pub struct IrProject {
    pub(crate) name: String,
    pub(crate) entry: Option<EntryPoint>,
    pub(crate) bindings: Vec<IrBinding>,
    pub(crate) types: Vec<IrType>,
    pub(crate) functions: Vec<IrFunction>,
    /// Native `LINK` resources declared in this project, surfaced to package
    /// metadata (`RESOURCE_TABLE`) since they carry no executable IR
    /// (plan-link-update.md §10).
    pub(crate) native_resources: Vec<IrNativeResource>,
    /// Native `LINK` functions declared in this project, carried to the backend
    /// so it can emit marshaling thunks + dlopen/dlsym initializers
    /// (plan-linker.md §12).
    pub(crate) link_functions: Vec<IrLinkFunction>,
    /// `CSTRUCT` C-layout declarations from every `LINK` block (plan-50-B).
    /// Carried so the backend can stage struct buffers and the package path can
    /// re-derive each layout from its field ctypes.
    pub(crate) link_cstructs: Vec<IrCStruct>,
    /// Re-export aliases targeting a native `LINK` function:
    /// `(alias_name, target_alias.func)` (plan-link-update.md §5a). Lets the
    /// backend route a call to the exported alias to the target's thunk.
    pub(crate) link_aliases: Vec<(String, String)>,
    /// Documentation collected from `DOC` blocks for the package's exported
    /// declarations (plan-09-doc.md §5). Carried so the package writer can emit
    /// the optional `doc` section; ignored when building an executable.
    pub(crate) docs: ProjectDocs,
    /// The project's **own** native library locators, assembled from its
    /// project.json `libraries` section (plan-46-B §4.3).
    ///
    /// A package build encodes this as `.mfp` section 10. An executable build
    /// keeps it here so a project declaring its *own* `LINK` block resolves
    /// against it — an imported binding's locators come from that binding's
    /// section 10 instead, read straight off the `.mfp` at codegen (plan-46-C).
    pub(crate) native_libraries: crate::binary_repr::NativeLibraryTable,
    /// The ceiling on a single `OUT CBuffer` allocation, in bytes, from the
    /// project.json `maxBuffer` field in MiB (plan-58-C). Defaults to 64 MiB.
    ///
    /// Not encoded into a `.mfp`, deliberately: LINK thunks are emitted when an
    /// executable links, so the ceiling that applies is the CONSUMING project's.
    /// A binding cannot raise an app's memory ceiling on its behalf.
    pub(crate) max_buffer_bytes: u64,
}

impl IrProject {
    /// The distinct native library logical names this project's `LINK` blocks
    /// name, in declaration order (plan-46-B §4.3). These are the names the
    /// manifest's `libraries` section must cover, and the only ones the
    /// `NATIVE_LIBRARY_TABLE` carries.
    pub(crate) fn link_library_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for function in &self.link_functions {
            if !names.contains(&function.library) {
                names.push(function.library.clone());
            }
        }
        names
    }
}

#[derive(Clone)]
pub(crate) struct EntryPoint {
    pub(crate) name: String,
    pub(crate) returns: ParameterType,
    pub(crate) accepts_args: bool,
}

/// One function (or SUB) in the IR: its signature, lowered body, source
/// provenance, and per-resource ownership decisions from escape analysis.
#[derive(Clone)]
pub(crate) struct IrFunction {
    pub(crate) name: String,
    pub(crate) visibility: String,
    pub(crate) kind: String,
    pub(crate) isolated: bool,
    pub(crate) params: Vec<IrParam>,
    pub(crate) returns: ParameterType,
    pub(crate) body: Vec<IrOp>,
    // Source file (project-relative path) this function was lowered from. Used to
    // build `ErrorLoc.filename` for errors that originate inside this function.
    pub(crate) file: String,
    // Source location of the function declaration.
    pub(crate) loc: IrSourceLoc,
    // Resource ownership decisions (escape analysis, §15.6), keyed by `RES`
    // binding name. Drives where each resource's close obligation is discharged:
    // its own scope, an outer collection's scope (runtime owned-list), or out via
    // a returned collection. Absent names are `Local`.
    pub(crate) resource_owners: HashMap<String, crate::ir::resource_escape::ResOwner>,
}
