use crate::ast::LoopKind;
use crate::binary_repr;
use crate::ir::{
    EntryPoint, IrBinding, IrEnumMember, IrField, IrFunction, IrMatchCase, IrMatchPattern, IrOp,
    IrParam, IrProject, IrRecordUpdate, IrType, IrValue, IrVariant,
};
use crate::json::json_string;
use crate::types::ParameterType;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;

use super::runtime;
use super::runtime::RuntimeHelper;

pub(crate) struct NirModule {
    pub(crate) target: String,
    /// Native build mode this module was lowered for (`console` or `macos-app`).
    /// Recorded so downstream plan/code stages and goldens reflect app mode.
    pub(crate) build_mode: crate::target::NativeBuildMode,
    /// Stdin broadcast-log backpressure cap in bytes, baked into the executable
    /// (plan-15 D3). Sourced from the `project.json` `"config"` section's
    /// `stdinLogCap` on the executable path; defaults to `STDIN_LOG_CAP_DEFAULT`
    /// (4 MiB) everywhere else. Read by `lower_stdin_next_byte`.
    pub(crate) stdin_log_cap: u64,
    pub(crate) project: String,
    pub(crate) entry: Option<NirEntryPoint>,
    pub(crate) globals: Vec<NirGlobal>,
    pub(crate) types: Vec<NirType>,
    pub(crate) imports: Vec<NirImport>,
    pub(crate) runtime_helpers: Vec<RuntimeHelper>,
    pub(crate) functions: Vec<NirFunction>,
    /// Native `LINK` functions whose marshaling thunks the backend emits
    /// (plan-linker.md §12). Carried verbatim from the IR.
    pub(crate) link_functions: Vec<crate::ir::IrLinkFunction>,
    /// `CSTRUCT` declarations backing struct ABI slots (plan-50-E). The thunk
    /// needs each one's field ctypes to recompute its layout.
    pub(crate) link_cstructs: Vec<crate::ir::IrCStruct>,
    /// plan-59-B: native `RESOURCE … CLOSE BY op` declarations, carried verbatim
    /// from the IR so codegen can tell which `LINK` function is a resource's
    /// registered close op — the one that must set `RESOURCE_CLOSED_BIT`.
    ///
    /// The IR verifier resolves this through `close_op_for`, but that table lives
    /// at the IR layer and never reached the backend; without it a thunk cannot
    /// know it is a close op, and the closed flag would be read by the guard and
    /// never written by anything.
    pub(crate) native_resources: Vec<crate::ir::IrNativeResource>,
    /// The project's **own** native library locators (plan-46-C), for a project
    /// that declares its own `LINK` block. An imported binding's locators are read
    /// from that binding's `.mfp` section 10 at codegen instead. Carried verbatim
    /// from the IR.
    pub(crate) native_libraries: crate::binary_repr::NativeLibraryTable,
    /// The `OUT CBuffer` allocation ceiling in bytes (project.json `maxBuffer`,
    /// in MiB). See `IrProject::max_buffer_bytes`.
    pub(crate) max_buffer_bytes: u64,
}

/// The internal text symbol of the per-program native `LINK` load-time
/// initializer (plan-linker.md §12.1): runs `dlopen`/`dlsym` before `main`.
pub(crate) const LINK_INIT_SYMBOL: &str = "_mfb_linker_init";

/// The internal text symbol for a native `LINK` function's marshaling thunk
/// (plan-linker.md §12.2): `_mfb_linker_<alias>_<name>`.
pub(crate) fn link_thunk_symbol(alias: &str, name: &str) -> String {
    // Escape each part so no character can be confused with the `_` that joins
    // the two parts: every byte that is not `[A-Za-z0-9]` (including `_` itself)
    // becomes an unambiguous `_XX` two-hex-digit escape. Reusing `_` as both a
    // pass-through character and the separator (the previous `sanitize`) made
    // `(a_b, c)` and `(a, b_c)` collide on `_mfb_linker_a_b_c` (bug-139.6);
    // escaping the interior `_` bytes keeps the two apart.
    let escape = |part: &str| {
        let mut out = String::new();
        for byte in part.bytes() {
            if byte.is_ascii_alphanumeric() {
                out.push(byte as char);
            } else {
                out.push_str(&format!("_{byte:02X}"));
            }
        }
        out
    };
    format!("_mfb_linker_{}_{}", escape(alias), escape(name))
}

pub(crate) struct NirEntryPoint {
    pub(crate) name: String,
    pub(crate) returns: ParameterType,
    pub(crate) accepts_args: bool,
}

pub(crate) struct NirType {
    pub(crate) kind: String,
    pub(crate) visibility: String,
    pub(crate) name: String,
    pub(crate) fields: Vec<NirField>,
    pub(crate) includes: Vec<String>,
    pub(crate) variants: Vec<NirVariant>,
    pub(crate) members: Vec<NirEnumMember>,
}

pub(crate) struct NirField {
    pub(crate) visibility: Option<String>,
    pub(crate) name: String,
    pub(crate) type_: ParameterType,
}

pub(crate) struct NirVariant {
    pub(crate) name: String,
    pub(crate) fields: Vec<NirField>,
}

pub(crate) struct NirEnumMember {
    pub(crate) name: String,
}

pub(crate) struct NirImport {
    pub(crate) package: String,
    pub(crate) name: String,
    pub(crate) symbol: String,
    pub(crate) kind: String,
    pub(crate) isolated: bool,
    pub(crate) params: Vec<NirImportParam>,
    pub(crate) returns: ParameterType,
}

pub(crate) struct NirImportParam {
    pub(crate) type_: ParameterType,
    pub(crate) has_default: bool,
}

pub(crate) struct NirGlobal {
    pub(crate) name: String,
    pub(crate) symbol: String,
    pub(crate) visibility: String,
    pub(crate) mutable: bool,
    pub(crate) type_: ParameterType,
    pub(crate) value: Option<NirValue>,
}

pub(crate) struct NirFunction {
    pub(crate) name: String,
    pub(crate) visibility: String,
    pub(crate) kind: String,
    pub(crate) isolated: bool,
    pub(crate) params: Vec<NirParam>,
    pub(crate) returns: ParameterType,
    pub(crate) body: Vec<NirOp>,
    /// Project-relative source file this function was lowered from. Used to build
    /// `ErrorLoc.filename` for errors that originate inside this function.
    pub(crate) file: String,
    /// Resource ownership decisions (escape analysis, §15.6), keyed by `RES`
    /// binding name. Absent names are [`crate::ir::resource_escape::ResOwner::Local`].
    pub(crate) resource_owners: HashMap<String, crate::ir::resource_escape::ResOwner>,
}

pub(crate) struct NirParam {
    pub(crate) name: String,
    pub(crate) type_: ParameterType,
    pub(crate) default: Option<NirValue>,
}

// Clone: the Opt1 loop rows (unswitching, peeling) duplicate statement lists
// wholesale — a structured-tree transform's bread and butter.
#[derive(Clone)]
pub(crate) enum NirOp {
    Bind {
        mutable: bool,
        name: String,
        type_: ParameterType,
        value: Option<NirValue>,
    },
    StoreGlobal {
        name: String,
        type_: ParameterType,
        value: Option<NirValue>,
    },
    Assign {
        name: String,
        value: NirValue,
    },
    /// Replace the `STATE` payload of a `RES` binding (`resource.state = value`).
    StateAssign {
        resource: String,
        value: NirValue,
    },
    Return {
        value: Option<NirValue>,
    },
    ExitLoop {
        kind: LoopKind,
    },
    ContinueLoop {
        kind: LoopKind,
    },
    ExitProgram {
        code: NirValue,
    },
    Fail {
        error: NirValue,
    },
    Eval {
        value: NirValue,
    },
    If {
        condition: NirValue,
        then_body: Vec<NirOp>,
        else_body: Vec<NirOp>,
    },
    Match {
        value: NirValue,
        cases: Vec<NirMatchCase>,
    },
    While {
        kind: LoopKind,
        condition: NirValue,
        body: Vec<NirOp>,
    },
    For {
        name: String,
        type_: ParameterType,
        start: NirValue,
        end: NirValue,
        step: NirValue,
        body: Vec<NirOp>,
        // Source location of the loop header; origin for increment overflow.
        loc: NirSourceLoc,
    },
    DoUntil {
        body: Vec<NirOp>,
        condition: NirValue,
    },
    ForEach {
        name: String,
        type_: ParameterType,
        iterable: NirValue,
        body: Vec<NirOp>,
    },
    Trap {
        name: String,
        body: Vec<NirOp>,
    },
}

#[derive(Clone)]
pub(crate) struct NirMatchCase {
    pub(crate) pattern: NirMatchPattern,
    pub(crate) guard: Option<NirValue>,
    pub(crate) body: Vec<NirOp>,
}

#[derive(Clone)]
pub(crate) enum NirMatchPattern {
    Else,
    Value(NirValue),
    OneOf(Vec<NirValue>),
}

#[derive(Clone)]
pub(crate) enum NirValue {
    Const {
        type_: ParameterType,
        value: String,
    },
    Local(String),
    /// The address of a local binding's slot — a reference to the slot itself (not
    /// a read of its value) used to capture a `MUT` binding into a non-escaping
    /// callback's environment.
    LocalRef {
        name: String,
        type_: ParameterType,
    },
    Global {
        name: String,
        type_: ParameterType,
    },
    FunctionRef {
        name: String,
        type_: ParameterType,
    },
    Closure {
        name: String,
        type_: ParameterType,
        captures: Vec<NirValue>,
    },
    Capture {
        index: usize,
        type_: ParameterType,
        /// When set, the env slot holds a pointer to the parent binding's slot:
        /// the capture binds a *reference* local whose reads and writes deref
        /// through the slot pointer.
        by_ref: bool,
    },
    Call {
        target: String,
        args: Vec<NirValue>,
        loc: NirSourceLoc,
    },
    CallResult {
        target: String,
        args: Vec<NirValue>,
        loc: NirSourceLoc,
    },
    RuntimeCall {
        helper: RuntimeHelper,
        target: String,
        args: Vec<NirValue>,
        loc: NirSourceLoc,
    },
    /// Evaluate `value` with its domain-error exits captured, yielding a
    /// `Result OF <type_>` (bug-471) — the operator twin of `CallResult`. See
    /// [`crate::ir::value::IrValue::Checked`]; codegen lowers it by running
    /// `value` under a `raw_result_capture`.
    Checked {
        type_: ParameterType,
        value: Box<NirValue>,
    },
    Constructor {
        type_: ParameterType,
        args: Vec<NirValue>,
    },
    UnionWrap {
        union_type: ParameterType,
        member_type: ParameterType,
        value: Box<NirValue>,
    },
    UnionExtract {
        type_: ParameterType,
        value: Box<NirValue>,
    },
    ResultIsOk {
        value: Box<NirValue>,
    },
    ResultValue {
        value: Box<NirValue>,
    },
    ResultError {
        value: Box<NirValue>,
    },
    WithUpdate {
        type_: ParameterType,
        target: Box<NirValue>,
        updates: Vec<NirRecordUpdate>,
    },
    ListLiteral {
        type_: ParameterType,
        values: Vec<NirValue>,
    },
    /// `Set OF T { … }` (plan-63): elements build a deduplicated set at runtime.
    SetLiteral {
        type_: ParameterType,
        values: Vec<NirValue>,
    },
    MapLiteral {
        type_: ParameterType,
        entries: Vec<(NirValue, NirValue)>,
    },
    MemberAccess {
        target: Box<NirValue>,
        member: String,
    },
    Binary {
        op: String,
        left: Box<NirValue>,
        right: Box<NirValue>,
        loc: NirSourceLoc,
    },
    Unary {
        op: String,
        operand: Box<NirValue>,
        loc: NirSourceLoc,
    },
}

/// Source location (line/column within the owning function's file) attached to
/// NIR nodes that can originate a runtime error. The file is carried on
/// [`NirFunction::file`].
#[derive(Clone, Copy, Default)]
pub(crate) struct NirSourceLoc {
    pub(crate) line: u32,
    pub(crate) column: u32,
}

#[derive(Clone)]
pub(crate) struct NirRecordUpdate {
    pub(crate) field: String,
    pub(crate) value: NirValue,
}

pub(crate) mod constfold;
mod json;
mod lower;
mod symbols;
pub(crate) mod visit;

pub(crate) use lower::{lower_module, merge_packages};
pub(crate) use symbols::{
    function_symbol, global_initializer_name, global_symbol, symbol_fragment,
};

#[cfg(test)]
mod link_thunk_symbol_tests {
    use super::link_thunk_symbol;

    #[test]
    fn separator_and_replacement_no_longer_collide() {
        // `(a_b, c)` and `(a, b_c)` used to both render as `_mfb_linker_a_b_c`.
        let left = link_thunk_symbol("a_b", "c");
        let right = link_thunk_symbol("a", "b_c");
        assert_ne!(left, right);
        assert_eq!(left, "_mfb_linker_a_5Fb_c");
        assert_eq!(right, "_mfb_linker_a_b_5Fc");
    }

    #[test]
    fn plain_alnum_parts_are_unescaped_except_separator() {
        assert_eq!(
            link_thunk_symbol("printf", "libc"),
            "_mfb_linker_printf_libc"
        );
    }
}
